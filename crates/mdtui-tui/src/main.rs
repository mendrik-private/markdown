use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use font8x8::UnicodeFonts;
use fontdb::{Database, Family, Query, Style as FontStyle, Weight};
use fontdue::{
    Font, FontSettings,
    layout::{CoordinateSystem, Layout as FontLayout, LayoutSettings, TextStyle},
};
use image::{ColorType, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use mdtui_core::{Block as DocBlock, Cursor, Direction};
use mdtui_render::{
    DisplayAction, DisplayKind, RenderOptions, Rendered, Theme, action_at, hit_test,
    render_document,
};
use mdtui_tui::App;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction as LayoutDirection, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;
type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;
type HeadlineRasterResult = (String, io::Result<Vec<u8>>);
const HEADLINE_RASTER_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExplorerMode {
    Nested,
    Flat,
}

#[derive(Clone, Debug)]
enum ExplorerAction {
    ToggleMode(ExplorerMode),
    ToggleDir(PathBuf),
    OpenFile(PathBuf),
}

#[derive(Clone, Debug)]
struct ExplorerHit {
    row: u16,
    start: u16,
    end: u16,
    action: ExplorerAction,
}

#[derive(Clone, Debug)]
struct OutlineHit {
    row: u16,
    block: usize,
}

#[derive(Clone, Debug)]
enum StatusAction {
    SetWrapWidth(u16),
    SetColumns(u8),
}

#[derive(Clone, Debug)]
struct StatusHit {
    start: u16,
    end: u16,
    action: StatusAction,
}

#[derive(Clone, Debug)]
struct TabHit {
    start: u16,
    end: u16,
    close_start: u16,
    close_end: u16,
    name: String,
}

fn main() -> AppResult<()> {
    let path = env::args().nth(1).map(PathBuf::from);
    let (file_name, source) = match &path {
        Some(path) => (path.display().to_string(), fs::read_to_string(path)?),
        None => ("untitled.md".to_string(), String::new()),
    };

    let mut state = TuiState::new(App::from_markdown(file_name, &source), path);
    let mut terminal = TerminalGuard::enter()?;
    let result = run(&mut terminal.terminal, &mut state);
    terminal.leave()?;
    result
}

struct TerminalGuard {
    terminal: TuiTerminal,
    active: bool,
}

impl TerminalGuard {
    fn enter() -> AppResult<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    fn leave(&mut self) -> AppResult<()> {
        if self.active {
            disable_raw_mode()?;
            execute!(
                self.terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            self.terminal.show_cursor()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

struct TuiState {
    app: App,
    path: Option<PathBuf>,
    scroll: u16,
    preferred_column: Option<u16>,
    wrap_width: u16,
    show_help: bool,
    dirty: bool,
    message: String,
    last_tabs_area: Rect,
    last_doc_area: Rect,
    last_explorer_area: Rect,
    last_outline_area: Rect,
    last_status_area: Rect,
    last_rendered: Option<Rendered>,
    drag_anchor: Option<(u16, u16)>,
    last_style_popup: Option<Rect>,
    explorer_mode: ExplorerMode,
    explorer_scroll: u16,
    outline_scroll: u16,
    collapsed_dirs: HashSet<PathBuf>,
    explorer_hits: Vec<ExplorerHit>,
    outline_hits: Vec<OutlineHit>,
    status_hits: Vec<StatusHit>,
    tab_hits: Vec<TabHit>,
    hidden_tabs: HashSet<String>,
    kitty_graphics: bool,
    last_kitty_signature: Option<String>,
    headline_png_cache: HashMap<String, Vec<u8>>,
    pending_headline_jobs: HashSet<String>,
    headline_raster_tx: Sender<HeadlineRasterResult>,
    headline_raster_rx: Receiver<HeadlineRasterResult>,
}

impl TuiState {
    fn new(app: App, path: Option<PathBuf>) -> Self {
        let (headline_raster_tx, headline_raster_rx) = mpsc::channel();
        Self {
            app,
            path,
            scroll: 0,
            preferred_column: None,
            wrap_width: 65,
            show_help: false,
            dirty: false,
            message: "direct editing · F1/? help · Ctrl-S save · Ctrl-Q quit".to_string(),
            last_tabs_area: Rect::default(),
            last_doc_area: Rect::default(),
            last_explorer_area: Rect::default(),
            last_outline_area: Rect::default(),
            last_status_area: Rect::default(),
            last_rendered: None,
            drag_anchor: None,
            last_style_popup: None,
            explorer_mode: ExplorerMode::Nested,
            explorer_scroll: 0,
            outline_scroll: 0,
            collapsed_dirs: HashSet::new(),
            explorer_hits: Vec::new(),
            outline_hits: Vec::new(),
            status_hits: Vec::new(),
            tab_hits: Vec::new(),
            hidden_tabs: HashSet::new(),
            kitty_graphics: detect_kitty_support(),
            last_kitty_signature: None,
            headline_png_cache: HashMap::new(),
            pending_headline_jobs: HashSet::new(),
            headline_raster_tx,
            headline_raster_rx,
        }
    }

    fn save(&mut self) {
        let Some(path) = &self.path else {
            self.message = "no file path for save".to_string();
            return;
        };
        match fs::write(path, self.app.save_to_gfm()) {
            Ok(()) => {
                self.dirty = false;
                self.message = format!("saved {}", path.display());
            }
            Err(error) => {
                self.message = format!("save failed: {error}");
            }
        }
    }
}

fn run(terminal: &mut TuiTerminal, state: &mut TuiState) -> AppResult<()> {
    let mut needs_redraw = true;
    loop {
        if drain_headline_raster_results(state) {
            needs_redraw = true;
        }
        if needs_redraw {
            terminal.draw(|frame| draw(frame, state))?;
            emit_kitty_headlines(terminal.backend_mut(), state)?;
            needs_redraw = false;
        }
        if event::poll(Duration::from_millis(16))? {
            needs_redraw = true;
            if process_event(state, event::read()?) {
                break;
            }
            while event::poll(Duration::ZERO)? {
                if process_event(state, event::read()?) {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn drain_headline_raster_results(state: &mut TuiState) -> bool {
    let mut changed = false;
    while let Ok((key, result)) = state.headline_raster_rx.try_recv() {
        state.pending_headline_jobs.remove(&key);
        if let Ok(bytes) = result {
            state.headline_png_cache.insert(key, bytes);
            changed = true;
        }
    }
    changed
}

fn process_event(state: &mut TuiState, event: Event) -> bool {
    match event {
        Event::Key(key) if should_handle_key(key) => handle_key(state, key),
        Event::Mouse(mouse) => {
            handle_mouse(state, mouse);
            false
        }
        Event::Resize(_, _) | Event::FocusGained | Event::FocusLost | Event::Paste(_) => false,
        Event::Key(_) => false,
    }
}

fn should_handle_key(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn handle_key(state: &mut TuiState, key: KeyEvent) -> bool {
    if state.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => state.show_help = false,
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => state.save(),
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 1;
            state.message = "column mode 1".to_string();
        }
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 2;
            state.message = "column mode 2".to_string();
        }
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 3;
            state.message = "column mode 3".to_string();
        }
        KeyCode::Char('-') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.wrap_width = state.wrap_width.saturating_sub(4).max(24);
            state.message = format!("wrap width {}", state.wrap_width);
        }
        KeyCode::Char('=') | KeyCode::Char('+')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            state.preferred_column = None;
            state.wrap_width = state.wrap_width.saturating_add(4).min(120);
            state.message = format!("wrap width {}", state.wrap_width);
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.editor.undo();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_bold();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_italic();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_code();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char('X') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_strike();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char('x')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            state.app.apply_strike();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::F(1) | KeyCode::Char('?') => state.show_help = true,
        KeyCode::Char(' ') => {
            state.app.editor.space();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char(ch) if is_plain_text_key(key) => {
            state.app.type_char(ch);
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Enter => {
            state.app.enter();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Backspace => {
            state.app.backspace();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Delete => {
            state.app.delete();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.ctrl_arrow(Direction::Left);
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.ctrl_arrow(Direction::Right);
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.ctrl_arrow(Direction::Up);
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.ctrl_arrow(Direction::Down);
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Left => {
            state.preferred_column = None;
            state
                .app
                .editor
                .move_left(key.modifiers.contains(KeyModifiers::SHIFT));
        }
        KeyCode::Right => {
            state.preferred_column = None;
            state
                .app
                .editor
                .move_right(key.modifiers.contains(KeyModifiers::SHIFT));
        }
        KeyCode::Up => move_visual(state, -1, key.modifiers.contains(KeyModifiers::SHIFT)),
        KeyCode::Down => move_visual(state, 1, key.modifiers.contains(KeyModifiers::SHIFT)),
        KeyCode::PageUp => {
            state.preferred_column = None;
            state.scroll = state.scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            state.preferred_column = None;
            state.scroll = state.scroll.saturating_add(10);
        }
        KeyCode::Tab => {
            state.app.tab();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::BackTab => {
            state.app.shift_tab();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Esc => {
            state.preferred_column = None;
            state.message = "direct editing".to_string();
        }
        _ => {}
    }
    ensure_cursor_visible(state);
    false
}

fn is_plain_text_key(key: KeyEvent) -> bool {
    key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT
}

fn move_visual(state: &mut TuiState, delta: i32, extend: bool) {
    let Some(rendered) = &state.last_rendered else {
        if delta < 0 {
            state.app.editor.move_up(extend);
        } else {
            state.app.editor.move_down(extend);
        }
        return;
    };
    let Some((x, y)) = cursor_position(&state.app.editor.cursor, &rendered.display.items) else {
        return;
    };
    let preferred_x = state.preferred_column.unwrap_or(x);
    let step = headline_visual_step(state, state.app.editor.cursor);
    let mut target_y = y as i32 + delta * step;
    if target_y < 0 {
        return;
    }
    let mut target = None;
    for _ in 0..3 {
        let row = target_y as u16;
        target = nearest_cursor_on_row(preferred_x, row, &rendered.display.items)
            .or_else(|| hit_test(preferred_x, row, &rendered.display))
            .or_else(|| hit_test(0, row, &rendered.display));
        if target.is_some() {
            break;
        }
        target_y += delta.signum();
        if target_y < 0 {
            break;
        }
    }
    let Some(cursor) = target else {
        return;
    };
    state.preferred_column = Some(preferred_x);
    if extend {
        let anchor = state
            .app
            .editor
            .selection
            .map_or(state.app.editor.cursor, |selection| selection.anchor);
        state.app.editor.select_range(anchor, cursor);
    } else {
        state.app.editor.set_cursor(cursor);
    }
}

fn headline_visual_step(state: &TuiState, cursor: Cursor) -> i32 {
    let block = cursor_block(cursor);
    match state.app.editor.document.blocks.get(block) {
        Some(DocBlock::Heading { level, inlines })
            if state.kitty_graphics && matches!(level, 1 | 2) =>
        {
            if mdtui_core::inline_text(inlines).is_ascii() {
                2
            } else {
                1
            }
        }
        _ => 1,
    }
}

fn ensure_cursor_visible(state: &mut TuiState) {
    let Some(rendered) = &state.last_rendered else {
        return;
    };
    let Some((_, y)) = cursor_position(&state.app.editor.cursor, &rendered.display.items) else {
        return;
    };
    let viewport = state.last_doc_area.height.saturating_sub(2);
    if viewport == 0 {
        return;
    }
    if y < state.scroll {
        state.scroll = y;
    } else if y >= state.scroll.saturating_add(viewport) {
        state.scroll = y.saturating_sub(viewport.saturating_sub(1));
    }
}

fn handle_mouse(state: &mut TuiState, mouse: MouseEvent) {
    if let Some(popup) = state.last_style_popup
        && mouse.column >= popup.x
        && mouse.column < popup.x.saturating_add(popup.width)
        && mouse.row >= popup.y
        && mouse.row < popup.y.saturating_add(popup.height)
    {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            click_style_popup(state, popup, mouse.column, mouse.row);
        }
        return;
    }

    if state.last_status_area.height > 0
        && mouse.column >= state.last_status_area.x
        && mouse.column
            < state
                .last_status_area
                .x
                .saturating_add(state.last_status_area.width)
        && mouse.row >= state.last_status_area.y
        && mouse.row
            < state
                .last_status_area
                .y
                .saturating_add(state.last_status_area.height)
    {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let local_x = mouse.column.saturating_sub(state.last_status_area.x);
                if let Some(action) = state
                    .status_hits
                    .iter()
                    .find(|hit| local_x >= hit.start && local_x < hit.end)
                    .map(|hit| hit.action.clone())
                {
                    run_status_action(state, &action);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {}
            _ => {}
        }
        return;
    }

    if state.last_tabs_area.width > 2
        && mouse.column >= state.last_tabs_area.x
        && mouse.column
            < state
                .last_tabs_area
                .x
                .saturating_add(state.last_tabs_area.width)
        && mouse.row >= state.last_tabs_area.y
        && mouse.row
            < state
                .last_tabs_area
                .y
                .saturating_add(state.last_tabs_area.height)
    {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let local_x = mouse.column.saturating_sub(state.last_tabs_area.x);
            if let Some(hit) = state
                .tab_hits
                .iter()
                .find(|hit| local_x >= hit.start && local_x < hit.end)
                .cloned()
            {
                if local_x >= hit.close_start && local_x < hit.close_end {
                    close_tab(state, &hit.name);
                } else {
                    activate_tab(state, &hit.name);
                }
            }
        }
        return;
    }

    if state.last_explorer_area.width > 2
        && mouse.column > state.last_explorer_area.x
        && mouse.column
            < state
                .last_explorer_area
                .x
                .saturating_add(state.last_explorer_area.width.saturating_sub(1))
        && mouse.row > state.last_explorer_area.y
        && mouse.row
            < state
                .last_explorer_area
                .y
                .saturating_add(state.last_explorer_area.height.saturating_sub(1))
    {
        let local_x = mouse.column.saturating_sub(state.last_explorer_area.x + 1);
        let row = mouse
            .row
            .saturating_sub(state.last_explorer_area.y + 1)
            .saturating_add(state.explorer_scroll);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(action) = state
                    .explorer_hits
                    .iter()
                    .find(|hit| hit.row == row && local_x >= hit.start && local_x < hit.end)
                    .map(|hit| hit.action.clone())
                {
                    run_explorer_action(state, &action);
                }
            }
            MouseEventKind::ScrollDown => {
                state.explorer_scroll = state.explorer_scroll.saturating_add(1)
            }
            MouseEventKind::ScrollUp => {
                state.explorer_scroll = state.explorer_scroll.saturating_sub(1)
            }
            _ => {}
        }
        return;
    }

    if state.last_outline_area.width > 2
        && mouse.column > state.last_outline_area.x
        && mouse.column
            < state
                .last_outline_area
                .x
                .saturating_add(state.last_outline_area.width.saturating_sub(1))
        && mouse.row > state.last_outline_area.y
        && mouse.row
            < state
                .last_outline_area
                .y
                .saturating_add(state.last_outline_area.height.saturating_sub(1))
    {
        let row = mouse
            .row
            .saturating_sub(state.last_outline_area.y + 1)
            .saturating_add(state.outline_scroll);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = state.outline_hits.iter().find(|hit| hit.row == row) {
                    state.preferred_column = None;
                    state.app.editor.set_cursor(Cursor::Text {
                        block: hit.block,
                        offset: 0,
                    });
                    state.message = "outline jump".to_string();
                    ensure_cursor_visible(state);
                }
            }
            MouseEventKind::ScrollDown => {
                state.outline_scroll = state.outline_scroll.saturating_add(1)
            }
            MouseEventKind::ScrollUp => {
                state.outline_scroll = state.outline_scroll.saturating_sub(1)
            }
            _ => {}
        }
        return;
    }

    let area = state.last_doc_area;
    if mouse.column <= area.x
        || mouse.row <= area.y
        || mouse.column >= area.x.saturating_add(area.width.saturating_sub(1))
        || mouse.row >= area.y.saturating_add(area.height.saturating_sub(1))
    {
        return;
    }

    let x = mouse.column.saturating_sub(area.x + 1);
    let y = mouse
        .row
        .saturating_sub(area.y + 1)
        .saturating_add(state.scroll);

    if let Some(rendered) = &state.last_rendered
        && let Some(action) = action_at(x, y, &rendered.display)
    {
        run_action(state, action);
        return;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            state.drag_anchor = Some((x, y));
            state.preferred_column = None;
            state.app.click(x, y);
            ensure_cursor_visible(state);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(anchor) = state.drag_anchor {
                state.preferred_column = None;
                state.app.drag_select(anchor, (x, y));
            }
        }
        MouseEventKind::Up(MouseButton::Left) => state.drag_anchor = None,
        MouseEventKind::ScrollDown => {
            state.preferred_column = None;
            state.scroll = state.scroll.saturating_add(1);
        }
        MouseEventKind::ScrollUp => {
            state.preferred_column = None;
            state.scroll = state.scroll.saturating_sub(1);
        }
        _ => {}
    }
}

fn draw(frame: &mut Frame<'_>, state: &mut TuiState) {
    let theme = Theme::dark_amber();
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(rgb(theme.app_bg))),
        area,
    );

    let vertical = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(area);
    let body_area = vertical[0];
    let status_area = vertical[1];

    let columns = Layout::default()
        .direction(LayoutDirection::Horizontal)
        .constraints([Constraint::Length(31), Constraint::Min(40)])
        .split(body_area);
    let sidebar = columns[0];
    let doc_column = columns[1];

    let sidebar_split = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(sidebar);
    let doc_split = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(5)])
        .split(doc_column);
    let tabs_area = doc_split[0];
    let doc_area = doc_split[1];
    state.last_tabs_area = tabs_area;
    state.last_doc_area = doc_area;
    state.last_explorer_area = sidebar_split[0];
    state.last_outline_area = sidebar_split[1];
    state.last_status_area = status_area;

    let doc_inner_width = doc_area
        .width
        .saturating_sub(4)
        .max(24)
        .min(state.wrap_width.max(24));
    state.app.render_options = RenderOptions {
        width: doc_inner_width,
        kitty_graphics: state.kitty_graphics,
        show_status: false,
        ..state.app.render_options
    };
    let rendered = render_document(&state.app.editor.document, state.app.render_options);
    let viewport = usize::from(doc_area.height.saturating_sub(2));
    let max_scroll = rendered.lines.len().saturating_sub(viewport);
    state.scroll = state.scroll.min(max_scroll as u16);
    state.last_rendered = Some(rendered.clone());

    let (explorer_lines, explorer_hits) = explorer_lines(
        state.path.as_deref(),
        &state.app.file_name,
        state.explorer_mode,
        &state.collapsed_dirs,
        &theme,
    );
    state.explorer_hits = explorer_hits;
    state.explorer_scroll = clamp_scroll(
        state.explorer_scroll,
        explorer_lines.len(),
        usize::from(sidebar_split[0].height.saturating_sub(2)),
    );

    let (outline_lines, outline_hits) = outline_lines(
        &state.app.editor.document.blocks,
        cursor_block(state.app.editor.cursor),
        &theme,
    );
    state.outline_hits = outline_hits;
    state.outline_scroll = clamp_scroll(
        state.outline_scroll,
        outline_lines.len(),
        usize::from(sidebar_split[1].height.saturating_sub(2)),
    );

    draw_tabs(frame, tabs_area, state, &theme);
    draw_explorer(
        frame,
        sidebar_split[0],
        &explorer_lines,
        state.explorer_scroll,
        &theme,
    );
    draw_outline(
        frame,
        sidebar_split[1],
        &outline_lines,
        state.outline_scroll,
        &theme,
    );
    draw_document(frame, doc_area, state, &rendered, &theme);
    draw_status(frame, status_area, state, &rendered, &theme);

    if has_selection(state) {
        state.last_style_popup = Some(draw_style_popover(frame, area, &theme));
    } else {
        state.last_style_popup = None;
    }
    if state.show_help {
        draw_help(frame, area, &theme);
    }

    if let Some((x, y)) = cursor_position(&state.app.editor.cursor, &rendered.display.items) {
        let screen_x = doc_area.x.saturating_add(1).saturating_add(x);
        let screen_y = doc_area
            .y
            .saturating_add(1)
            .saturating_add(y.saturating_sub(state.scroll));
        if screen_y > doc_area.y
            && screen_y < doc_area.y.saturating_add(doc_area.height.saturating_sub(1))
        {
            frame.set_cursor_position((screen_x, screen_y));
        }
    }
}

fn draw_tabs(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    state.tab_hits.clear();

    let top_y = area.y;
    let label_y = area.y + if area.height > 1 { 1 } else { 0 };
    let roof = area.height >= 2;
    let right = area.x.saturating_add(area.width);
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(rgb(theme.app_bg)));

    let mut x = area.x;

    for (name, active) in tab_entries(
        state.path.as_deref(),
        &state.app.file_name,
        &state.hidden_tabs,
    ) {
        if x >= right {
            break;
        }
        let start_x = x;
        let width = (name.chars().count() as u16 + 6).min(right.saturating_sub(x));
        let close_start = start_x
            .saturating_add(1)
            .saturating_add(name.chars().count() as u16)
            .saturating_add(2);
        state.tab_hits.push(TabHit {
            start: start_x.saturating_sub(area.x),
            end: start_x.saturating_sub(area.x).saturating_add(width),
            close_start: close_start.saturating_sub(area.x),
            close_end: close_start.saturating_sub(area.x).saturating_add(1),
            name: name.clone(),
        });
        let border = Style::default()
            .fg(rgb(if active {
                theme.border_strong
            } else {
                theme.border
            }))
            .bg(rgb(theme.app_bg));
        let fill = Style::default()
            .fg(rgb(if active {
                theme.accent_highlight
            } else {
                theme.text_secondary
            }))
            .bg(rgb(if active {
                theme.panel_raised
            } else {
                theme.panel_bg
            }))
            .add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let close = Style::default()
            .fg(rgb(if active {
                theme.accent_primary
            } else {
                theme.text_muted
            }))
            .bg(rgb(if active {
                theme.panel_raised
            } else {
                theme.panel_bg
            }));

        if roof && width >= 2 {
            let mut roof_x = start_x;
            put(buf, &mut roof_x, top_y, right, "╭", border);
            if width > 2 {
                put(
                    buf,
                    &mut roof_x,
                    top_y,
                    right,
                    &"─".repeat((width - 2) as usize),
                    border,
                );
            }
            put(buf, &mut roof_x, top_y, right, "╮", border);
        }

        x = start_x;
        put(buf, &mut x, label_y, right, "│", border);
        put(buf, &mut x, label_y, right, " ", fill);
        put(buf, &mut x, label_y, right, &name, fill);
        put(buf, &mut x, label_y, right, " ", fill);
        put(buf, &mut x, label_y, right, "×", close);
        put(buf, &mut x, label_y, right, " ", fill);
        put(buf, &mut x, label_y, right, "│", border);

        x = start_x.saturating_add(width);
    }

    let controls_width = "  +  ⋮ ".chars().count() as u16;
    let mut controls_x = right.saturating_sub(controls_width.saturating_add(1));
    put(
        buf,
        &mut controls_x,
        label_y,
        right,
        "  +  ⋮ ",
        Style::default()
            .fg(rgb(theme.text_secondary))
            .bg(rgb(theme.panel_bg)),
    );
}

fn draw_explorer(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: &[Line<'static>],
    scroll: u16,
    theme: &Theme,
) {
    frame.render_widget(
        Paragraph::new(lines.to_vec())
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" EXPLORER ")
                    .title_style(
                        Style::default()
                            .fg(rgb(theme.accent_highlight))
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_type(BorderType::Rounded)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(rgb(theme.border)))
                    .style(Style::default().bg(rgb(theme.panel_bg))),
            ),
        area,
    );
}

fn draw_outline(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: &[Line<'static>],
    scroll: u16,
    theme: &Theme,
) {
    frame.render_widget(
        Paragraph::new(lines.to_vec())
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" OUTLINE ")
                    .title_style(
                        Style::default()
                            .fg(rgb(theme.accent_highlight))
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_type(BorderType::Rounded)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(rgb(theme.border)))
                    .style(Style::default().bg(rgb(theme.panel_bg))),
            ),
        area,
    );
}

fn draw_document(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    rendered: &Rendered,
    theme: &Theme,
) {
    let current_y =
        cursor_position(&state.app.editor.cursor, &rendered.display.items).map(|(_, y)| y);
    let selection_range = selection_line_range(state, rendered);
    let lines = rendered
        .lines
        .iter()
        .enumerate()
        .scan(false, |in_code, (index, line)| {
            Some(style_rendered_line(
                index,
                line,
                rendered.lines.get(index + 1).map(String::as_str),
                in_code,
                theme,
                current_y,
                selection_range,
            ))
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .border_type(BorderType::Rounded)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(rgb(theme.border_strong)))
                    .style(Style::default().bg(rgb(theme.app_bg))),
            )
            .scroll((state.scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );

    draw_scrollbar(
        frame,
        Rect {
            x: area.x + area.width.saturating_sub(2),
            y: area.y + 1,
            width: 1,
            height: area.height.saturating_sub(2),
        },
        usize::from(state.scroll),
        rendered.lines.len(),
        theme,
    );
}

fn draw_status(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut TuiState,
    rendered: &Rendered,
    theme: &Theme,
) {
    state.status_hits.clear();
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(rgb(theme.panel_bg)));
    let right = area.x.saturating_add(area.width);
    let (row, col) = state.app.editor.logical_position();
    let selection = state
        .app
        .editor
        .selection
        .filter(|selection| !selection.is_collapsed())
        .map_or(String::new(), |_| " · selection".to_string());
    let mut x = area.x;

    put(
        buf,
        &mut x,
        area.y,
        right,
        " main ",
        Style::default()
            .fg(rgb(theme.app_bg))
            .bg(rgb(theme.accent_primary))
            .add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        &mut x,
        area.y,
        right,
        &format!(" {}{} ", compact_text(&state.message, 28), selection),
        Style::default()
            .fg(rgb(theme.text_secondary))
            .bg(rgb(theme.panel_bg)),
    );
    put(
        buf,
        &mut x,
        area.y,
        right,
        &format!(" {} row {row} col {col} ", file_leaf(&state.app.file_name)),
        Style::default()
            .fg(rgb(theme.text_primary))
            .bg(rgb(theme.panel_bg))
            .add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        &mut x,
        area.y,
        right,
        " wrap ",
        Style::default()
            .fg(rgb(theme.text_muted))
            .bg(rgb(theme.panel_bg)),
    );
    let slider_slots = 10u16;
    let min_wrap = 24u16;
    let max_wrap = 120u16;
    let thumb = usize::from(
        state
            .wrap_width
            .saturating_sub(min_wrap)
            .saturating_mul(slider_slots.saturating_sub(1))
            / max_wrap.saturating_sub(min_wrap),
    );
    put(
        buf,
        &mut x,
        area.y,
        right,
        "[",
        Style::default()
            .fg(rgb(theme.border))
            .bg(rgb(theme.panel_bg)),
    );
    for index in 0..slider_slots {
        let start = x.saturating_sub(area.x);
        let value = min_wrap
            + (index.saturating_mul(max_wrap.saturating_sub(min_wrap))
                / slider_slots.saturating_sub(1));
        state.status_hits.push(StatusHit {
            start,
            end: start.saturating_add(1),
            action: StatusAction::SetWrapWidth(value),
        });
        put(
            buf,
            &mut x,
            area.y,
            right,
            if usize::from(index) == thumb {
                "◆"
            } else {
                "─"
            },
            Style::default()
                .fg(rgb(if usize::from(index) == thumb {
                    theme.accent_highlight
                } else {
                    theme.link
                }))
                .bg(rgb(theme.panel_bg)),
        );
    }
    put(
        buf,
        &mut x,
        area.y,
        right,
        &format!("] {:>3} ", state.wrap_width),
        Style::default()
            .fg(rgb(theme.text_primary))
            .bg(rgb(theme.panel_bg)),
    );
    put(
        buf,
        &mut x,
        area.y,
        right,
        " cols ",
        Style::default()
            .fg(rgb(theme.text_muted))
            .bg(rgb(theme.panel_bg)),
    );
    for column in 1..=3u8 {
        let label = if state.app.render_options.columns == column {
            format!("[{column}]")
        } else {
            format!(" {column} ")
        };
        let start = x.saturating_sub(area.x);
        state.status_hits.push(StatusHit {
            start,
            end: start.saturating_add(label.chars().count() as u16),
            action: StatusAction::SetColumns(column),
        });
        put(
            buf,
            &mut x,
            area.y,
            right,
            &label,
            Style::default()
                .fg(rgb(if state.app.render_options.columns == column {
                    theme.accent_highlight
                } else {
                    theme.link
                }))
                .bg(rgb(if state.app.render_options.columns == column {
                    theme.panel_raised
                } else {
                    theme.panel_bg
                }))
                .add_modifier(if state.app.render_options.columns == column {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        );
    }
    put(
        buf,
        &mut x,
        area.y,
        right,
        &format!(
            "  {} / {}  ctrl-1/2/3 cols  ctrl--/ctrl-= wrap  F1/? help",
            row,
            rendered.lines.len().max(1)
        ),
        Style::default()
            .fg(rgb(theme.text_secondary))
            .bg(rgb(theme.panel_bg)),
    );
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let width = area.width.min(68);
    let height = area.height.min(14);
    let popup = centered(area, width, height);
    let text = vec![
        Line::from("Navigation"),
        Line::from("  arrows move · Shift+arrows select · PageUp/PageDown scroll"),
        Line::from("  Ctrl-1/2/3 columns · Ctrl--/Ctrl-= wrap width"),
        Line::from("Editing"),
        Line::from("  type to edit · Enter split/create · Backspace/Delete remove"),
        Line::from("  Ctrl-B bold · Ctrl-I italic · Ctrl-E code · Ctrl-Shift-X strike"),
        Line::from("Tables & Lists"),
        Line::from("  Tab / Shift+Tab move cells · Ctrl+Arrow add row/column"),
        Line::from("Mouse"),
        Line::from("  click place cursor · drag select · click explorer/outline/status controls"),
        Line::from("Session"),
        Line::from("  Ctrl-S save · Ctrl-Q quit · F1 or Esc close help"),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Help ")
                    .title_style(
                        Style::default()
                            .fg(rgb(theme.accent_highlight))
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_type(BorderType::Rounded)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(rgb(theme.border_strong)))
                    .style(Style::default().bg(rgb(theme.panel_raised))),
            )
            .style(
                Style::default()
                    .fg(rgb(theme.text_primary))
                    .bg(rgb(theme.panel_raised)),
            ),
        popup,
    );
}

fn draw_style_popover(frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
    let width = area.width.min(38);
    let height = 5;
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width + 4),
        y: area.y + area.height / 3,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(" B   I   S   </> "),
            Line::from(" Ctrl-B  Ctrl-I  Ctrl-Shift-X  Ctrl-E "),
            Line::from("              Inline Style              "),
        ])
        .block(
            Block::default()
                .border_type(BorderType::Rounded)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(rgb(theme.border_strong)))
                .style(Style::default().bg(rgb(theme.panel_raised))),
        )
        .style(
            Style::default()
                .fg(rgb(theme.text_primary))
                .bg(rgb(theme.panel_raised)),
        ),
        popup,
    );
    popup
}

fn draw_scrollbar(frame: &mut Frame<'_>, area: Rect, offset: usize, content: usize, theme: &Theme) {
    if area.height == 0 || content <= usize::from(area.height) {
        return;
    }
    let viewport = usize::from(area.height);
    let thumb = ((viewport * viewport) / content).max(1).min(viewport);
    let track = viewport.saturating_sub(thumb).max(1);
    let max_offset = content.saturating_sub(viewport).max(1);
    let start = (offset * track) / max_offset;
    let mut lines = Vec::new();
    for index in 0..viewport {
        let ch = if index >= start && index < start + thumb {
            "█"
        } else {
            "│"
        };
        let style = if ch == "█" {
            Style::default()
                .fg(rgb(theme.accent_highlight))
                .bg(rgb(theme.app_bg))
        } else {
            Style::default().fg(rgb(theme.border)).bg(rgb(theme.app_bg))
        };
        lines.push(Line::from(Span::styled(ch.to_string(), style)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn run_action(state: &mut TuiState, action: DisplayAction) {
    match action {
        DisplayAction::CopyCodeBlock { block } => {
            let Some(DocBlock::CodeBlock { text, .. }) =
                state.app.editor.document.blocks.get(block)
            else {
                state.message = "copy failed".to_string();
                return;
            };
            match copy_osc52(text) {
                Ok(()) => state.message = "code block copied".to_string(),
                Err(error) => state.message = format!("copy failed: {error}"),
            }
        }
    }
}

fn click_style_popup(state: &mut TuiState, popup: Rect, x: u16, y: u16) {
    if y != popup.y.saturating_add(1) {
        return;
    }
    let local_x = x.saturating_sub(popup.x.saturating_add(2));
    match local_x {
        0..=2 => state.app.apply_bold(),
        4..=6 => state.app.apply_italic(),
        8..=10 => state.app.apply_strike(),
        12..=17 => state.app.apply_code(),
        _ => return,
    }
    state.dirty = true;
    state.message = "style applied".to_string();
}

fn run_explorer_action(state: &mut TuiState, action: &ExplorerAction) {
    match action {
        ExplorerAction::ToggleMode(mode) => {
            state.explorer_mode = *mode;
            state.explorer_scroll = 0;
            state.message = match mode {
                ExplorerMode::Flat => "explorer flat".to_string(),
                ExplorerMode::Nested => "explorer nested".to_string(),
            };
        }
        ExplorerAction::ToggleDir(path) => {
            if !state.collapsed_dirs.insert(path.clone()) {
                state.collapsed_dirs.remove(path);
            }
        }
        ExplorerAction::OpenFile(path) => open_file(state, path),
    }
}

fn run_status_action(state: &mut TuiState, action: &StatusAction) {
    match *action {
        StatusAction::SetWrapWidth(width) => {
            state.wrap_width = width.clamp(24, 120);
            state.message = format!("wrap width {}", state.wrap_width);
        }
        StatusAction::SetColumns(columns) => {
            state.app.render_options.columns = columns.clamp(1, 3);
            state.scroll = 0;
            state.message = format!("column mode {}", state.app.render_options.columns);
        }
    }
}

fn open_file(state: &mut TuiState, path: &Path) {
    match fs::read_to_string(path) {
        Ok(source) => {
            let mut app = App::from_markdown(path.display().to_string(), &source);
            app.render_options = state.app.render_options;
            state.app = app;
            state.path = Some(path.to_path_buf());
            state
                .hidden_tabs
                .remove(&file_leaf(&path.display().to_string()));
            state.scroll = 0;
            state.preferred_column = None;
            state.dirty = false;
            state.message = format!("opened {}", file_leaf(&path.display().to_string()));
        }
        Err(error) => {
            state.message = format!("open failed: {error}");
        }
    }
}

fn activate_tab(state: &mut TuiState, name: &str) {
    let path = tab_path(state, name);
    open_file(state, &path);
}

fn close_tab(state: &mut TuiState, name: &str) {
    let visible = tab_entries(
        state.path.as_deref(),
        &state.app.file_name,
        &state.hidden_tabs,
    );
    if visible.len() <= 1 && visible.iter().any(|(tab_name, _)| tab_name == name) {
        state.message = "last tab stays open".to_string();
        return;
    }
    state.hidden_tabs.insert(name.to_string());
    if state.app.file_name.ends_with(name) {
        if let Some((next, _)) = visible.into_iter().find(|(tab_name, _)| tab_name != name) {
            activate_tab(state, &next);
        }
    } else {
        state.message = format!("closed {name}");
    }
}

fn explorer_lines(
    path: Option<&Path>,
    active_file: &str,
    mode: ExplorerMode,
    collapsed_dirs: &HashSet<PathBuf>,
    theme: &Theme,
) -> (Vec<Line<'static>>, Vec<ExplorerHit>) {
    let root = workspace_root_for(path);
    let mut lines = Vec::new();
    let mut hits = Vec::new();

    let flat_label = "[flat]";
    let nested_label = "[nested]";
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(
            flat_label.to_string(),
            toggle_style(mode == ExplorerMode::Flat, theme),
        ),
        Span::raw(" "),
        Span::styled(
            nested_label.to_string(),
            toggle_style(mode == ExplorerMode::Nested, theme),
        ),
    ]));
    hits.push(ExplorerHit {
        row: 0,
        start: 1,
        end: 1 + flat_label.chars().count() as u16,
        action: ExplorerAction::ToggleMode(ExplorerMode::Flat),
    });
    hits.push(ExplorerHit {
        row: 0,
        start: 2 + flat_label.chars().count() as u16,
        end: 2 + flat_label.chars().count() as u16 + nested_label.chars().count() as u16,
        action: ExplorerAction::ToggleMode(ExplorerMode::Nested),
    });

    match mode {
        ExplorerMode::Flat => {
            let mut files = Vec::new();
            collect_markdown_files(&root, &mut files);
            files.sort();
            for file in files {
                let row = lines.len() as u16;
                let label = relative_label(&root, &file);
                let active =
                    active_file.ends_with(&label) || active_file == file.display().to_string();
                lines.push(Line::from(vec![
                    Span::styled(
                        " M ".to_string(),
                        Style::default()
                            .fg(rgb(theme.link))
                            .bg(rgb(if active {
                                theme.active_row
                            } else {
                                theme.panel_bg
                            }))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        label.clone(),
                        Style::default()
                            .fg(rgb(if active {
                                theme.accent_highlight
                            } else {
                                theme.link
                            }))
                            .bg(rgb(if active {
                                theme.active_row
                            } else {
                                theme.panel_bg
                            }))
                            .add_modifier(if active {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ]));
                hits.push(ExplorerHit {
                    row,
                    start: 0,
                    end: u16::MAX,
                    action: ExplorerAction::OpenFile(file),
                });
            }
        }
        ExplorerMode::Nested => {
            let root_count = markdown_count(&root);
            let collapsed = collapsed_dirs.contains(&root);
            let chevron = if collapsed { "▸" } else { "▾" };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{chevron} "),
                    Style::default()
                        .fg(rgb(theme.link))
                        .bg(rgb(theme.panel_bg))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ({root_count})", workspace_name(Some(&root))),
                    Style::default()
                        .fg(rgb(theme.link))
                        .bg(rgb(theme.panel_bg))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            hits.push(ExplorerHit {
                row: 1,
                start: 0,
                end: u16::MAX,
                action: ExplorerAction::ToggleDir(root.clone()),
            });
            if !collapsed {
                walk_dir_nested(
                    &root,
                    0,
                    active_file,
                    collapsed_dirs,
                    theme,
                    &mut lines,
                    &mut hits,
                );
            }
        }
    }

    if lines.len() == 1 {
        lines.push(Line::from(Span::styled(
            " no markdown files".to_string(),
            Style::default()
                .fg(rgb(theme.text_muted))
                .bg(rgb(theme.panel_bg)),
        )));
    }

    (lines, hits)
}

fn walk_dir_nested(
    path: &Path,
    depth: usize,
    active_file: &str,
    collapsed_dirs: &HashSet<PathBuf>,
    theme: &Theme,
    lines: &mut Vec<Line<'static>>,
    hits: &mut Vec<ExplorerHit>,
) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        (
            entry.file_type().map(|ft| !ft.is_dir()).unwrap_or(true),
            entry.path(),
        )
    });
    for entry in entries {
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "target" || name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let row = lines.len() as u16;
        let indent = "  ".repeat(depth + 1);
        if file_type.is_dir() {
            let count = markdown_count(&entry_path);
            if count == 0 {
                continue;
            }
            let collapsed = collapsed_dirs.contains(&entry_path);
            let chevron = if collapsed { "▸" } else { "▾" };
            lines.push(Line::from(vec![
                Span::styled(
                    indent.clone(),
                    Style::default()
                        .fg(rgb(theme.text_muted))
                        .bg(rgb(theme.panel_bg)),
                ),
                Span::styled(
                    format!("{chevron} "),
                    Style::default()
                        .fg(rgb(theme.link))
                        .bg(rgb(theme.panel_bg))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{name} ({count})"),
                    Style::default().fg(rgb(theme.link)).bg(rgb(theme.panel_bg)),
                ),
            ]));
            hits.push(ExplorerHit {
                row,
                start: 0,
                end: u16::MAX,
                action: ExplorerAction::ToggleDir(entry_path.clone()),
            });
            if !collapsed {
                walk_dir_nested(
                    &entry_path,
                    depth + 1,
                    active_file,
                    collapsed_dirs,
                    theme,
                    lines,
                    hits,
                );
            }
        } else if is_markdown_file(&entry_path) {
            let active =
                active_file.ends_with(&name) || active_file == entry_path.display().to_string();
            lines.push(Line::from(vec![
                Span::styled(
                    indent,
                    Style::default()
                        .fg(rgb(theme.text_muted))
                        .bg(rgb(if active {
                            theme.active_row
                        } else {
                            theme.panel_bg
                        })),
                ),
                Span::styled(
                    " M ".to_string(),
                    Style::default()
                        .fg(rgb(theme.link))
                        .bg(rgb(if active {
                            theme.active_row
                        } else {
                            theme.panel_bg
                        }))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    name,
                    Style::default()
                        .fg(rgb(if active {
                            theme.accent_highlight
                        } else {
                            theme.link
                        }))
                        .bg(rgb(if active {
                            theme.active_row
                        } else {
                            theme.panel_bg
                        }))
                        .add_modifier(if active {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            ]));
            hits.push(ExplorerHit {
                row,
                start: 0,
                end: u16::MAX,
                action: ExplorerAction::OpenFile(entry_path),
            });
        }
    }
}

fn outline_lines(
    blocks: &[DocBlock],
    active_block: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, Vec<OutlineHit>) {
    let headings = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            DocBlock::Heading { level, inlines } => {
                Some((index, *level, mdtui_core::inline_text(inlines)))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let max_prefix = headings
        .iter()
        .filter_map(|(_, _, text)| split_heading_number(text))
        .map(|(prefix, _)| prefix.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    for (index, level, text) in headings {
        let row = lines.len() as u16;
        let active = active_block == index;
        let bg = if active {
            theme.active_row
        } else {
            theme.panel_bg
        };
        let title_style = Style::default()
            .fg(rgb(if active {
                theme.accent_highlight
            } else {
                theme.link
            }))
            .bg(rgb(bg))
            .add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let prefix_style = Style::default()
            .fg(rgb(theme.link))
            .bg(rgb(bg))
            .add_modifier(Modifier::BOLD);
        let indent = "  ".repeat(level.saturating_sub(1) as usize);
        let line = if let Some((prefix, title)) = split_heading_number(&text) {
            Line::from(vec![
                Span::styled(
                    indent,
                    Style::default().fg(rgb(theme.text_muted)).bg(rgb(bg)),
                ),
                Span::styled(
                    format!("{prefix:>width$} ", width = max_prefix),
                    prefix_style,
                ),
                Span::styled(title.to_string(), title_style),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!("{indent}• "), prefix_style),
                Span::styled(text, title_style),
            ])
        };
        lines.push(line);
        hits.push(OutlineHit { row, block: index });
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            " no headings".to_string(),
            Style::default()
                .fg(rgb(theme.text_muted))
                .bg(rgb(theme.panel_bg)),
        )));
    }
    (lines, hits)
}

fn tab_entries(
    path: Option<&Path>,
    active_file: &str,
    hidden_tabs: &HashSet<String>,
) -> Vec<(String, bool)> {
    let parent = workspace_root_for(path);
    let Ok(entries) = fs::read_dir(&parent) else {
        return vec![(file_leaf(active_file), true)];
    };
    let mut tabs = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            ((lower.ends_with(".md") || lower.ends_with(".markdown"))
                && !hidden_tabs.contains(&name))
            .then_some(name)
        })
        .collect::<Vec<_>>();
    tabs.sort();
    tabs.truncate(4);
    let active_leaf = file_leaf(active_file);
    if !hidden_tabs.contains(&active_leaf) && !tabs.iter().any(|name| active_file.ends_with(name)) {
        tabs.insert(0, active_leaf);
    }
    tabs.into_iter()
        .map(|name| {
            let active = active_file.ends_with(&name);
            (name, active)
        })
        .collect()
}

fn tab_path(state: &TuiState, name: &str) -> PathBuf {
    if state
        .path
        .as_deref()
        .is_some_and(|path| file_leaf(&path.display().to_string()) == name)
    {
        state.path.clone().unwrap_or_else(|| PathBuf::from(name))
    } else {
        workspace_root_for(state.path.as_deref()).join(name)
    }
}

fn selection_line_range(state: &TuiState, rendered: &Rendered) -> Option<(u16, u16)> {
    let selection = state.app.editor.selection?;
    if selection.is_collapsed() {
        return None;
    }
    let (_, start_y) = cursor_position(&selection.anchor, &rendered.display.items)?;
    let (_, end_y) = cursor_position(&selection.head, &rendered.display.items)?;
    Some((start_y.min(end_y), start_y.max(end_y)))
}

fn has_selection(state: &TuiState) -> bool {
    state
        .app
        .editor
        .selection
        .is_some_and(|selection| !selection.is_collapsed())
}

fn style_rendered_line(
    index: usize,
    line: &str,
    next_line: Option<&str>,
    in_code: &mut bool,
    theme: &Theme,
    current_y: Option<u16>,
    selection_range: Option<(u16, u16)>,
) -> Line<'static> {
    if line.starts_with('╭') && line.contains('┬') {
        *in_code = true;
        return style_code_header(line, theme);
    }
    if *in_code && line.contains('📋') {
        return style_code_toolbar(line, theme);
    }
    if *in_code && line.starts_with("├") {
        return Line::from(Span::styled(line.to_string(), code_border(theme)));
    }
    if *in_code && line.starts_with("╰") {
        *in_code = false;
        return Line::from(Span::styled(line.to_string(), code_border(theme)));
    }
    if *in_code && line.starts_with('│') {
        return style_code_body(line, theme);
    }
    if line.starts_with("▰ ") {
        return Line::from(Span::styled(
            " ".repeat(line.chars().count().max(1)),
            Style::default().bg(rgb(theme.app_bg)),
        ));
    }

    let mut style = Style::default()
        .fg(rgb(theme.text_primary))
        .bg(rgb(theme.app_bg));
    if next_line.is_some_and(is_heading_rule) {
        style = style
            .fg(rgb(theme.accent_highlight))
            .add_modifier(Modifier::BOLD);
    } else if is_heading_rule(line) {
        style = Style::default()
            .fg(rgb(theme.border_strong))
            .bg(rgb(theme.app_bg));
    }
    if let Some((start, end)) = selection_range
        && (start as usize..=end as usize).contains(&index)
    {
        style = style.bg(rgb(theme.active_row));
    } else if current_y.is_some_and(|y| y as usize == index) {
        style = style.bg(rgb(theme.panel_raised));
    }
    Line::from(Span::styled(line.to_string(), style))
}

fn is_heading_rule(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|ch| ch == '═' || ch == '─')
}

fn style_code_header(line: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(line.to_string(), code_border(theme)))
}

fn style_code_toolbar(line: &str, theme: &Theme) -> Line<'static> {
    let Some(copy_start) = line.find('📋') else {
        return Line::from(Span::styled(line.to_string(), code_text(theme)));
    };
    let before = &line[..copy_start];
    let after = &line[copy_start + '📋'.len_utf8()..];
    Line::from(vec![
        Span::styled(before.to_string(), code_text(theme)),
        Span::styled(
            "📋".to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(rgb(theme.panel_bg))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(after.to_string(), code_text(theme)),
    ])
}

fn style_code_body(line: &str, theme: &Theme) -> Line<'static> {
    let content = line.trim_start_matches('│');
    let Some((number, rest)) = content.split_once("│ ") else {
        return Line::from(Span::styled(line.to_string(), code_text(theme)));
    };
    let rest = rest.strip_suffix(" │").unwrap_or(rest).to_string();
    let mut spans = vec![
        Span::styled("│".to_string(), code_gutter_border(theme)),
        Span::styled(number.to_string(), code_gutter(theme)),
        Span::styled("│ ".to_string(), code_gutter_border(theme)),
    ];
    spans.extend(highlight_code(&rest, theme));
    spans.push(Span::styled(" │".to_string(), code_border(theme)));
    Line::from(spans)
}

fn highlight_code(source: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if ch == '"' || ch == '\'' || ch == '`' {
            let quote = ch;
            let mut token = String::new();
            token.push(chars.next().unwrap_or(quote));
            while let Some(next) = chars.next() {
                token.push(next);
                if next == '\\' {
                    if let Some(escaped) = chars.next() {
                        token.push(escaped);
                    }
                    continue;
                }
                if next == quote {
                    break;
                }
            }
            spans.push(Span::styled(token, code_string(theme)));
        } else if ch == '#'
            || (ch == '/' && chars.clone().nth(1) == Some('/'))
            || (ch == '-' && chars.clone().nth(1) == Some('-'))
        {
            let mut token = String::new();
            for next in chars.by_ref() {
                token.push(next);
            }
            spans.push(Span::styled(token, code_comment(theme)));
        } else if ch.is_ascii_digit() {
            let mut token = String::new();
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_hexdigit() || matches!(next, '_' | '.' | 'x' | 'o' | 'b') {
                    token.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            spans.push(Span::styled(token, code_number(theme)));
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let mut token = String::new();
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    token.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            let style = if is_keyword(&token) {
                code_keyword(theme)
            } else if matches!(
                token.as_str(),
                "true" | "false" | "None" | "null" | "Some" | "Ok" | "Err"
            ) {
                code_number(theme)
            } else if chars.peek() == Some(&'(') {
                code_call(theme)
            } else if token
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase())
            {
                code_type(theme)
            } else {
                code_text(theme)
            };
            spans.push(Span::styled(token, style));
        } else if "{}[]()<>:=!+-/*.,|&;%".contains(ch) {
            spans.push(Span::styled(
                chars.next().unwrap_or(ch).to_string(),
                code_punct(theme),
            ));
        } else {
            spans.push(Span::styled(
                chars.next().unwrap_or(ch).to_string(),
                code_text(theme),
            ));
        }
    }
    spans
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "def"
            | "fn"
            | "let"
            | "const"
            | "var"
            | "pub"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "where"
            | "use"
            | "mod"
            | "crate"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "loop"
            | "break"
            | "continue"
            | "match"
            | "class"
            | "import"
            | "from"
            | "export"
            | "async"
            | "await"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "raise"
            | "in"
            | "as"
            | "self"
            | "Self"
            | "super"
            | "type"
            | "interface"
            | "switch"
            | "case"
            | "default"
    )
}

fn copy_osc52(text: &str) -> io::Result<()> {
    let payload = STANDARD.encode(text);
    let sequence = format!("\u{1b}]52;c;{payload}\u{7}");
    let mut stdout = io::stdout();
    stdout.write_all(sequence.as_bytes())?;
    stdout.flush()
}

fn cursor_position(cursor: &Cursor, items: &[mdtui_render::DisplayItem]) -> Option<(u16, u16)> {
    for item in items {
        let Some(base) = item.cursor else {
            continue;
        };
        match (*cursor, base) {
            (
                Cursor::Text { block, offset },
                Cursor::Text {
                    block: base_block,
                    offset: base_offset,
                },
            ) if block == base_block
                && offset >= base_offset
                && offset <= base_offset + usize::from(item.rect.width) =>
            {
                let local = offset.saturating_sub(base_offset) as u16;
                return Some((item.rect.x.saturating_add(local), item.rect.y));
            }
            (
                Cursor::ListItem {
                    block,
                    item: list_item,
                    offset,
                },
                Cursor::ListItem {
                    block: base_block,
                    item: base_item,
                    offset: base_offset,
                },
            ) if block == base_block
                && list_item == base_item
                && offset >= base_offset
                && offset <= base_offset + usize::from(item.rect.width) =>
            {
                let local = offset.saturating_sub(base_offset) as u16;
                return Some((item.rect.x.saturating_add(local), item.rect.y));
            }
            (
                Cursor::TableCell {
                    block,
                    row,
                    col,
                    offset,
                },
                Cursor::TableCell {
                    block: base_block,
                    row: base_row,
                    col: base_col,
                    offset: base_offset,
                },
            ) if block == base_block
                && row == base_row
                && col == base_col
                && offset >= base_offset
                && offset <= base_offset + usize::from(item.rect.width) =>
            {
                let local = offset.saturating_sub(base_offset) as u16;
                return Some((item.rect.x.saturating_add(local), item.rect.y));
            }
            (
                Cursor::Checkbox {
                    block,
                    item: checkbox_item,
                },
                Cursor::Checkbox {
                    block: base_block,
                    item: base_item,
                },
            ) if block == base_block && checkbox_item == base_item => {
                return Some((item.rect.x, item.rect.y));
            }
            _ => {}
        }
    }
    None
}

fn nearest_cursor_on_row(x: u16, y: u16, items: &[mdtui_render::DisplayItem]) -> Option<Cursor> {
    let mut best: Option<(u16, Cursor)> = None;
    for item in items {
        let Some(cursor) = item.cursor else {
            continue;
        };
        if item.rect.y != y {
            continue;
        }
        let start = item.rect.x;
        let end = item.rect.x.saturating_add(item.rect.width);
        let distance = if x < start {
            start - x
        } else {
            x.saturating_sub(end)
        };
        let mapped = match cursor {
            Cursor::Text { block, offset } => Cursor::Text {
                block,
                offset: offset + usize::from(x.saturating_sub(start).min(item.rect.width)),
            },
            Cursor::ListItem {
                block,
                item: list_item,
                offset,
            } => Cursor::ListItem {
                block,
                item: list_item,
                offset: offset + usize::from(x.saturating_sub(start).min(item.rect.width)),
            },
            Cursor::TableCell {
                block,
                row,
                col,
                offset,
            } => Cursor::TableCell {
                block,
                row,
                col,
                offset: offset + usize::from(x.saturating_sub(start).min(item.rect.width)),
            },
            Cursor::Checkbox { block, item } => Cursor::Checkbox { block, item },
        };
        if best
            .as_ref()
            .is_none_or(|(best_distance, _)| distance < *best_distance)
        {
            best = Some((distance, mapped));
        }
    }
    best.map(|(_, cursor)| cursor)
}

fn emit_kitty_headlines<W: Write>(writer: &mut W, state: &mut TuiState) -> io::Result<()> {
    if !state.kitty_graphics {
        return Ok(());
    }
    let Some(rendered) = state.last_rendered.clone() else {
        return Ok(());
    };
    let commands = visible_headline_commands(&rendered, state.last_doc_area, state.scroll, state)?;
    let signature = commands.join("\x1f");
    if state.last_kitty_signature.as_ref() == Some(&signature) {
        return Ok(());
    }
    let mut output = String::new();
    if state.last_kitty_signature.is_some() || !commands.is_empty() {
        output.push_str("\u{1b}_Ga=d,d=A\u{1b}\\");
    }
    output.push_str(&signature);
    writer.write_all(output.as_bytes())?;
    writer.flush()?;
    state.last_kitty_signature = Some(signature);
    Ok(())
}

fn visible_headline_commands(
    rendered: &Rendered,
    area: Rect,
    scroll: u16,
    state: &mut TuiState,
) -> io::Result<Vec<String>> {
    let viewport_rows = area.height.saturating_sub(2);
    let mut out = Vec::new();
    for item in &rendered.display.items {
        if item.kind != DisplayKind::HeadlinePlacement {
            continue;
        }
        if item.rect.y < scroll || item.rect.y >= scroll.saturating_add(viewport_rows) {
            continue;
        }
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let cols = item.rect.width.max(8);
        let key = format!("{HEADLINE_RASTER_VERSION}:{text}:{cols}");
        let png = if let Some(bytes) = state.headline_png_cache.get(&key) {
            bytes.clone()
        } else {
            request_headline_raster(state, key.clone(), text.to_string(), cols, 2);
            continue;
        };
        let row = area
            .y
            .saturating_add(1)
            .saturating_add(item.rect.y.saturating_sub(scroll))
            + 1;
        let col = area.x.saturating_add(1).saturating_add(item.rect.x) + 1;
        out.push(format!(
            "\u{1b}[{row};{col}H{}",
            kitty_png_apc(&png, cols, 2)
        ));
    }
    Ok(out)
}

fn kitty_png_apc(png: &[u8], cols: u16, rows: u16) -> String {
    let encoded = STANDARD.encode(png);
    let mut chunks = String::new();
    let total = encoded.len();
    let mut index = 0;
    let mut first = true;
    while index < total {
        let end = (index + 4096).min(total);
        let chunk = &encoded[index..end];
        if first {
            let more = if end < total { 1 } else { 0 };
            chunks.push_str(&format!(
                "\u{1b}_Ga=T,f=100,c={cols},r={rows},C=1,m={more};{chunk}\u{1b}\\"
            ));
            first = false;
        } else {
            let more = if end < total { 1 } else { 0 };
            chunks.push_str(&format!("\u{1b}_Gm={more};{chunk}\u{1b}\\"));
        }
        index = end;
    }
    chunks
}

fn request_headline_raster(state: &mut TuiState, key: String, text: String, cols: u16, rows: u16) {
    if state.headline_png_cache.contains_key(&key) || state.pending_headline_jobs.contains(&key) {
        return;
    }
    state.pending_headline_jobs.insert(key.clone());
    let tx = state.headline_raster_tx.clone();
    thread::spawn(move || {
        let _ = tx.send((key, headline_png(&text, cols, rows)));
    });
}

fn headline_png(text: &str, cols: u16, rows: u16) -> io::Result<Vec<u8>> {
    let cell_w = 16u32;
    let cell_h = 32u32;
    let width = u32::from(cols.max(8)) * cell_w;
    let height = u32::from(rows).max(2) * cell_h;
    let mut img = RgbaImage::from_pixel(width, height, Rgba([15, 12, 8, 255]));
    if draw_headline_font(&mut img, text).is_err() {
        let fg = Rgba([230, 168, 90, 255]);
        let glow = Rgba([216, 154, 74, 110]);
        let shadow = Rgba([42, 24, 15, 180]);
        let scale = 2u32;
        let mut pen_x = 8u32;
        let baseline_y = 3u32;
        for ch in text.chars() {
            if let Some(glyph) = font8x8::BASIC_FONTS.get(ch) {
                draw_glyph(&mut img, glyph, pen_x + 1, baseline_y + 1, scale, shadow);
                draw_glyph(&mut img, glyph, pen_x, baseline_y, scale, glow);
                draw_glyph(&mut img, glyph, pen_x, baseline_y, scale, fg);
            }
            pen_x = pen_x.saturating_add(8 * scale + 2);
            if pen_x + 16 >= width {
                break;
            }
        }
    }

    let mut bytes = Vec::new();
    let encoder = PngEncoder::new(&mut bytes);
    encoder
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ColorType::Rgba8.into(),
        )
        .map_err(io::Error::other)?;
    Ok(bytes)
}

fn draw_headline_font(image: &mut RgbaImage, text: &str) -> io::Result<()> {
    let font = load_headline_font()?;
    let mut layout = FontLayout::new(CoordinateSystem::PositiveYDown);
    let size = (image.height() as f32 * 0.82).max(20.0);
    layout.reset(&LayoutSettings {
        x: 12.0,
        y: 0.0,
        max_width: Some(image.width().saturating_sub(18) as f32),
        max_height: Some(image.height() as f32),
        ..LayoutSettings::default()
    });
    layout.append(&[&font], &TextStyle::new(text, size, 0));
    if layout.glyphs().is_empty() {
        return Err(io::Error::other("no glyphs laid out"));
    }
    let (min_y, max_y) = headline_bounds(&font, &layout)?;
    let offset_y = ((image.height() as f32 - (max_y - min_y)) / 2.0 - min_y).round() as i32;
    let shadow = Rgba([44, 28, 12, 96]);
    let top = Rgba([255, 220, 160, 255]);
    let bottom = Rgba([217, 138, 82, 255]);
    for glyph in layout.glyphs() {
        let (metrics, bitmap) = font.rasterize_config(glyph.key);
        paint_alpha_bitmap(
            image,
            glyph.x as i32 + 1,
            glyph.y as i32 + offset_y + 1,
            metrics.width,
            metrics.height,
            &bitmap,
            shadow,
        );
        paint_alpha_bitmap_gradient(
            image,
            glyph.x as i32,
            glyph.y as i32 + offset_y,
            metrics.width,
            metrics.height,
            &bitmap,
            (top, bottom),
        );
    }
    Ok(())
}

fn headline_bounds(font: &Font, layout: &FontLayout) -> io::Result<(f32, f32)> {
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for glyph in layout.glyphs() {
        let (metrics, _) = font.rasterize_config(glyph.key);
        if metrics.height == 0 {
            continue;
        }
        min_y = min_y.min(glyph.y);
        max_y = max_y.max(glyph.y + metrics.height as f32);
    }
    if min_y == f32::MAX || max_y == f32::MIN {
        return Err(io::Error::other("no visible headline glyph bounds"));
    }
    Ok((min_y, max_y))
}

fn load_headline_font() -> io::Result<Font> {
    let mut db = Database::new();
    db.load_system_fonts();
    let query = Query {
        families: &[
            Family::Name("DejaVu Sans"),
            Family::Name("Noto Sans"),
            Family::Name("Liberation Sans"),
            Family::Name("Ubuntu"),
            Family::SansSerif,
        ],
        weight: Weight::BOLD,
        style: FontStyle::Normal,
        ..Query::default()
    };
    let id = db
        .query(&query)
        .ok_or_else(|| io::Error::other("no usable system font"))?;
    let (bytes, face_index) = db
        .with_face_data(id, |data, face_index| (data.to_vec(), face_index))
        .ok_or_else(|| io::Error::other("unable to load font bytes"))?;
    Font::from_bytes(
        bytes,
        FontSettings {
            collection_index: face_index,
            ..FontSettings::default()
        },
    )
    .map_err(io::Error::other)
}

fn paint_alpha_bitmap(
    image: &mut RgbaImage,
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    bitmap: &[u8],
    color: Rgba<u8>,
) {
    for row in 0..height {
        for col in 0..width {
            let px = x + col as i32;
            let py = y + row as i32;
            if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                continue;
            }
            let coverage = bitmap[row * width + col];
            if coverage == 0 {
                continue;
            }
            let alpha = (u16::from(coverage) * u16::from(color[3]) / 255) as u8;
            let dst = image.get_pixel_mut(px as u32, py as u32);
            let inv = 255u16.saturating_sub(u16::from(alpha));
            dst[0] =
                ((u16::from(color[0]) * u16::from(alpha) + u16::from(dst[0]) * inv) / 255) as u8;
            dst[1] =
                ((u16::from(color[1]) * u16::from(alpha) + u16::from(dst[1]) * inv) / 255) as u8;
            dst[2] =
                ((u16::from(color[2]) * u16::from(alpha) + u16::from(dst[2]) * inv) / 255) as u8;
            dst[3] = (u16::from(alpha) + (u16::from(dst[3]) * inv) / 255).min(255) as u8;
        }
    }
}

fn paint_alpha_bitmap_gradient(
    image: &mut RgbaImage,
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    bitmap: &[u8],
    gradient: (Rgba<u8>, Rgba<u8>),
) {
    let (top, bottom) = gradient;
    for row in 0..height {
        for col in 0..width {
            let px = x + col as i32;
            let py = y + row as i32;
            if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                continue;
            }
            let coverage = bitmap[row * width + col];
            if coverage == 0 {
                continue;
            }
            let t = py as f32 / image.height().max(1) as f32;
            let color = Rgba([
                lerp_channel(top[0], bottom[0], t),
                lerp_channel(top[1], bottom[1], t),
                lerp_channel(top[2], bottom[2], t),
                lerp_channel(top[3], bottom[3], t),
            ]);
            let alpha = (u16::from(coverage) * u16::from(color[3]) / 255) as u8;
            let dst = image.get_pixel_mut(px as u32, py as u32);
            let inv = 255u16.saturating_sub(u16::from(alpha));
            dst[0] =
                ((u16::from(color[0]) * u16::from(alpha) + u16::from(dst[0]) * inv) / 255) as u8;
            dst[1] =
                ((u16::from(color[1]) * u16::from(alpha) + u16::from(dst[1]) * inv) / 255) as u8;
            dst[2] =
                ((u16::from(color[2]) * u16::from(alpha) + u16::from(dst[2]) * inv) / 255) as u8;
            dst[3] = (u16::from(alpha) + (u16::from(dst[3]) * inv) / 255).min(255) as u8;
        }
    }
}

fn lerp_channel(start: u8, end: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (start as f32 + (end as f32 - start as f32) * t).round() as u8
}

fn draw_glyph(image: &mut RgbaImage, glyph: [u8; 8], x: u32, y: u32, scale: u32, color: Rgba<u8>) {
    for (row_index, row) in glyph.iter().enumerate() {
        for col_index in 0..8 {
            if row & (1 << col_index) == 0 {
                continue;
            }
            let px = x + col_index as u32 * scale;
            let py = y + row_index as u32 * scale;
            for dy in 0..scale {
                for dx in 0..scale {
                    if px + dx < image.width() && py + dy < image.height() {
                        image.put_pixel(px + dx, py + dy, color);
                    }
                }
            }
        }
    }
}

fn detect_kitty_support() -> bool {
    env::var("KITTY_WINDOW_ID").is_ok()
        || env::var("TERM")
            .map(|term| term.contains("kitty") || term.contains("xterm-kitty"))
            .unwrap_or(false)
        || env::var("TERM_PROGRAM")
            .map(|program| {
                let lower = program.to_ascii_lowercase();
                lower.contains("ghostty") || lower.contains("kitty")
            })
            .unwrap_or(false)
}

fn workspace_root(path: &Path) -> PathBuf {
    let candidate = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf())
}

fn workspace_name(path: Option<&Path>) -> String {
    workspace_root_for(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string())
}

fn file_leaf(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn cursor_block(cursor: Cursor) -> usize {
    match cursor {
        Cursor::Text { block, .. } => block,
        Cursor::ListItem { block, .. } => block,
        Cursor::TableCell { block, .. } => block,
        Cursor::Checkbox { block, .. } => block,
    }
}

fn clamp_scroll(offset: u16, content: usize, viewport: usize) -> u16 {
    if viewport == 0 {
        return 0;
    }
    offset.min(content.saturating_sub(viewport) as u16)
}

fn workspace_root_for(path: Option<&Path>) -> PathBuf {
    path.map_or_else(
        || env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        workspace_root,
    )
}

fn toggle_style(active: bool, theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(if active {
            theme.accent_highlight
        } else {
            theme.link
        }))
        .bg(rgb(if active {
            theme.panel_raised
        } else {
            theme.panel_bg
        }))
        .add_modifier(if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown"))
}

fn collect_markdown_files(path: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "target" || name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_markdown_files(&entry_path, out);
        } else if is_markdown_file(&entry_path) {
            out.push(entry_path);
        }
    }
}

fn markdown_count(path: &Path) -> usize {
    let mut files = Vec::new();
    collect_markdown_files(path, &mut files);
    files.len()
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn split_heading_number(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    if bytes.first().is_none_or(|byte| !byte.is_ascii_digit()) {
        return None;
    }
    let mut end = 0usize;
    let mut seen_period = false;
    while end < bytes.len() {
        let byte = bytes[end];
        if byte.is_ascii_digit() {
            end += 1;
            continue;
        }
        if byte == b'.' {
            seen_period = true;
            end += 1;
            if end < bytes.len() && bytes[end] == b' ' {
                let prefix = text[..end].to_string();
                let title = text[end + 1..].trim_start();
                return Some((prefix, title));
            }
            continue;
        }
        break;
    }
    seen_period.then_some((text[..end].to_string(), text[end..].trim_start()))
}

fn compact_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out = text.chars().take(max.saturating_sub(1)).collect::<String>();
        out.push('…');
        out
    }
}

fn put(buf: &mut Buffer, x: &mut u16, y: u16, right: u16, text: &str, style: Style) {
    for ch in text.chars() {
        if *x >= right {
            break;
        }
        buf.set_string(*x, y, ch.to_string(), style);
        *x = x.saturating_add(1);
    }
}

fn rgb(hex: &str) -> Color {
    let clean = hex.trim_start_matches('#');
    let Some(red) = u8::from_str_radix(&clean[0..2], 16).ok() else {
        return Color::Reset;
    };
    let Some(green) = u8::from_str_radix(&clean[2..4], 16).ok() else {
        return Color::Reset;
    };
    let Some(blue) = u8::from_str_radix(&clean[4..6], 16).ok() else {
        return Color::Reset;
    };
    Color::Rgb(red, green, blue)
}

fn code_border(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.border_strong))
        .bg(rgb(theme.panel_raised))
}

fn code_gutter_border(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.border))
        .bg(rgb(theme.panel_bg))
}

fn code_gutter(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_muted))
        .bg(rgb(theme.panel_bg))
}

fn code_text(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_primary))
        .bg(rgb(theme.panel_raised))
}

fn code_keyword(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.accent_primary))
        .bg(rgb(theme.panel_raised))
        .add_modifier(Modifier::BOLD)
}

fn code_string(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.success))
        .bg(rgb(theme.panel_raised))
}

fn code_call(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.accent_highlight))
        .bg(rgb(theme.panel_raised))
        .add_modifier(Modifier::BOLD)
}

fn code_type(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.link))
        .bg(rgb(theme.panel_raised))
}

fn code_punct(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_secondary))
        .bg(rgb(theme.panel_raised))
}

fn code_comment(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_muted))
        .bg(rgb(theme.panel_raised))
        .add_modifier(Modifier::ITALIC)
}

fn code_number(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.link))
        .bg(rgb(theme.panel_raised))
}
