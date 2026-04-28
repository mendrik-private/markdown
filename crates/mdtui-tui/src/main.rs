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
use mdtui_core::{Block as DocBlock, Cursor, Direction, Inline};
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
const HEADLINE_RASTER_VERSION: u32 = 8;
const HEADLINE_DEBUG_SLAB: bool = true;

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
struct ExplorerModeHit {
    start: u16,
    end: u16,
    action: ExplorerAction,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StylePopupAction {
    Bold,
    Italic,
    Strike,
    Code,
    Superscript,
    Subscript,
    Clear,
    Quote,
}

impl StylePopupAction {
    fn shortcut(self) -> &'static str {
        match self {
            Self::Bold => "Ctrl+B",
            Self::Italic => "Ctrl+I",
            Self::Strike => "Ctrl+Shift+X",
            Self::Code => "Ctrl+E",
            Self::Superscript => "Ctrl+.",
            Self::Subscript => "Ctrl+,",
            Self::Clear => "Enter",
            Self::Quote => "Enter",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Strike => "Strike",
            Self::Code => "Code",
            Self::Superscript => "Superscript",
            Self::Subscript => "Subscript",
            Self::Clear => "Clear style",
            Self::Quote => "Block quote",
        }
    }
}

#[derive(Clone, Debug)]
struct StylePopupHit {
    row: u16,
    start: u16,
    end: u16,
    action: StylePopupAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RenderCacheKey {
    version: u64,
    options: RenderOptions,
    active_block: usize,
}

#[derive(Clone, Debug)]
struct TabHit {
    start: u16,
    end: u16,
    close_start: u16,
    close_end: u16,
    name: String,
}

#[derive(Clone, Debug)]
enum TabControlAction {
    NewFile,
    OpenMenu,
}

#[derive(Clone, Debug)]
struct TabControlHit {
    start: u16,
    end: u16,
    action: TabControlAction,
}

#[derive(Clone, Debug)]
enum FilePopup {
    Menu,
    NewFile { input: String },
    RenameFile { input: String, path: PathBuf },
    DeleteFile { path: PathBuf },
}

#[derive(Clone, Debug)]
enum FilePopupAction {
    Create,
    Rename,
    Delete,
    Cancel,
    OpenRename,
    OpenDelete,
}

#[derive(Clone, Debug)]
struct FilePopupHit {
    row: u16,
    start: u16,
    end: u16,
    action: FilePopupAction,
}

#[derive(Clone, Copy, Debug)]
struct CodeThumbDrag {
    block: usize,
    track_start: u16,
    track_width: u16,
    thumb_width: u16,
    content_width: usize,
    visible_width: usize,
    grab_offset: u16,
}

#[derive(Clone, Copy, Debug)]
struct WrapSliderTrack {
    start: u16,
    slots: u16,
    min: u16,
    max: u16,
}

#[derive(Clone, Copy, Debug)]
struct PanelScrollDrag {
    area_y: u16,
    track_height: u16,
    thumb_height: u16,
    content: usize,
    viewport: usize,
    grab_offset: u16,
}

struct RenderLineContext<'a> {
    state: &'a TuiState,
    rendered: &'a Rendered,
    theme: &'a Theme,
    current_y: Option<u16>,
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
    last_render_key: Option<RenderCacheKey>,
    drag_anchor: Option<(u16, u16)>,
    last_style_popup: Option<Rect>,
    style_popup_hits: Vec<StylePopupHit>,
    style_popup_selected: StylePopupAction,
    style_popup_hover: Option<StylePopupAction>,
    explorer_mode: ExplorerMode,
    explorer_scroll: u16,
    outline_scroll: u16,
    collapsed_dirs: HashSet<PathBuf>,
    explorer_hits: Vec<ExplorerHit>,
    explorer_mode_hits: Vec<ExplorerModeHit>,
    outline_hits: Vec<OutlineHit>,
    status_hits: Vec<StatusHit>,
    tab_hits: Vec<TabHit>,
    tab_control_hits: Vec<TabControlHit>,
    hidden_tabs: HashSet<String>,
    file_popup: Option<FilePopup>,
    file_popup_hits: Vec<FilePopupHit>,
    last_file_popup: Option<Rect>,
    code_thumb_drag: Option<CodeThumbDrag>,
    panel_scroll_drag: Option<PanelScrollDrag>,
    wrap_slider_track: Option<WrapSliderTrack>,
    wrap_slider_drag: Option<WrapSliderTrack>,
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
        let mut state = Self {
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
            last_render_key: None,
            drag_anchor: None,
            last_style_popup: None,
            style_popup_hits: Vec::new(),
            style_popup_selected: StylePopupAction::Bold,
            style_popup_hover: None,
            explorer_mode: ExplorerMode::Nested,
            explorer_scroll: 0,
            outline_scroll: 0,
            collapsed_dirs: HashSet::new(),
            explorer_hits: Vec::new(),
            explorer_mode_hits: Vec::new(),
            outline_hits: Vec::new(),
            status_hits: Vec::new(),
            tab_hits: Vec::new(),
            tab_control_hits: Vec::new(),
            hidden_tabs: HashSet::new(),
            file_popup: None,
            file_popup_hits: Vec::new(),
            last_file_popup: None,
            code_thumb_drag: None,
            panel_scroll_drag: None,
            wrap_slider_track: None,
            wrap_slider_drag: None,
            kitty_graphics: detect_kitty_support(),
            last_kitty_signature: None,
            headline_png_cache: HashMap::new(),
            pending_headline_jobs: HashSet::new(),
            headline_raster_tx,
            headline_raster_rx,
        };
        load_view_state(&mut state);
        state
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

    if state.file_popup.is_some() {
        match key.code {
            KeyCode::Esc => state.file_popup = None,
            KeyCode::Enter => confirm_file_popup(state),
            KeyCode::Char('n') if matches!(state.file_popup, Some(FilePopup::Menu)) => {
                open_new_file_popup(state)
            }
            KeyCode::Char('r') if matches!(state.file_popup, Some(FilePopup::Menu)) => {
                open_rename_file_popup(state)
            }
            KeyCode::Char('d') if matches!(state.file_popup, Some(FilePopup::Menu)) => {
                open_delete_file_popup(state)
            }
            KeyCode::Backspace => {
                if let Some(FilePopup::NewFile { input } | FilePopup::RenameFile { input, .. }) =
                    state.file_popup.as_mut()
                {
                    input.pop();
                }
            }
            KeyCode::Char(ch)
                if is_plain_text_key(key) || ch == '/' || ch == '.' || ch == '-' || ch == '_' =>
            {
                if let Some(FilePopup::NewFile { input } | FilePopup::RenameFile { input, .. }) =
                    state.file_popup.as_mut()
                {
                    input.push(ch);
                }
            }
            _ => {}
        }
        state.file_popup_hits.clear();
        return false;
    }

    if has_selection(state) && state.app.editor.show_style_popover && key.modifiers.is_empty() {
        match key.code {
            KeyCode::Left => {
                step_style_popup_selection(state, -1);
                state.style_popup_hover = None;
                return false;
            }
            KeyCode::Right => {
                step_style_popup_selection(state, 1);
                state.style_popup_hover = None;
                return false;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                apply_style_popup_action(state, active_style_popup_action(state));
                state.dirty = true;
                state.message = "style toggled".to_string();
                return false;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => state.save(),
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 1;
            state.message = "column mode 1".to_string();
            persist_view_state(state);
        }
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 2;
            state.message = "column mode 2".to_string();
            persist_view_state(state);
        }
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.preferred_column = None;
            state.app.render_options.columns = 3;
            state.message = "column mode 3".to_string();
            persist_view_state(state);
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
        KeyCode::Char('.') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_superscript();
            state.preferred_column = None;
            state.dirty = true;
        }
        KeyCode::Char(',') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.app.apply_subscript();
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
    let max_y = rendered.lines.len() as i32;
    if target_y < 0 || target_y >= max_y {
        return;
    }
    let mut target = None;
    while target_y >= 0 && target_y < max_y {
        let row = target_y as u16;
        target = nearest_cursor_on_row(preferred_x, row, &rendered.display.items)
            .or_else(|| hit_test(preferred_x, row, &rendered.display))
            .or_else(|| hit_test(0, row, &rendered.display));
        if target.is_some() {
            break;
        }
        target_y += delta.signum();
    }
    let Some(cursor) = target.map(|cursor| normalize_headline_cursor(state, cursor)) else {
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
        Some(DocBlock::Heading { level, .. }) if state.kitty_graphics && matches!(level, 1 | 2) => {
            2
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
    if let Some(popup) = state.last_file_popup {
        if mouse.column >= popup.x
            && mouse.column < popup.x.saturating_add(popup.width)
            && mouse.row >= popup.y
            && mouse.row < popup.y.saturating_add(popup.height)
        {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                click_file_popup(state, popup, mouse.column, mouse.row);
            }
            return;
        }
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            state.file_popup = None;
            state.file_popup_hits.clear();
        }
    }
    if state.last_style_popup.is_some()
        && !matches!(
            mouse.kind,
            MouseEventKind::Moved | MouseEventKind::Drag(MouseButton::Left)
        )
        && state.style_popup_hover.is_some()
    {
        state.style_popup_hover = None;
    }
    if let Some(popup) = state.last_style_popup
        && mouse.column >= popup.x
        && mouse.column < popup.x.saturating_add(popup.width)
        && mouse.row >= popup.y
        && mouse.row < popup.y.saturating_add(popup.height)
    {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                click_style_popup(state, popup, mouse.column, mouse.row)
            }
            MouseEventKind::Moved | MouseEventKind::Drag(MouseButton::Left) => {
                hover_style_popup(state, popup, mouse.column, mouse.row);
            }
            _ => {}
        }
        return;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
        hide_style_popup(state);
    }
    if state.style_popup_hover.take().is_some() {
        state.dirty = true;
    }

    if let Some(track) = state.wrap_slider_drag {
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                let local_x = mouse.column.saturating_sub(state.last_status_area.x);
                update_wrap_width_from_slider(state, track, local_x);
                return;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                state.wrap_slider_drag = None;
                return;
            }
            _ => {}
        }
    }

    if let Some(drag) = state.panel_scroll_drag {
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                update_panel_scroll_drag(state, drag, mouse.row);
                return;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                state.panel_scroll_drag = None;
                return;
            }
            _ => {}
        }
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
        let local_x = mouse.column.saturating_sub(state.last_status_area.x);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(track) = state.wrap_slider_track.filter(|track| {
                    local_x >= track.start
                        && local_x < track.start.saturating_add(track.slots.max(1))
                }) {
                    state.wrap_slider_drag = Some(track);
                    update_wrap_width_from_slider(state, track, local_x);
                } else if let Some(action) = state
                    .status_hits
                    .iter()
                    .find(|hit| local_x >= hit.start && local_x < hit.end)
                    .map(|hit| hit.action.clone())
                {
                    run_status_action(state, &action);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(track) = state.wrap_slider_drag {
                    update_wrap_width_from_slider(state, track, local_x);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => state.wrap_slider_drag = None,
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
                .tab_control_hits
                .iter()
                .find(|hit| local_x >= hit.start && local_x < hit.end)
                .cloned()
            {
                match hit.action {
                    TabControlAction::NewFile => open_new_file_popup(state),
                    TabControlAction::OpenMenu => state.file_popup = Some(FilePopup::Menu),
                }
            } else if let Some(hit) = state
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
        && mouse.row == state.last_explorer_area.y
        && mouse.column >= state.last_explorer_area.x
        && mouse.column
            < state
                .last_explorer_area
                .x
                .saturating_add(state.last_explorer_area.width)
    {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let local_x = mouse.column.saturating_sub(state.last_explorer_area.x);
            if let Some(action) = state
                .explorer_mode_hits
                .iter()
                .find(|hit| local_x >= hit.start && local_x < hit.end)
                .map(|hit| hit.action.clone())
            {
                run_explorer_action(state, &action);
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
    if mouse.column == area.x.saturating_add(area.width.saturating_sub(1))
        && mouse.row > area.y
        && mouse.row < area.y.saturating_add(area.height.saturating_sub(1))
        && let MouseEventKind::Down(MouseButton::Left) = mouse.kind
    {
        let viewport = usize::from(area.height.saturating_sub(2));
        let content = state
            .last_rendered
            .as_ref()
            .map(|rendered| rendered.lines.len())
            .unwrap_or(0);
        if viewport > 0 && content > viewport {
            let thumb_height = ((viewport * viewport) / content).max(1).min(viewport) as u16;
            let track_height = area.height.saturating_sub(2);
            let drag = PanelScrollDrag {
                area_y: area.y + 1,
                track_height,
                thumb_height,
                content,
                viewport,
                grab_offset: mouse
                    .row
                    .saturating_sub(area.y + 1)
                    .min(thumb_height.saturating_sub(1)),
            };
            state.panel_scroll_drag = Some(drag);
            update_panel_scroll_drag(state, drag, mouse.row);
        }
        return;
    }
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

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(rendered) = &state.last_rendered
                && let Some(action) = action_at(x, y, &rendered.display)
            {
                match action {
                    DisplayAction::CopyCodeBlock { .. } | DisplayAction::FollowLink { .. } => {
                        run_action(state, action)
                    }
                    DisplayAction::ScrollCodeBlock {
                        block,
                        track_start,
                        track_width,
                        thumb_width,
                        content_width,
                        visible_width,
                    } => {
                        state.code_thumb_drag = Some(CodeThumbDrag {
                            block,
                            track_start,
                            track_width,
                            thumb_width,
                            content_width: usize::from(content_width),
                            visible_width: usize::from(visible_width),
                            grab_offset: x
                                .saturating_sub(track_start)
                                .min(thumb_width.saturating_sub(1)),
                        });
                    }
                }
                return;
            }
            state.drag_anchor = Some((x, y));
            state.preferred_column = None;
            state.app.click(x, y);
            ensure_cursor_visible(state);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(drag) = state.code_thumb_drag {
                state.preferred_column = None;
                update_code_thumb_drag(state, drag, x);
                return;
            }
            if let Some(anchor) = state.drag_anchor {
                state.preferred_column = None;
                state.app.drag_select(anchor, (x, y));
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            state.drag_anchor = None;
            state.code_thumb_drag = None;
            state.panel_scroll_drag = None;
        }
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
        .constraints([Constraint::Length(43), Constraint::Min(40)])
        .split(body_area);
    let sidebar = columns[0];
    let doc_column = columns[1];

    let sidebar_split = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(sidebar);
    let doc_split = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(doc_column);
    let tabs_area = doc_split[0];
    let doc_body_area = doc_split[1];
    let doc_area = Rect {
        x: doc_body_area.x,
        y: doc_body_area.y.saturating_sub(1),
        width: doc_body_area.width,
        height: doc_body_area.height.saturating_add(1),
    };
    state.last_tabs_area = Rect {
        height: tabs_area.height.saturating_sub(1),
        ..tabs_area
    };
    state.last_doc_area = doc_area;
    state.last_explorer_area = sidebar_split[0];
    state.last_outline_area = sidebar_split[1];
    state.last_status_area = status_area;

    let doc_inner_width = doc_area
        .width
        .saturating_sub(4)
        .max(24)
        .min(state.wrap_width.max(24));
    let headline_width = doc_area.width.saturating_sub(2).max(8);
    state.app.render_options = RenderOptions {
        width: doc_inner_width,
        heading_width: headline_width,
        kitty_graphics: state.kitty_graphics,
        show_status: false,
        ..state.app.render_options.clone()
    };
    let render_key = RenderCacheKey {
        version: state.app.editor.document.version,
        options: state.app.render_options.clone(),
        active_block: cursor_block(state.app.editor.cursor),
    };
    let rendered = if state.last_render_key.as_ref() == Some(&render_key) {
        state.last_rendered.clone().unwrap_or_else(|| {
            let mut rendered =
                render_document(&state.app.editor.document, state.app.render_options.clone());
            materialize_active_headline_fallback(state, &mut rendered);
            rendered
        })
    } else {
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(state, &mut rendered);
        state.last_render_key = Some(render_key);
        state.last_rendered = Some(rendered.clone());
        rendered
    };
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
        sidebar_split[1].width.saturating_sub(2),
    );
    state.outline_hits = outline_hits;
    state.outline_scroll = clamp_scroll(
        state.outline_scroll,
        outline_lines.len(),
        usize::from(sidebar_split[1].height.saturating_sub(2)),
    );

    draw_explorer(
        frame,
        sidebar_split[0],
        state,
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
    draw_tabs(frame, tabs_area, state, &theme);
    draw_status(frame, status_area, state, &rendered, &theme);

    if has_selection(state) && state.app.editor.show_style_popover {
        let selection_rects = selection_rects(state, &rendered);
        state.last_style_popup = Some(draw_style_popover(
            frame,
            area,
            doc_area,
            state.scroll,
            &selection_rects,
            state,
            &theme,
        ));
    } else {
        state.last_style_popup = None;
        state.style_popup_hits.clear();
    }
    if state.show_help {
        draw_help(frame, area, &theme);
    }
    if state.file_popup.is_some() {
        state.last_file_popup = Some(draw_file_popup(frame, area, state, &theme));
    } else {
        state.last_file_popup = None;
        state.file_popup_hits.clear();
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
    state.tab_control_hits.clear();

    let top_y = area.y;
    let label_y = area.y + if area.height > 1 { 1 } else { 0 };
    let roof = area.height >= 2;
    let join_y = area.y + area.height.saturating_sub(1);
    let join = area.height >= 3;
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

        if active && join {
            let mut join_x = start_x;
            put(
                buf,
                &mut join_x,
                join_y,
                right,
                if start_x == area.x { "│" } else { "┘" },
                border,
            );
            if width > 2 {
                put(
                    buf,
                    &mut join_x,
                    join_y,
                    right,
                    &" ".repeat((width - 2) as usize),
                    fill,
                );
            }
            put(
                buf,
                &mut join_x,
                join_y,
                right,
                if start_x.saturating_add(width) >= right {
                    "│"
                } else {
                    "└"
                },
                border,
            );
        }

        x = start_x.saturating_add(width);
    }

    let controls_width = "  +  ⋮ ".chars().count() as u16;
    let mut controls_x = right.saturating_sub(controls_width.saturating_add(1));
    let start = controls_x.saturating_sub(area.x);
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
    state.tab_control_hits.push(TabControlHit {
        start: start.saturating_add(2),
        end: start.saturating_add(3),
        action: TabControlAction::NewFile,
    });
    state.tab_control_hits.push(TabControlHit {
        start: start.saturating_add(5),
        end: start.saturating_add(6),
        action: TabControlAction::OpenMenu,
    });
}

fn draw_explorer(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut TuiState,
    lines: &[Line<'static>],
    scroll: u16,
    theme: &Theme,
) {
    state.explorer_mode_hits.clear();
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
    if area.width <= 2 {
        return;
    }
    let buf = frame.buffer_mut();
    let right = area.x.saturating_add(area.width.saturating_sub(1));
    let flat_label = "[flat]";
    let nested_label = "[nested]";
    let controls = format!("{flat_label} {nested_label}");
    let start = right
        .saturating_sub(controls.chars().count() as u16)
        .saturating_sub(1);
    let mut x = start;
    put(
        buf,
        &mut x,
        area.y,
        right,
        " ",
        Style::default().bg(rgb(theme.panel_bg)),
    );
    let flat_start = x.saturating_sub(area.x);
    put(
        buf,
        &mut x,
        area.y,
        right,
        flat_label,
        toggle_style(state.explorer_mode == ExplorerMode::Flat, theme),
    );
    state.explorer_mode_hits.push(ExplorerModeHit {
        start: flat_start,
        end: flat_start.saturating_add(flat_label.chars().count() as u16),
        action: ExplorerAction::ToggleMode(ExplorerMode::Flat),
    });
    put(
        buf,
        &mut x,
        area.y,
        right,
        " ",
        Style::default().bg(rgb(theme.panel_bg)),
    );
    let nested_start = x.saturating_sub(area.x);
    put(
        buf,
        &mut x,
        area.y,
        right,
        nested_label,
        toggle_style(state.explorer_mode == ExplorerMode::Nested, theme),
    );
    state.explorer_mode_hits.push(ExplorerModeHit {
        start: nested_start,
        end: nested_start.saturating_add(nested_label.chars().count() as u16),
        action: ExplorerAction::ToggleMode(ExplorerMode::Nested),
    });
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
    let active_headline_edit_rows = rendered
        .display
        .items
        .iter()
        .filter_map(|item| {
            let cursor = item.cursor?;
            match state.app.editor.document.blocks.get(cursor_block(cursor)) {
                Some(DocBlock::Heading { .. })
                    if cursor_block(cursor) == cursor_block(state.app.editor.cursor)
                        && item.kind == DisplayKind::TextRun =>
                {
                    Some(usize::from(item.rect.y))
                }
                _ => None,
            }
        })
        .collect::<HashSet<_>>();
    let headline_debug_rows = if HEADLINE_DEBUG_SLAB {
        rendered
            .display
            .items
            .iter()
            .filter(|item| item.kind == DisplayKind::HeadlinePlacement)
            .flat_map(|item| {
                (item.rect.y..item.rect.y.saturating_add(item.rect.height))
                    .map(usize::from)
                    .collect::<Vec<_>>()
            })
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let current_y =
        cursor_position(&state.app.editor.cursor, &rendered.display.items).map(|(_, y)| y);
    let selection_rects = selection_rects(state, rendered);
    let render_context = RenderLineContext {
        state,
        rendered,
        theme,
        current_y,
    };
    let lines = rendered
        .lines
        .iter()
        .enumerate()
        .scan(false, |in_code, (index, line)| {
            if active_headline_edit_rows.contains(&index) {
                Some(Line::from(Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(rgb(theme.accent_highlight))
                        .bg(rgb(theme.panel_bg))
                        .add_modifier(Modifier::BOLD),
                )))
            } else if headline_debug_rows.contains(&index) {
                let width = line.chars().count().max(1);
                let mut debug = "*******".repeat(width.div_ceil(7));
                debug.truncate(width);
                Some(Line::from(Span::styled(
                    debug,
                    Style::default()
                        .fg(rgb(theme.text_secondary))
                        .bg(rgb(theme.panel_bg)),
                )))
            } else {
                Some(style_rendered_line(
                    index,
                    line,
                    rendered.lines.get(index + 1).map(String::as_str),
                    &render_context,
                    in_code,
                ))
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .border_type(BorderType::Rounded)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(rgb(theme.border_strong)))
                    .style(Style::default().bg(rgb(theme.panel_bg))),
            )
            .scroll((state.scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );

    apply_selection_highlight(
        frame.buffer_mut(),
        area,
        state.scroll,
        &selection_rects,
        theme,
    );

    draw_scrollbar(
        frame,
        Rect {
            x: area.x + area.width.saturating_sub(1),
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
    state.wrap_slider_track = None;
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
    let slider_start = x.saturating_sub(area.x);
    state.wrap_slider_track = Some(WrapSliderTrack {
        start: slider_start,
        slots: slider_slots,
        min: min_wrap,
        max: max_wrap,
    });
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
        &format!(
            "  {} / {}  ctrl-1/2/3 cols  drag wrap slider  F1/? help",
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
        Line::from("  Ctrl-1/2/3 columns · drag the wrap slider"),
        Line::from("Editing"),
        Line::from("  type to edit · Enter split/create · Backspace/Delete remove"),
        Line::from("  Ctrl-B/I/E style · Ctrl-Shift-X strike · Ctrl-./, super/sub"),
        Line::from("  Ctrl-B bold · Ctrl-I italic · Ctrl-E code · Ctrl-Shift-X strike"),
        Line::from("Tables & Lists"),
        Line::from("  Tab / Shift+Tab move cells · Ctrl+Arrow add row/column"),
        Line::from("Mouse"),
        Line::from(
            "  click place cursor · drag select · drag wrap slider · click explorer/outline/status controls",
        ),
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

fn draw_style_popover(
    frame: &mut Frame<'_>,
    area: Rect,
    doc_area: Rect,
    scroll: u16,
    selection_rects: &[mdtui_render::Rect],
    state: &mut TuiState,
    theme: &Theme,
) -> Rect {
    let chips = style_popup_cells();
    let width = popup_line_width(&chips);
    let height = 3;
    let popup = anchored_style_popup(area, doc_area, scroll, selection_rects, width, height);
    state.style_popup_hits.clear();
    frame.render_widget(Clear, popup);
    let border_style = Style::default()
        .fg(rgb(theme.text_muted))
        .bg(rgb(theme.panel_bg));
    let fill_style = Style::default().bg(rgb(theme.panel_bg));
    frame.buffer_mut().set_style(popup, fill_style);
    let active = active_style_popup_action(state);
    let footer = format!(" {} - {} ", active.shortcut(), active.label());
    let footer_inner = build_footer_line(usize::from(width.saturating_sub(2)), &footer);
    let top = build_popup_top_line(&chips);
    let bottom = format!("└{footer_inner}┘");
    for (row, line) in [(0u16, top), (2u16, bottom)] {
        let y = popup.y + row;
        let mut x = popup.x;
        put(
            frame.buffer_mut(),
            &mut x,
            y,
            popup.x + popup.width,
            &line,
            border_style,
        );
    }
    render_popup_middle(frame.buffer_mut(), popup, state, theme, &chips);
    populate_style_popup_hits(state, &chips);
    popup
}

#[derive(Clone, Copy)]
struct StylePopupCell {
    label: &'static str,
    width: u16,
    action: Option<StylePopupAction>,
}

fn style_popup_cells() -> [StylePopupCell; 8] {
    [
        StylePopupCell {
            label: "B",
            width: 3,
            action: Some(StylePopupAction::Bold),
        },
        StylePopupCell {
            label: "I",
            width: 3,
            action: Some(StylePopupAction::Italic),
        },
        StylePopupCell {
            label: "S",
            width: 3,
            action: Some(StylePopupAction::Strike),
        },
        StylePopupCell {
            label: "</>",
            width: 5,
            action: Some(StylePopupAction::Code),
        },
        StylePopupCell {
            label: "x^",
            width: 4,
            action: Some(StylePopupAction::Superscript),
        },
        StylePopupCell {
            label: "x_",
            width: 4,
            action: Some(StylePopupAction::Subscript),
        },
        StylePopupCell {
            label: "clr",
            width: 5,
            action: Some(StylePopupAction::Clear),
        },
        StylePopupCell {
            label: ">",
            width: 3,
            action: Some(StylePopupAction::Quote),
        },
    ]
}

fn popup_line_width(cells: &[StylePopupCell]) -> u16 {
    2 + cells.iter().map(|cell| cell.width).sum::<u16>() + cells.len().saturating_sub(1) as u16
}

fn build_popup_top_line(cells: &[StylePopupCell]) -> String {
    let mut out = String::from("┌");
    for (index, cell) in cells.iter().enumerate() {
        out.push_str(&"─".repeat(cell.width as usize));
        out.push(if index + 1 == cells.len() {
            '┐'
        } else {
            '┬'
        });
    }
    out
}

fn build_footer_line(inner_width: usize, footer: &str) -> String {
    let footer_width = footer.chars().count().min(inner_width);
    let leading = inner_width.saturating_sub(footer_width) / 2;
    let trailing = inner_width.saturating_sub(footer_width + leading);
    format!(
        "{}{}{}",
        "─".repeat(leading),
        footer.chars().take(footer_width).collect::<String>(),
        "─".repeat(trailing)
    )
}

fn populate_style_popup_hits(state: &mut TuiState, cells: &[StylePopupCell]) {
    let mut x = 0u16;
    for cell in cells {
        if let Some(action) = cell.action {
            state.style_popup_hits.push(StylePopupHit {
                row: 1,
                start: x,
                end: x.saturating_add(cell.width),
                action,
            });
        }
        x = x.saturating_add(cell.width + 1);
    }
}

fn render_popup_middle(
    buf: &mut Buffer,
    popup: Rect,
    state: &TuiState,
    theme: &Theme,
    cells: &[StylePopupCell],
) {
    let y = popup.y + 1;
    let right = popup.x + popup.width;
    let border_style = Style::default()
        .fg(rgb(theme.text_muted))
        .bg(rgb(theme.panel_bg));
    let idle_style = Style::default()
        .fg(rgb(theme.accent_primary))
        .bg(rgb(theme.panel_bg));
    let active_style = Style::default()
        .fg(rgb(theme.panel_bg))
        .bg(rgb(theme.accent_highlight))
        .add_modifier(Modifier::BOLD);
    let mut x = popup.x;
    put(buf, &mut x, y, right, "│", border_style);
    for cell in cells {
        let action = cell.action;
        let selected = action == Some(active_style_popup_action(state));
        let style = if selected { active_style } else { idle_style };
        let label = if cell.label.chars().count() >= cell.width as usize {
            cell.label.to_string()
        } else {
            let left = (usize::from(cell.width) - cell.label.chars().count()) / 2;
            let right_pad = usize::from(cell.width)
                .saturating_sub(cell.label.chars().count())
                .saturating_sub(left);
            format!(
                "{}{}{}",
                " ".repeat(left),
                cell.label,
                " ".repeat(right_pad)
            )
        };
        put(buf, &mut x, y, right, &label, style);
        put(buf, &mut x, y, right, "│", border_style);
    }
}

fn anchored_style_popup(
    area: Rect,
    doc_area: Rect,
    scroll: u16,
    selection_rects: &[mdtui_render::Rect],
    width: u16,
    height: u16,
) -> Rect {
    let inner_left = doc_area.x.saturating_add(1);
    let inner_top = doc_area.y.saturating_add(1);
    let inner_bottom = doc_area.y.saturating_add(doc_area.height.saturating_sub(2));
    let mut min_x: Option<u16> = None;
    let mut max_x = 0u16;
    let mut min_y: Option<u16> = None;
    let mut max_y = 0u16;
    for rect in selection_rects {
        if rect.y < scroll {
            continue;
        }
        let screen_y = inner_top.saturating_add(rect.y - scroll);
        if screen_y < inner_top || screen_y > inner_bottom {
            continue;
        }
        let screen_x = inner_left.saturating_add(rect.x);
        min_x = Some(min_x.map_or(screen_x, |current: u16| current.min(screen_x)));
        max_x = max_x.max(screen_x.saturating_add(rect.width.saturating_sub(1)));
        min_y = Some(min_y.map_or(screen_y, |current: u16| current.min(screen_y)));
        max_y = max_y.max(screen_y);
    }
    let default = centered(area, width.min(area.width), height.min(area.height));
    let (Some(selection_left), Some(selection_top)) = (min_x, min_y) else {
        return default;
    };
    let selection_center = selection_left.saturating_add(max_x).saturating_div(2);
    let max_x_origin = area.x.saturating_add(area.width.saturating_sub(width));
    let mut x = selection_center.saturating_sub(width / 2);
    x = x.clamp(area.x, max_x_origin);
    let above_y = selection_top.saturating_sub(height);
    let y = if selection_top >= area.y.saturating_add(height) {
        above_y
    } else {
        max_y
            .saturating_add(1)
            .min(area.y.saturating_add(area.height.saturating_sub(height)))
    };
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
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
            "▒"
        } else {
            "│"
        };
        let style = if ch == "▒" {
            Style::default()
                .fg(rgb(theme.accent_highlight))
                .bg(rgb(theme.panel_bg))
        } else {
            Style::default()
                .fg(rgb(theme.accent_primary))
                .bg(rgb(theme.panel_bg))
        };
        lines.push(Line::from(Span::styled(ch.to_string(), style)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn update_panel_scroll_drag(state: &mut TuiState, drag: PanelScrollDrag, row: u16) {
    let max_thumb_start = drag.track_height.saturating_sub(drag.thumb_height);
    let thumb_start = row
        .saturating_sub(drag.area_y)
        .saturating_sub(drag.grab_offset)
        .min(max_thumb_start);
    let max_scroll = drag.content.saturating_sub(drag.viewport);
    state.scroll = if max_scroll == 0 || max_thumb_start == 0 {
        0
    } else {
        usize::from(thumb_start)
            .saturating_mul(max_scroll)
            .div_ceil(usize::from(max_thumb_start)) as u16
    };
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
        DisplayAction::FollowLink { block } => {
            state.preferred_column = None;
            state
                .app
                .editor
                .set_cursor(Cursor::Text { block, offset: 0 });
            state.message = "link jump".to_string();
            ensure_cursor_visible(state);
        }
        DisplayAction::ScrollCodeBlock { .. } => {}
    }
}

fn update_code_thumb_drag(state: &mut TuiState, drag: CodeThumbDrag, x: u16) {
    let max_thumb_start = drag.track_width.saturating_sub(drag.thumb_width);
    let thumb_start = x
        .saturating_sub(drag.track_start)
        .saturating_sub(drag.grab_offset)
        .min(max_thumb_start);
    let max_scroll = drag.content_width.saturating_sub(drag.visible_width);
    let scroll = if max_scroll == 0 || max_thumb_start == 0 {
        0
    } else {
        usize::from(thumb_start)
            .saturating_mul(max_scroll)
            .div_ceil(usize::from(max_thumb_start))
    };
    set_code_horizontal_scroll(state, drag.block, scroll);
}

fn set_code_horizontal_scroll(state: &mut TuiState, block: usize, scroll: usize) {
    if let Some((_, current)) = state
        .app
        .render_options
        .code_horizontal_scrolls
        .iter_mut()
        .find(|(entry_block, _)| *entry_block == block)
    {
        *current = scroll;
    } else {
        state
            .app
            .render_options
            .code_horizontal_scrolls
            .push((block, scroll));
        state
            .app
            .render_options
            .code_horizontal_scrolls
            .sort_by_key(|(entry_block, _)| *entry_block);
    }
}

fn click_style_popup(state: &mut TuiState, popup: Rect, x: u16, y: u16) {
    let local_y = y.saturating_sub(popup.y);
    let local_x = x.saturating_sub(popup.x.saturating_add(1));
    let Some(action) = state
        .style_popup_hits
        .iter()
        .find(|hit| hit.row == local_y && local_x >= hit.start && local_x < hit.end)
        .map(|hit| hit.action)
    else {
        return;
    };
    state.style_popup_selected = action;
    state.style_popup_hover = Some(action);
    apply_style_popup_action(state, action);
    state.dirty = true;
    state.message = "style toggled".to_string();
}

fn hover_style_popup(state: &mut TuiState, popup: Rect, x: u16, y: u16) {
    let local_y = y.saturating_sub(popup.y);
    let local_x = x.saturating_sub(popup.x.saturating_add(1));
    let hover = state
        .style_popup_hits
        .iter()
        .find(|hit| hit.row == local_y && local_x >= hit.start && local_x < hit.end)
        .map(|hit| hit.action);
    if state.style_popup_hover != hover {
        state.style_popup_hover = hover;
        state.dirty = true;
    }
}

fn draw_file_popup(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState, theme: &Theme) -> Rect {
    state.file_popup_hits.clear();
    let Some(popup_kind) = state.file_popup.clone() else {
        return Rect::default();
    };
    let (title, body, actions, width, height) = match &popup_kind {
        FilePopup::Menu => (
            " file ",
            vec![
                " [n] new file".to_string(),
                " [r] rename file".to_string(),
                " [d] delete file".to_string(),
            ],
            vec![
                ("new", FilePopupAction::OpenRename), // placeholder overwritten below
                ("rename", FilePopupAction::OpenRename),
                ("delete", FilePopupAction::OpenDelete),
            ],
            28,
            7,
        ),
        FilePopup::NewFile { input } => (
            " new file ",
            vec![
                " name".to_string(),
                format!(" {}", pad_width(input, 28)),
                " [enter] create  [esc] cancel".to_string(),
            ],
            vec![
                ("create", FilePopupAction::Create),
                ("cancel", FilePopupAction::Cancel),
            ],
            32,
            7,
        ),
        FilePopup::RenameFile { input, .. } => (
            " rename file ",
            vec![
                " name".to_string(),
                format!(" {}", pad_width(input, 28)),
                " [enter] rename  [esc] cancel".to_string(),
            ],
            vec![
                ("rename", FilePopupAction::Rename),
                ("cancel", FilePopupAction::Cancel),
            ],
            32,
            7,
        ),
        FilePopup::DeleteFile { path } => (
            " delete file ",
            vec![
                " delete this file?".to_string(),
                format!(" {}", file_leaf(&path.display().to_string())),
                " [enter] delete  [esc] cancel".to_string(),
            ],
            vec![
                ("delete", FilePopupAction::Delete),
                ("cancel", FilePopupAction::Cancel),
            ],
            32,
            7,
        ),
    };
    let popup = centered(area, width.min(area.width), height.min(area.height));
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(
            Style::default()
                .fg(rgb(theme.accent_primary))
                .bg(rgb(theme.panel_raised))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(rgb(theme.panel_raised)))
        .border_style(
            Style::default()
                .fg(rgb(theme.text_muted))
                .bg(rgb(theme.panel_raised)),
        );
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    frame.buffer_mut().set_style(
        inner,
        Style::default()
            .fg(rgb(theme.text_primary))
            .bg(rgb(theme.panel_raised)),
    );
    for (row, line) in body.iter().enumerate() {
        if row as u16 >= inner.height {
            break;
        }
        frame.buffer_mut().set_stringn(
            inner.x,
            inner.y + row as u16,
            line,
            usize::from(inner.width),
            if row == 1
                && !matches!(
                    state.file_popup,
                    Some(FilePopup::DeleteFile { .. }) | Some(FilePopup::Menu)
                )
            {
                Style::default()
                    .fg(rgb(theme.accent_highlight))
                    .bg(rgb(theme.app_bg))
            } else {
                Style::default()
                    .fg(rgb(theme.text_primary))
                    .bg(rgb(theme.panel_raised))
            },
        );
    }
    match popup_kind {
        FilePopup::Menu => {
            state.file_popup_hits.push(FilePopupHit {
                row: 1,
                start: 1,
                end: inner.width.saturating_sub(1),
                action: FilePopupAction::Create,
            });
            state.file_popup_hits.push(FilePopupHit {
                row: 2,
                start: 1,
                end: inner.width.saturating_sub(1),
                action: FilePopupAction::OpenRename,
            });
            state.file_popup_hits.push(FilePopupHit {
                row: 3,
                start: 1,
                end: inner.width.saturating_sub(1),
                action: FilePopupAction::OpenDelete,
            });
        }
        _ => {
            let actions_y = inner.y + inner.height.saturating_sub(1);
            let action_text = actions
                .iter()
                .map(|(label, _)| format!("[{label}]"))
                .collect::<Vec<_>>()
                .join("  ");
            frame.buffer_mut().set_stringn(
                inner.x,
                actions_y,
                &action_text,
                usize::from(inner.width),
                Style::default()
                    .fg(rgb(theme.text_secondary))
                    .bg(rgb(theme.panel_raised)),
            );
            let mut cursor = 0u16;
            for (label, action) in actions {
                let len = label.chars().count() as u16 + 2;
                state.file_popup_hits.push(FilePopupHit {
                    row: actions_y.saturating_sub(popup.y),
                    start: cursor,
                    end: cursor.saturating_add(len),
                    action,
                });
                cursor = cursor.saturating_add(len + 2);
            }
        }
    }
    popup
}

fn click_file_popup(state: &mut TuiState, popup: Rect, x: u16, y: u16) {
    let local_y = y.saturating_sub(popup.y);
    let local_x = x.saturating_sub(popup.x.saturating_add(1));
    let Some(action) = state
        .file_popup_hits
        .iter()
        .find(|hit| hit.row == local_y && local_x >= hit.start && local_x < hit.end)
        .map(|hit| hit.action.clone())
    else {
        return;
    };
    match action {
        FilePopupAction::Create => {
            if matches!(state.file_popup, Some(FilePopup::Menu)) {
                open_new_file_popup(state);
            } else {
                confirm_file_popup(state);
            }
        }
        FilePopupAction::Rename => confirm_file_popup(state),
        FilePopupAction::Delete => confirm_file_popup(state),
        FilePopupAction::Cancel => state.file_popup = None,
        FilePopupAction::OpenRename => open_rename_file_popup(state),
        FilePopupAction::OpenDelete => open_delete_file_popup(state),
    }
}

fn active_style_popup_action(state: &TuiState) -> StylePopupAction {
    state
        .style_popup_hover
        .unwrap_or(state.style_popup_selected)
}

fn step_style_popup_selection(state: &mut TuiState, delta: i32) {
    let actions = [
        StylePopupAction::Bold,
        StylePopupAction::Italic,
        StylePopupAction::Strike,
        StylePopupAction::Code,
        StylePopupAction::Superscript,
        StylePopupAction::Subscript,
        StylePopupAction::Clear,
        StylePopupAction::Quote,
    ];
    let current = actions
        .iter()
        .position(|action| *action == state.style_popup_selected)
        .unwrap_or(0) as i32;
    let next = (current + delta).rem_euclid(actions.len() as i32) as usize;
    state.style_popup_selected = actions[next];
    state.message = format!(
        "{} - {}",
        state.style_popup_selected.shortcut(),
        state.style_popup_selected.label()
    );
    state.dirty = true;
}

fn apply_style_popup_action(state: &mut TuiState, action: StylePopupAction) {
    match action {
        StylePopupAction::Bold => state.app.apply_bold(),
        StylePopupAction::Italic => state.app.apply_italic(),
        StylePopupAction::Strike => state.app.apply_strike(),
        StylePopupAction::Code => {
            if state.app.editor.selection_covers_active_text()
                || matches!(state.app.editor.cursor, Cursor::Text { block, .. } if matches!(
                    state.app.editor.document.blocks.get(block),
                    Some(DocBlock::CodeBlock { .. })
                ))
            {
                state.app.apply_code_block();
            } else {
                state.app.apply_code();
            }
        }
        StylePopupAction::Superscript => state.app.apply_superscript(),
        StylePopupAction::Subscript => state.app.apply_subscript(),
        StylePopupAction::Clear => state.app.clear_styles(),
        StylePopupAction::Quote => state.app.apply_block_quote(),
    }
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
            persist_view_state(state);
        }
        StatusAction::SetColumns(columns) => {
            state.app.render_options.columns = columns.clamp(1, 3);
            state.scroll = 0;
            state.message = format!("column mode {}", state.app.render_options.columns);
            persist_view_state(state);
        }
    }
}

fn update_wrap_width_from_slider(state: &mut TuiState, track: WrapSliderTrack, local_x: u16) {
    let slot = local_x
        .saturating_sub(track.start)
        .min(track.slots.saturating_sub(1));
    let width = track.min
        + (slot.saturating_mul(track.max.saturating_sub(track.min))
            / track.slots.saturating_sub(1).max(1));
    run_status_action(state, &StatusAction::SetWrapWidth(width));
}

fn open_file(state: &mut TuiState, path: &Path) {
    match fs::read_to_string(path) {
        Ok(source) => {
            let mut app = App::from_markdown(path.display().to_string(), &source);
            app.render_options = state.app.render_options.clone();
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

fn open_new_file_popup(state: &mut TuiState) {
    state.file_popup = Some(FilePopup::NewFile {
        input: String::new(),
    });
    state.file_popup_hits.clear();
}

fn open_rename_file_popup(state: &mut TuiState) {
    let Some(path) = state.path.clone() else {
        state.message = "rename unavailable".to_string();
        return;
    };
    let input = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("note.md")
        .to_string();
    state.file_popup = Some(FilePopup::RenameFile { input, path });
    state.file_popup_hits.clear();
}

fn open_delete_file_popup(state: &mut TuiState) {
    let Some(path) = state.path.clone() else {
        state.message = "delete unavailable".to_string();
        return;
    };
    state.file_popup = Some(FilePopup::DeleteFile { path });
    state.file_popup_hits.clear();
}

fn confirm_file_popup(state: &mut TuiState) {
    let Some(popup) = state.file_popup.clone() else {
        return;
    };
    match popup {
        FilePopup::Menu => {}
        FilePopup::NewFile { input } => {
            if let Err(error) = create_file_from_input(state, &input) {
                state.message = format!("create failed: {error}");
            }
        }
        FilePopup::RenameFile { input, path } => {
            if let Err(error) = rename_file_from_input(state, &path, &input) {
                state.message = format!("rename failed: {error}");
            }
        }
        FilePopup::DeleteFile { path } => {
            if let Err(error) = delete_file(state, &path) {
                state.message = format!("delete failed: {error}");
            }
        }
    }
}

fn create_file_from_input(state: &mut TuiState, input: &str) -> io::Result<()> {
    let Some(path) = named_path_for_input(state, input) else {
        state.message = "name required".to_string();
        return Ok(());
    };
    if path.exists() {
        state.message = "file already exists".to_string();
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, "")?;
    state.file_popup = None;
    open_file(state, &path);
    state.message = format!("created {}", file_leaf(&path.display().to_string()));
    Ok(())
}

fn rename_file_from_input(state: &mut TuiState, old_path: &Path, input: &str) -> io::Result<()> {
    let Some(new_path) = named_path_for_input(state, input) else {
        state.message = "name required".to_string();
        return Ok(());
    };
    if new_path == old_path {
        state.file_popup = None;
        return Ok(());
    }
    if new_path.exists() {
        state.message = "file already exists".to_string();
        return Ok(());
    }
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(old_path, &new_path)?;
    let old_name = file_leaf(&old_path.display().to_string());
    let new_name = file_leaf(&new_path.display().to_string());
    if state.hidden_tabs.remove(&old_name) {
        state.hidden_tabs.insert(new_name.clone());
    }
    state.file_popup = None;
    open_file(state, &new_path);
    state.message = format!("renamed {old_name} -> {new_name}");
    Ok(())
}

fn delete_file(state: &mut TuiState, path: &Path) -> io::Result<()> {
    let mut files = Vec::new();
    collect_markdown_files(&workspace_root_for(state.path.as_deref()), &mut files);
    let replacement = files.into_iter().find(|candidate| candidate != path);
    if state.path.as_deref() == Some(path) && replacement.is_none() {
        state.message = "cannot delete last file".to_string();
        return Ok(());
    }
    fs::remove_file(path)?;
    let name = file_leaf(&path.display().to_string());
    state.hidden_tabs.remove(&name);
    state.file_popup = None;
    if let Some(next) = replacement {
        open_file(state, &next);
    }
    state.message = format!("deleted {name}");
    Ok(())
}

fn named_path_for_input(state: &TuiState, input: &str) -> Option<PathBuf> {
    let trimmed = input.trim().trim_start_matches("./");
    if trimmed.is_empty() {
        return None;
    }
    let mut path = workspace_root_for(state.path.as_deref()).join(trimmed);
    if path.extension().is_none() {
        path.set_extension("md");
    }
    Some(path)
}

fn view_state_path(state: &TuiState) -> PathBuf {
    workspace_root_for(state.path.as_deref()).join(".mdtui-view")
}

fn load_view_state(state: &mut TuiState) {
    let Ok(content) = fs::read_to_string(view_state_path(state)) else {
        return;
    };
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("wrap_width=") {
            if let Ok(width) = value.parse::<u16>() {
                state.wrap_width = width.clamp(24, 120);
            }
        } else if let Some(value) = line.strip_prefix("columns=")
            && let Ok(columns) = value.parse::<u16>()
        {
            state.app.render_options.columns = columns.clamp(1, 3) as u8;
        }
    }
}

fn persist_view_state(state: &TuiState) {
    let path = view_state_path(state);
    let _ = fs::write(
        path,
        format!(
            "wrap_width={}\ncolumns={}\n",
            state.wrap_width, state.app.render_options.columns
        ),
    );
}

fn hide_style_popup(state: &mut TuiState) {
    if state.app.editor.show_style_popover {
        state.app.editor.show_style_popover = false;
        state.style_popup_hover = None;
        state.dirty = true;
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
                        "▪ ".to_string(),
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
                row: 0,
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

    if lines.is_empty() {
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
                    "▪ ".to_string(),
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
    width: u16,
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
        let active = active_block == index;
        let bg = if active {
            theme.active_row
        } else {
            theme.panel_bg
        };
        let depth = level.saturating_sub(1);
        let title_color = rgb(&mix_hex(
            if active {
                theme.accent_highlight
            } else {
                theme.accent_primary
            },
            theme.panel_bg,
            u16::from(depth).saturating_mul(10),
        ));
        let title_style = Style::default()
            .fg(title_color)
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
        let (prefix_marker, title_text) = if let Some((prefix, title)) = split_heading_number(&text)
        {
            (
                format!("{prefix:>width$} ", width = max_prefix),
                title.to_string(),
            )
        } else {
            ("• ".to_string(), text)
        };
        let prefix_text = format!("{indent}{prefix_marker}");
        let prefix_width = prefix_text.chars().count();
        let available = usize::from(width).saturating_sub(prefix_width).max(8);
        let wrapped = wrap_outline_text(&title_text, available);
        for (wrap_index, part) in wrapped.iter().enumerate() {
            let row = lines.len() as u16;
            if wrap_index == 0 {
                lines.push(Line::from(vec![
                    Span::styled(
                        indent.clone(),
                        Style::default().fg(rgb(theme.text_muted)).bg(rgb(bg)),
                    ),
                    Span::styled(prefix_marker.clone(), prefix_style),
                    Span::styled(part.clone(), title_style),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        " ".repeat(prefix_width),
                        Style::default().fg(rgb(theme.text_muted)).bg(rgb(bg)),
                    ),
                    Span::styled(part.clone(), title_style),
                ]));
            }
            hits.push(OutlineHit { row, block: index });
        }
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
    context: &RenderLineContext<'_>,
    in_code: &mut bool,
) -> Line<'static> {
    let theme = context.theme;
    if line.starts_with('╭') && line.contains('┬') {
        *in_code = true;
        return style_code_header(line, theme);
    }
    if *in_code && line.contains("│ copy │") {
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
        return style_code_body(
            line,
            theme,
            code_source_for_row(
                index,
                context.rendered,
                &context.state.app.editor.document,
                &context.state.app.render_options.code_horizontal_scrolls,
            ),
        );
    }
    if line.starts_with("▰ ") {
        return Line::from(Span::styled(
            " ".repeat(line.chars().count().max(1)),
            Style::default().bg(rgb(theme.panel_bg)),
        ));
    }

    let background = if context.current_y.is_some_and(|y| y as usize == index) {
        rgb(theme.panel_raised)
    } else {
        rgb(theme.panel_bg)
    };
    let mut style = Style::default().fg(rgb(theme.text_primary)).bg(background);
    if next_line.is_some_and(is_heading_rule) {
        style = style
            .fg(rgb(theme.accent_highlight))
            .add_modifier(Modifier::BOLD);
    } else if is_heading_rule(line) {
        style = Style::default().fg(rgb(theme.border)).bg(background);
    } else if let Some(spans) = styled_text_spans_for_row(
        index,
        line,
        context.state,
        context.rendered,
        theme,
        background,
        next_line,
    ) {
        return Line::from(spans);
    } else if let Some(rest) = line.strip_prefix("▌ ") {
        return Line::from(vec![
            Span::styled(
                "▌".to_string(),
                Style::default()
                    .fg(rgb(theme.border))
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {rest}"),
                Style::default()
                    .fg(rgb(theme.text_secondary))
                    .bg(background),
            ),
        ]);
    }
    Line::from(Span::styled(line.to_string(), style))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlinePaintStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
    superscript: bool,
    subscript: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StyledFragment {
    text: String,
    style: InlinePaintStyle,
}

fn styled_text_spans_for_row(
    index: usize,
    line: &str,
    state: &TuiState,
    rendered: &Rendered,
    theme: &Theme,
    background: Color,
    next_line: Option<&str>,
) -> Option<Vec<Span<'static>>> {
    let mut row_items = rendered
        .display
        .items
        .iter()
        .filter(|item| {
            item.rect.y as usize == index
                && item.kind != DisplayKind::HeadlinePlacement
                && !(item.kind == DisplayKind::Adornment && item.text.trim().is_empty())
        })
        .collect::<Vec<_>>();
    if row_items.is_empty() {
        return None;
    }
    row_items.sort_by_key(|item| item.rect.x);
    let line_chars = line.chars().collect::<Vec<_>>();
    let heading = next_line.is_some_and(is_heading_rule);
    let toc_row = row_items
        .iter()
        .any(|item| matches!(item.action, Some(DisplayAction::FollowLink { .. })));
    let blockquote_row = row_items
        .iter()
        .any(|item| item.kind == DisplayKind::Adornment && item.text.starts_with("▌ "));
    let numbered_row = !toc_row
        && row_items.first().is_some_and(|item| {
            item.kind == DisplayKind::Adornment && is_numbered_marker(&item.text)
        });
    let mut spans = Vec::new();
    let mut cursor_x = 0usize;
    for item in row_items {
        let item_x = usize::from(item.rect.x);
        if item_x > cursor_x && cursor_x < line_chars.len() {
            spans.push(Span::styled(
                line_chars[cursor_x..item_x.min(line_chars.len())]
                    .iter()
                    .collect::<String>(),
                base_document_style(theme, background, heading, numbered_row, blockquote_row),
            ));
        }
        spans.extend(spans_for_display_item(
            item,
            state,
            theme,
            background,
            heading,
            numbered_row,
            blockquote_row,
        ));
        cursor_x = item_x.saturating_add(usize::from(item.rect.width));
    }
    if cursor_x < line_chars.len() {
        spans.push(Span::styled(
            line_chars[cursor_x..].iter().collect::<String>(),
            base_document_style(theme, background, heading, numbered_row, blockquote_row),
        ));
    }
    Some(spans)
}

fn spans_for_display_item(
    item: &mdtui_render::DisplayItem,
    state: &TuiState,
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Vec<Span<'static>> {
    if matches!(item.action, Some(DisplayAction::FollowLink { .. })) {
        return toc_row_spans(&item.text, theme, background);
    }
    if item.kind == DisplayKind::Adornment {
        return adornment_spans(
            &item.text,
            theme,
            background,
            heading,
            numbered_row,
            blockquote_row,
        );
    }
    let visible_len = item.text.chars().count();
    if let Some(fragments) =
        styled_fragments_for_item(item, &state.app.editor.document, visible_len)
    {
        let mut spans = fragments_to_spans(
            &fragments,
            theme,
            background,
            heading,
            numbered_row,
            blockquote_row,
        );
        let painted = fragments
            .iter()
            .map(|fragment| fragment.text.chars().count())
            .sum::<usize>();
        if painted < usize::from(item.rect.width) {
            spans.push(Span::styled(
                " ".repeat(usize::from(item.rect.width) - painted),
                base_document_style(theme, background, heading, numbered_row, blockquote_row),
            ));
        }
        return spans;
    }
    vec![Span::styled(
        pad_width(&item.text, usize::from(item.rect.width)),
        style_for_display_item(
            item,
            theme,
            background,
            heading,
            numbered_row,
            blockquote_row,
        ),
    )]
}

fn adornment_spans(
    text: &str,
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Vec<Span<'static>> {
    if text.starts_with("▌ ") {
        return vec![
            Span::styled(
                "▌".to_string(),
                Style::default()
                    .fg(rgb(theme.border))
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " ".to_string(),
                base_document_style(theme, background, heading, numbered_row, blockquote_row),
            ),
        ];
    }
    if numbered_row && is_numbered_marker(text) {
        return vec![Span::styled(
            text.to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(background)
                .add_modifier(Modifier::BOLD),
        )];
    }
    if text == "[_]" || text == "[✗]" {
        return vec![
            Span::styled(
                "[".to_string(),
                Style::default().fg(rgb(theme.text_muted)).bg(background),
            ),
            Span::styled(
                text.chars().nth(1).unwrap_or('_').to_string(),
                Style::default()
                    .fg(rgb(if text == "[✗]" {
                        theme.error
                    } else {
                        theme.accent_highlight
                    }))
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "]".to_string(),
                Style::default().fg(rgb(theme.text_muted)).bg(background),
            ),
        ];
    }
    vec![Span::styled(
        text.to_string(),
        base_document_style(theme, background, heading, numbered_row, blockquote_row),
    )]
}

fn toc_row_spans(text: &str, theme: &Theme, background: Color) -> Vec<Span<'static>> {
    let Some((head, number)) = text.rsplit_once(' ') else {
        return vec![Span::styled(
            text.to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(background)
                .add_modifier(Modifier::UNDERLINED),
        )];
    };
    let Some((title, dots)) = head.rsplit_once(' ') else {
        return vec![Span::styled(
            text.to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(background)
                .add_modifier(Modifier::UNDERLINED),
        )];
    };
    vec![
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(background)
                .add_modifier(Modifier::UNDERLINED),
        ),
        Span::styled(
            format!(" {dots} "),
            Style::default()
                .fg(rgb(theme.text_secondary))
                .bg(background),
        ),
        Span::styled(
            number.to_string(),
            Style::default().fg(rgb(theme.link)).bg(background),
        ),
    ]
}

fn styled_fragments_for_item(
    item: &mdtui_render::DisplayItem,
    document: &mdtui_core::Document,
    visible_len: usize,
) -> Option<Vec<StyledFragment>> {
    let cursor = item.cursor?;
    if visible_len == 0 {
        return Some(Vec::new());
    }
    match cursor {
        Cursor::Text { block, offset } => {
            styled_fragments_for_block(document.blocks.get(block)?, offset, visible_len)
        }
        Cursor::ListItem {
            block,
            item,
            offset,
        } => match document.blocks.get(block)? {
            DocBlock::List(list) => {
                styled_fragments_for_list_item(list.items.get(item)?, offset, visible_len)
            }
            _ => None,
        },
        Cursor::TableCell {
            block,
            row,
            col,
            offset,
        } => match document.blocks.get(block)? {
            DocBlock::Table(table) => {
                let cell = table.rows.get(row)?.cells.get(col)?;
                styled_fragments_for_table_cell(cell, offset, visible_len)
            }
            _ => None,
        },
        Cursor::Checkbox { .. } => None,
    }
}

fn styled_fragments_for_block(
    block: &DocBlock,
    offset: usize,
    visible_len: usize,
) -> Option<Vec<StyledFragment>> {
    let fragments = styled_fragments_from_block(block);
    Some(slice_styled_fragments(&fragments, offset, visible_len))
}

fn styled_fragments_for_list_item(
    item: &mdtui_core::ListItem,
    offset: usize,
    visible_len: usize,
) -> Option<Vec<StyledFragment>> {
    let block = item.blocks.first()?;
    styled_fragments_for_block(block, offset, visible_len)
}

fn styled_fragments_for_table_cell(
    cell: &mdtui_core::TableCell,
    offset: usize,
    visible_len: usize,
) -> Option<Vec<StyledFragment>> {
    let block = cell.blocks.first()?;
    styled_fragments_for_block(block, offset, visible_len)
}

fn styled_fragments_from_block(block: &DocBlock) -> Vec<StyledFragment> {
    match block {
        DocBlock::Paragraph(inlines) => styled_fragments_from_inlines(inlines),
        DocBlock::Heading { level, inlines } => {
            let mut fragments = styled_fragments_from_inlines(inlines);
            if *level == 1 {
                for fragment in &mut fragments {
                    fragment.text = fragment.text.to_uppercase();
                }
            }
            fragments
        }
        DocBlock::BlockQuote(blocks) => {
            let mut fragments = Vec::new();
            for (index, block) in blocks.iter().enumerate() {
                fragments.extend(styled_fragments_from_block(block));
                if index + 1 < blocks.len() {
                    push_fragment(
                        &mut fragments,
                        "\n".to_string(),
                        InlinePaintStyle::default(),
                    );
                }
            }
            fragments
        }
        _ => vec![StyledFragment {
            text: block.rendered_text(),
            style: InlinePaintStyle::default(),
        }],
    }
}

fn styled_fragments_from_inlines(inlines: &[Inline]) -> Vec<StyledFragment> {
    let mut fragments = Vec::new();
    flatten_inlines(inlines, InlinePaintStyle::default(), &mut fragments);
    fragments
}

fn flatten_inlines(inlines: &[Inline], current: InlinePaintStyle, out: &mut Vec<StyledFragment>) {
    let mut current = current;
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_fragment(out, apply_super_sub(text, current), current),
            Inline::Emphasis(children) => {
                let mut next = current;
                next.italic = true;
                flatten_inlines(children, next, out);
            }
            Inline::Strong(children) => {
                let mut next = current;
                next.bold = true;
                flatten_inlines(children, next, out);
            }
            Inline::Strike(children) => {
                let mut next = current;
                next.strike = true;
                flatten_inlines(children, next, out);
            }
            Inline::InlineCode(text) => {
                let mut next = current;
                next.code = true;
                push_fragment(out, apply_super_sub(text, next), next);
            }
            Inline::Link { children, .. } => {
                let mut next = current;
                next.link = true;
                flatten_inlines(children, next, out);
            }
            Inline::Image { alt, .. } => push_fragment(out, apply_super_sub(alt, current), current),
            Inline::HtmlInline(html) => {
                let tag = html.trim();
                if tag.eq_ignore_ascii_case("<sup>") {
                    current.superscript = true;
                    continue;
                }
                if tag.eq_ignore_ascii_case("</sup>") {
                    current.superscript = false;
                    continue;
                }
                if tag.eq_ignore_ascii_case("<sub>") {
                    current.subscript = true;
                    continue;
                }
                if tag.eq_ignore_ascii_case("</sub>") {
                    current.subscript = false;
                    continue;
                }
                push_fragment(out, html.clone(), current);
            }
            Inline::SoftBreak | Inline::HardBreak => {
                push_fragment(out, "\n".to_string(), InlinePaintStyle::default())
            }
        }
    }
}

fn push_fragment(out: &mut Vec<StyledFragment>, text: String, style: InlinePaintStyle) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.style == style
    {
        last.text.push_str(&text);
    } else {
        out.push(StyledFragment { text, style });
    }
}

fn apply_super_sub(text: &str, style: InlinePaintStyle) -> String {
    if style.superscript {
        text.chars().map(to_superscript_char).collect()
    } else if style.subscript {
        text.chars().map(to_subscript_char).collect()
    } else {
        text.to_string()
    }
}

fn slice_styled_fragments(
    fragments: &[StyledFragment],
    start: usize,
    visible_len: usize,
) -> Vec<StyledFragment> {
    let end = start.saturating_add(visible_len);
    let mut offset = 0usize;
    let mut out = Vec::new();
    for fragment in fragments {
        let fragment_len = fragment.text.chars().count();
        let fragment_end = offset.saturating_add(fragment_len);
        if fragment_end <= start {
            offset = fragment_end;
            continue;
        }
        if offset >= end {
            break;
        }
        let local_start = start.saturating_sub(offset);
        let local_end = fragment_len.min(end.saturating_sub(offset));
        if local_end > local_start {
            out.push(StyledFragment {
                text: fragment
                    .text
                    .chars()
                    .skip(local_start)
                    .take(local_end - local_start)
                    .collect(),
                style: fragment.style,
            });
        }
        offset = fragment_end;
    }
    out
}

fn fragments_to_spans(
    fragments: &[StyledFragment],
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Vec<Span<'static>> {
    fragments
        .iter()
        .map(|fragment| {
            Span::styled(
                fragment.text.clone(),
                style_for_inline_fragment(
                    fragment.style,
                    theme,
                    background,
                    heading,
                    numbered_row,
                    blockquote_row,
                ),
            )
        })
        .collect()
}

fn base_document_style(
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Style {
    let mut style = Style::default()
        .fg(rgb(if numbered_row {
            theme.accent_highlight
        } else if blockquote_row {
            theme.text_secondary
        } else {
            theme.text_primary
        }))
        .bg(background);
    if heading {
        style = style
            .fg(rgb(theme.accent_highlight))
            .add_modifier(Modifier::BOLD);
    }
    style
}

fn style_for_display_item(
    item: &mdtui_render::DisplayItem,
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Style {
    if matches!(item.action, Some(DisplayAction::FollowLink { .. })) {
        return Style::default()
            .fg(rgb(theme.link))
            .bg(background)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    base_document_style(theme, background, heading, numbered_row, blockquote_row)
}

fn style_for_inline_fragment(
    fragment: InlinePaintStyle,
    theme: &Theme,
    background: Color,
    heading: bool,
    numbered_row: bool,
    blockquote_row: bool,
) -> Style {
    let mut style = base_document_style(theme, background, heading, numbered_row, blockquote_row);
    if fragment.link {
        style = style.fg(rgb(theme.link)).add_modifier(Modifier::UNDERLINED);
    }
    if fragment.code {
        style = style.bg(rgb(theme.panel_raised));
    }
    if fragment.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if fragment.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if fragment.strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    if fragment.superscript || fragment.subscript {
        style = style
            .fg(rgb(theme.accent_highlight))
            .add_modifier(Modifier::BOLD);
    }
    style
}

fn is_numbered_marker(text: &str) -> bool {
    let trimmed = text.trim();
    let Some((number, _)) = trimmed.split_once('.') else {
        return false;
    };
    number.chars().all(|ch| ch.is_ascii_digit())
}

fn pad_width(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

fn wrap_outline_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let current_len = current.chars().count();
        let word_len = word.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if current_len + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if current.is_empty() {
        lines.push(String::new());
    } else {
        lines.push(current);
    }
    lines
}

fn to_superscript_char(ch: char) -> char {
    match ch {
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        _ => ch,
    }
}

fn to_subscript_char(ch: char) -> char {
    match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'A' => 'ₐ',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => ch,
    }
}

fn selection_rects(state: &TuiState, rendered: &Rendered) -> Vec<mdtui_render::Rect> {
    let Some(selection) = state.app.editor.selection else {
        return Vec::new();
    };
    if selection.is_collapsed() {
        return Vec::new();
    }
    let (start, end) = ordered_cursors(selection.anchor, selection.head);
    rendered
        .display
        .items
        .iter()
        .filter_map(|item| selection_rect_for_item(item, start, end))
        .collect()
}

fn ordered_cursors(a: Cursor, b: Cursor) -> (Cursor, Cursor) {
    if compare_cursors(a, b).is_gt() {
        (b, a)
    } else {
        (a, b)
    }
}

fn compare_cursors(a: Cursor, b: Cursor) -> std::cmp::Ordering {
    cursor_sort_key(a).cmp(&cursor_sort_key(b))
}

fn cursor_sort_key(cursor: Cursor) -> (usize, u8, usize, usize, usize) {
    match cursor {
        Cursor::Text { block, offset } => (block, 0, 0, 0, offset),
        Cursor::ListItem {
            block,
            item,
            offset,
        } => (block, 1, item, 0, offset),
        Cursor::TableCell {
            block,
            row,
            col,
            offset,
        } => (block, 2, row, col, offset),
        Cursor::Checkbox { block, item } => (block, 1, item, 0, 0),
    }
}

fn selection_rect_for_item(
    item: &mdtui_render::DisplayItem,
    selection_start: Cursor,
    selection_end: Cursor,
) -> Option<mdtui_render::Rect> {
    let item_start = item.cursor?;
    if item.kind == DisplayKind::Adornment || item.kind == DisplayKind::HeadlinePlacement {
        return None;
    }
    let item_end = advance_cursor(item_start, usize::from(item.rect.width))?;
    if compare_cursors(selection_end, item_start).is_le()
        || compare_cursors(selection_start, item_end).is_ge()
    {
        return None;
    }
    let start = if compare_cursors(selection_start, item_start).is_gt() {
        cursor_delta(item_start, selection_start)?
    } else {
        0
    };
    let end = if compare_cursors(selection_end, item_end).is_lt() {
        cursor_delta(item_start, selection_end)?
    } else {
        usize::from(item.rect.width)
    };
    (end > start).then_some(mdtui_render::Rect {
        x: item.rect.x.saturating_add(start as u16),
        y: item.rect.y,
        width: (end - start) as u16,
        height: 1,
    })
}

fn advance_cursor(cursor: Cursor, width: usize) -> Option<Cursor> {
    Some(match cursor {
        Cursor::Text { block, offset } => Cursor::Text {
            block,
            offset: offset + width,
        },
        Cursor::ListItem {
            block,
            item,
            offset,
        } => Cursor::ListItem {
            block,
            item,
            offset: offset + width,
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
            offset: offset + width,
        },
        Cursor::Checkbox { .. } => return None,
    })
}

fn cursor_delta(base: Cursor, target: Cursor) -> Option<usize> {
    match (base, target) {
        (
            Cursor::Text {
                block: base_block,
                offset: base_offset,
            },
            Cursor::Text { block, offset },
        ) if block == base_block && offset >= base_offset => Some(offset - base_offset),
        (
            Cursor::ListItem {
                block: base_block,
                item: base_item,
                offset: base_offset,
            },
            Cursor::ListItem {
                block,
                item,
                offset,
            },
        ) if block == base_block && item == base_item && offset >= base_offset => {
            Some(offset - base_offset)
        }
        (
            Cursor::TableCell {
                block: base_block,
                row: base_row,
                col: base_col,
                offset: base_offset,
            },
            Cursor::TableCell {
                block,
                row,
                col,
                offset,
            },
        ) if block == base_block && row == base_row && col == base_col && offset >= base_offset => {
            Some(offset - base_offset)
        }
        _ => None,
    }
}

fn apply_selection_highlight(
    buf: &mut Buffer,
    area: Rect,
    scroll: u16,
    rects: &[mdtui_render::Rect],
    theme: &Theme,
) {
    for rect in rects {
        let screen_y = area
            .y
            .saturating_add(1)
            .saturating_add(rect.y.saturating_sub(scroll));
        if screen_y <= area.y || screen_y >= area.y.saturating_add(area.height.saturating_sub(1)) {
            continue;
        }
        let screen_x = area.x.saturating_add(1).saturating_add(rect.x);
        let right = area.x.saturating_add(area.width.saturating_sub(1));
        if screen_x >= right {
            continue;
        }
        let width = rect.width.min(right.saturating_sub(screen_x));
        if width == 0 {
            continue;
        }
        buf.set_style(
            Rect {
                x: screen_x,
                y: screen_y,
                width,
                height: 1,
            },
            Style::default().bg(rgb(theme.active_row)),
        );
    }
}

fn is_heading_rule(line: &str) -> bool {
    !line.is_empty()
        && line
            .chars()
            .all(|ch| matches!(ch, '═' | '─' | '🬂' | '🭶' | '‾'))
}

fn style_code_header(line: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(line.to_string(), code_border(theme)))
}

fn style_code_toolbar(line: &str, theme: &Theme) -> Line<'static> {
    let Some(copy_start) = line.rfind("copy") else {
        return Line::from(Span::styled(line.to_string(), code_border(theme)));
    };
    let before = &line[..copy_start];
    let after = &line[copy_start + "copy".len()..];
    Line::from(vec![
        Span::styled(before.to_string(), code_border(theme)),
        Span::styled(
            "copy".to_string(),
            Style::default()
                .fg(rgb(theme.link))
                .bg(rgb(theme.panel_bg))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(after.to_string(), code_border(theme)),
    ])
}

fn style_code_body(
    line: &str,
    theme: &Theme,
    code_source: Option<(String, usize)>,
) -> Line<'static> {
    let content = line.trim_start_matches('│');
    let Some((number, rest)) = content.split_once("│ ") else {
        return Line::from(Span::styled(line.to_string(), code_text(theme)));
    };
    let rest = rest.strip_suffix(" │").unwrap_or(rest).to_string();
    let mut spans = vec![
        Span::styled("│".to_string(), code_gutter_border(theme)),
        Span::styled(number.to_string(), code_gutter(theme)),
        Span::styled("│ ".to_string(), code_gutter_divider(theme)),
    ];
    if let Some((source, scroll)) = code_source {
        spans.extend(highlight_code_window(
            &source,
            scroll,
            rest.chars().count(),
            theme,
        ));
    } else {
        spans.extend(highlight_code(&rest, theme));
    }
    spans.push(Span::styled(" │".to_string(), code_border(theme)));
    Line::from(spans)
}

fn code_source_for_row(
    index: usize,
    rendered: &Rendered,
    document: &mdtui_core::Document,
    scrolls: &[(usize, usize)],
) -> Option<(String, usize)> {
    let item = rendered.display.items.iter().find(|item| {
        item.rect.y as usize == index
            && item.kind == DisplayKind::TextRun
            && item.rect.x == 6
            && matches!(
                item.cursor,
                Some(Cursor::Text {
                    block: _,
                    offset: _
                })
            )
    })?;
    let Cursor::Text { block, offset } = item.cursor? else {
        return None;
    };
    let DocBlock::CodeBlock { text, .. } = document.blocks.get(block)? else {
        return None;
    };
    let source = code_line_for_offset(text, offset)?.to_string();
    Some((
        source,
        scrolls
            .iter()
            .find(|(scroll_block, _)| *scroll_block == block)
            .map(|(_, scroll)| *scroll)
            .unwrap_or(0),
    ))
}

fn code_line_for_offset(text: &str, offset: usize) -> Option<&str> {
    let mut current = 0usize;
    for line in text.lines() {
        if current == offset {
            return Some(line);
        }
        current += line.chars().count() + 1;
    }
    None
}

fn highlight_code_window(
    source: &str,
    scroll: usize,
    visible_width: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let highlighted = highlight_code(source, theme);
    slice_highlighted_spans(highlighted, scroll, visible_width, code_text(theme))
}

fn slice_highlighted_spans(
    spans: Vec<Span<'static>>,
    start: usize,
    visible_width: usize,
    fill_style: Style,
) -> Vec<Span<'static>> {
    let end = start.saturating_add(visible_width);
    let mut offset = 0usize;
    let mut painted = 0usize;
    let mut out = Vec::new();
    for span in spans {
        let span_text = span.content.to_string();
        let span_len = span_text.chars().count();
        let span_end = offset.saturating_add(span_len);
        if span_end <= start {
            offset = span_end;
            continue;
        }
        if offset >= end {
            break;
        }
        let local_start = start.saturating_sub(offset);
        let local_end = span_len.min(end.saturating_sub(offset));
        if local_end > local_start {
            let text = span_text
                .chars()
                .skip(local_start)
                .take(local_end - local_start)
                .collect::<String>();
            painted += text.chars().count();
            out.push(Span::styled(text, span.style));
        }
        offset = span_end;
    }
    if painted < visible_width {
        out.push(Span::styled(
            " ".repeat(visible_width - painted),
            fill_style,
        ));
    }
    out
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
    if !state.kitty_graphics || HEADLINE_DEBUG_SLAB {
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
    let viewport_end = scroll.saturating_add(viewport_rows);
    let mut out = Vec::new();
    for item in &rendered.display.items {
        if item.kind != DisplayKind::HeadlinePlacement {
            continue;
        }
        if item
            .cursor
            .is_some_and(|cursor| cursor_block(cursor) == cursor_block(state.app.editor.cursor))
        {
            continue;
        }
        let rows = item.rect.height.max(2);
        if item.rect.y < scroll || item.rect.y.saturating_add(rows) > viewport_end {
            continue;
        }
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let cols = item.rect.width.max(8);
        let level = headline_level(state, item);
        let key = format!("{HEADLINE_RASTER_VERSION}:{level}:{text}:{cols}:{rows}");
        let png = if let Some(bytes) = state.headline_png_cache.get(&key) {
            bytes.clone()
        } else {
            request_headline_raster(state, key.clone(), text.to_string(), level, cols, rows);
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
            kitty_png_apc(&png, cols, rows)
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

fn request_headline_raster(
    state: &mut TuiState,
    key: String,
    text: String,
    level: u8,
    cols: u16,
    rows: u16,
) {
    if state.headline_png_cache.contains_key(&key) || state.pending_headline_jobs.contains(&key) {
        return;
    }
    state.pending_headline_jobs.insert(key.clone());
    let tx = state.headline_raster_tx.clone();
    thread::spawn(move || {
        let _ = tx.send((key, headline_png(&text, level, cols, rows)));
    });
}

fn headline_png(text: &str, level: u8, cols: u16, rows: u16) -> io::Result<Vec<u8>> {
    let cell_w = 16u32;
    let cell_h = 32u32;
    let width = u32::from(cols.max(8)) * cell_w;
    let height = u32::from(rows).max(2) * cell_h;
    let mut img = RgbaImage::from_pixel(width, height, Rgba([15, 12, 8, 255]));
    if HEADLINE_DEBUG_SLAB {
        draw_headline_debug_slab(&mut img, cols, rows);
    } else if draw_headline_font(&mut img, text, level).is_err() {
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

fn draw_headline_font(image: &mut RgbaImage, text: &str, level: u8) -> io::Result<()> {
    if text.is_empty() {
        return Err(io::Error::other("empty headline"));
    }
    let font = load_headline_font()?;
    let (layout, bounds) = fit_headline_layout(&font, text, level, image.width(), image.height())?;
    let offset_x = (-bounds.min_x).round() as i32;
    let optical_bias = match level {
        1 => (image.height() as f32 * 0.14).round() as i32,
        2 => (image.height() as f32 * 0.10).round() as i32,
        _ => (image.height() as f32 * 0.10).round() as i32,
    };
    let offset_y = ((image.height() as f32 - bounds.height()) / 2.0 - bounds.min_y).round() as i32
        + optical_bias;
    let shadow = Rgba([44, 28, 12, 96]);
    let top = Rgba([255, 220, 160, 255]);
    let bottom = Rgba([217, 138, 82, 255]);
    for glyph in layout.glyphs() {
        let (metrics, bitmap) = font.rasterize_config(glyph.key);
        paint_alpha_bitmap(
            image,
            glyph.x as i32 + offset_x + 1,
            glyph.y as i32 + offset_y + 1,
            metrics.width,
            metrics.height,
            &bitmap,
            shadow,
        );
        paint_alpha_bitmap_gradient(
            image,
            glyph.x as i32 + offset_x,
            glyph.y as i32 + offset_y,
            metrics.width,
            metrics.height,
            &bitmap,
            (top, bottom),
        );
    }
    Ok(())
}

struct HeadlineBounds {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl HeadlineBounds {
    fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

fn fit_headline_layout(
    font: &Font,
    text: &str,
    level: u8,
    image_width: u32,
    image_height: u32,
) -> io::Result<(FontLayout, HeadlineBounds)> {
    let target_height = match level {
        1 => image_height.saturating_sub(4) as f32,
        2 => (image_height as f32 * 0.75).round(),
        _ => (image_height as f32 * 0.75).round(),
    };
    let mut size = match level {
        1 => image_height as f32 * 1.15,
        2 => image_height as f32 * 0.92,
        _ => image_height as f32 * 0.92,
    };
    let mut best: Option<(FontLayout, HeadlineBounds)> = None;
    for _ in 0..18 {
        let mut layout = FontLayout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            x: 0.0,
            y: 0.0,
            max_width: Some(image_width as f32),
            max_height: Some(image_height as f32),
            ..LayoutSettings::default()
        });
        layout.append(&[font], &TextStyle::new(text, size, 0));
        let bounds = headline_bounds(font, &layout)?;
        let fits_height = bounds.height() <= target_height;
        let fits_width = bounds.width() <= image_width as f32;
        best = Some((layout, bounds));
        if fits_height && fits_width {
            break;
        }
        size *= 0.92;
    }
    best.ok_or_else(|| io::Error::other("unable to fit headline layout"))
}

fn headline_bounds(font: &Font, layout: &FontLayout) -> io::Result<HeadlineBounds> {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for glyph in layout.glyphs() {
        let (metrics, _) = font.rasterize_config(glyph.key);
        if metrics.width == 0 || metrics.height == 0 {
            continue;
        }
        min_x = min_x.min(glyph.x);
        max_x = max_x.max(glyph.x + metrics.width as f32);
        min_y = min_y.min(glyph.y);
        max_y = max_y.max(glyph.y + metrics.height as f32);
    }
    if min_x == f32::MAX || max_x == f32::MIN || min_y == f32::MAX || max_y == f32::MIN {
        return Err(io::Error::other("no visible headline glyph bounds"));
    }
    Ok(HeadlineBounds {
        min_x,
        max_x,
        min_y,
        max_y,
    })
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

fn draw_headline_debug_slab(image: &mut RgbaImage, cols: u16, rows: u16) {
    let glyph = font8x8::BASIC_FONTS
        .get('▒')
        .or_else(|| font8x8::BASIC_FONTS.get('#'))
        .unwrap_or([0b0101_0101; 8]);
    let fg = Rgba([198, 176, 140, 255]);
    let cell_w = image.width() / u32::from(cols.max(1));
    let cell_h = image.height() / u32::from(rows.max(1));
    let scale = (cell_w / 8).max(1).min((cell_h / 8).max(1));
    let glyph_w = 8 * scale;
    let glyph_h = 8 * scale;
    for row in 0..rows.max(1) {
        for col in 0..cols.max(1) {
            let x = u32::from(col) * cell_w + cell_w.saturating_sub(glyph_w) / 2;
            let y = u32::from(row) * cell_h + cell_h.saturating_sub(glyph_h) / 2;
            draw_glyph(image, glyph, x, y, scale, fg);
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

fn headline_level(state: &TuiState, item: &mdtui_render::DisplayItem) -> u8 {
    let Some(cursor) = item.cursor else {
        return 1;
    };
    match state.app.editor.document.blocks.get(cursor_block(cursor)) {
        Some(DocBlock::Heading { level, .. }) => *level,
        _ => 1,
    }
}

fn materialize_active_headline_fallback(state: &TuiState, rendered: &mut Rendered) {
    let active_block = cursor_block(state.app.editor.cursor);
    let mut items = Vec::with_capacity(rendered.display.items.len() + 2);
    for item in &rendered.display.items {
        if item.kind != DisplayKind::HeadlinePlacement {
            items.push(item.clone());
            continue;
        }
        let Some(cursor) = item.cursor else {
            items.push(item.clone());
            continue;
        };
        if cursor_block(cursor) != active_block {
            items.push(item.clone());
            continue;
        }
        let Some(DocBlock::Heading { .. }) = state.app.editor.document.blocks.get(active_block)
        else {
            items.push(item.clone());
            continue;
        };
        let text = item.text.trim();
        let display = text.to_string();
        let text_y = usize::from(item.rect.y);
        let rule_y = usize::from(item.rect.y.saturating_add(1));
        if let Some(line) = rendered.lines.get_mut(text_y) {
            *line = String::new();
        }
        if let Some(line) = rendered.lines.get_mut(rule_y) {
            *line = display.clone();
        }
        items.push(mdtui_render::DisplayItem {
            kind: DisplayKind::TextRun,
            rect: mdtui_render::Rect {
                x: 0,
                y: item.rect.y.saturating_add(1),
                width: display.chars().count() as u16,
                height: 1,
            },
            cursor: Some(Cursor::Text {
                block: active_block,
                offset: 0,
            }),
            action: None,
            text: display,
        });
    }
    rendered.display.items = items;
}

fn clamp_scroll(offset: u16, content: usize, viewport: usize) -> u16 {
    if viewport == 0 {
        return 0;
    }
    offset.min(content.saturating_sub(viewport) as u16)
}

fn normalize_headline_cursor(state: &TuiState, cursor: Cursor) -> Cursor {
    match cursor {
        Cursor::Text { block, offset } => match state.app.editor.document.blocks.get(block) {
            Some(DocBlock::Heading { inlines, .. }) if state.kitty_graphics => Cursor::Text {
                block,
                offset: offset.min(mdtui_core::inline_text(inlines).chars().count()),
            },
            _ => cursor,
        },
        _ => cursor,
    }
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

fn mix_hex(foreground: &str, background: &str, percent_background: u16) -> String {
    let percent = percent_background.min(100) as u32;
    let fg = foreground.trim_start_matches('#');
    let bg = background.trim_start_matches('#');
    let mut mixed = String::from("#");
    for index in [0usize, 2, 4] {
        let fg_channel = u8::from_str_radix(&fg[index..index + 2], 16).unwrap_or(0) as u32;
        let bg_channel = u8::from_str_radix(&bg[index..index + 2], 16).unwrap_or(0) as u32;
        let value = (fg_channel * (100 - percent) + bg_channel * percent) / 100;
        mixed.push_str(&format!("{value:02x}"));
    }
    mixed
}

fn code_border(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.border))
        .bg(rgb(theme.panel_bg))
}

fn code_gutter_border(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.border))
        .bg(rgb(theme.panel_bg))
}

fn code_gutter_divider(theme: &Theme) -> Style {
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
        .bg(rgb(theme.code_bg))
}

fn code_keyword(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.accent_primary))
        .bg(rgb(theme.code_bg))
        .add_modifier(Modifier::BOLD)
}

fn code_string(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.success))
        .bg(rgb(theme.code_bg))
}

fn code_call(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.accent_highlight))
        .bg(rgb(theme.code_bg))
        .add_modifier(Modifier::BOLD)
}

fn code_type(theme: &Theme) -> Style {
    Style::default().fg(rgb(theme.link)).bg(rgb(theme.code_bg))
}

fn code_punct(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_secondary))
        .bg(rgb(theme.code_bg))
}

fn code_comment(theme: &Theme) -> Style {
    Style::default()
        .fg(rgb(theme.text_muted))
        .bg(rgb(theme.code_bg))
        .add_modifier(Modifier::ITALIC)
}

fn code_number(theme: &Theme) -> Style {
    Style::default().fg(rgb(theme.link)).bg(rgb(theme.code_bg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::backend::TestBackend;

    #[test]
    fn move_visual_skips_code_block_chrome_rows() {
        let mut state = TuiState::new(
            App::from_markdown(
                "x.md",
                "before\n\n```python\ndef greet():\n    return 1\n```\n\nafter",
            ),
            None,
        );
        state.app.render_options = RenderOptions {
            width: 80,
            heading_width: 80,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        state.last_rendered = Some(rendered);

        move_visual(&mut state, 1, false);

        assert_eq!(
            state.app.editor.cursor,
            Cursor::Text {
                block: 1,
                offset: 0
            }
        );
    }

    #[test]
    fn wheel_over_copy_button_scrolls_without_triggering_copy() {
        let mut state = TuiState::new(
            App::from_markdown("x.md", "```python\ndef greet():\n    return 1\n```"),
            None,
        );
        state.last_doc_area = Rect {
            x: 1,
            y: 1,
            width: 80,
            height: 12,
        };
        state.scroll = 2;
        state.app.render_options = RenderOptions {
            width: 80,
            heading_width: 80,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        let copy_item = rendered
            .display
            .items
            .iter()
            .find(|item| item.action.is_some())
            .cloned()
            .expect("copy action item");
        state.last_rendered = Some(rendered);
        let column = state.last_doc_area.x + 1 + copy_item.rect.x;
        let row = state.last_doc_area.y + 1 + copy_item.rect.y;

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert_eq!(state.scroll, 3);
        assert_ne!(state.message, "code block copied");
        assert!(state.drag_anchor.is_none());
    }

    #[test]
    fn dragging_code_footer_thumb_updates_horizontal_scroll() {
        let mut state = TuiState::new(
            App::from_markdown(
                "x.md",
                "```python\nabcdefghijklmnopqrstuvwxyz0123456789\n```",
            ),
            None,
        );
        state.last_doc_area = Rect {
            x: 1,
            y: 1,
            width: 80,
            height: 12,
        };
        state.app.render_options = RenderOptions {
            width: 36,
            heading_width: 36,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        let thumb_item = rendered
            .display
            .items
            .iter()
            .find(|item| matches!(item.action, Some(DisplayAction::ScrollCodeBlock { .. })))
            .cloned()
            .expect("scroll thumb item");
        state.last_rendered = Some(rendered);
        let down_column = state.last_doc_area.x + 1 + thumb_item.rect.x;
        let row = state.last_doc_area.y + 1 + thumb_item.rect.y;

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: down_column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: down_column + 6,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert!(
            state
                .app
                .render_options
                .code_horizontal_scrolls
                .iter()
                .find(|(block, _)| *block == 0)
                .is_some_and(|(_, scroll)| *scroll > 0)
        );
    }

    #[test]
    fn selection_rects_are_character_precise() {
        let mut state = TuiState::new(App::from_markdown("x.md", "alpha beta"), None);
        state.app.editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 1,
            },
            Cursor::Text {
                block: 0,
                offset: 4,
            },
        );
        state.app.render_options = RenderOptions {
            width: 80,
            heading_width: 80,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);

        assert_eq!(
            selection_rects(&state, &rendered),
            vec![mdtui_render::Rect {
                x: 1,
                y: 0,
                width: 3,
                height: 1,
            }]
        );
    }

    #[test]
    fn selection_highlight_only_tints_selected_cells() {
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 4,
        };
        let mut buf = Buffer::empty(area);
        buf.set_style(area, Style::default().bg(rgb(theme.panel_bg)));

        apply_selection_highlight(
            &mut buf,
            area,
            0,
            &[mdtui_render::Rect {
                x: 1,
                y: 0,
                width: 3,
                height: 1,
            }],
            &theme,
        );

        assert_eq!(
            buf.cell((1, 1)).expect("left gutter cell").bg,
            rgb(theme.panel_bg)
        );
        assert_eq!(
            buf.cell((2, 1)).expect("selection start cell").bg,
            rgb(theme.active_row)
        );
        assert_eq!(
            buf.cell((4, 1)).expect("selection end cell").bg,
            rgb(theme.active_row)
        );
        assert_eq!(
            buf.cell((5, 1)).expect("right gutter cell").bg,
            rgb(theme.panel_bg)
        );
    }

    #[test]
    fn dragging_wrap_slider_updates_wrap_width() {
        let mut state = TuiState::new(App::from_markdown("x.md", "alpha beta"), None);
        state.last_status_area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 1,
        };
        state.app.render_options = RenderOptions {
            width: 80,
            heading_width: 80,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        let theme = Theme::dark_amber();
        let mut terminal = Terminal::new(TestBackend::new(120, 1)).expect("test terminal");
        terminal
            .draw(|frame| draw_status(frame, state.last_status_area, &mut state, &rendered, &theme))
            .expect("draw status");
        let slider = state.wrap_slider_track.expect("wrap slider track");
        let row = state.last_status_area.y;
        let start_column = state.last_status_area.x + slider.start;
        let end_column = start_column + slider.slots.saturating_sub(1);

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: start_column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: end_column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: end_column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert_eq!(state.wrap_width, 120);
        assert!(state.wrap_slider_drag.is_none());
    }

    #[test]
    fn document_scrollbar_renders_on_panel_border() {
        let mut state = TuiState::new(
            App::from_markdown("x.md", "one\ntwo\nthree\nfour\nfive\nsix"),
            None,
        );
        state.scroll = 1;
        state.app.render_options = RenderOptions {
            width: 20,
            heading_width: 20,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 6,
        };
        let mut terminal = Terminal::new(TestBackend::new(20, 6)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();

        assert!(matches!(
            buffer
                .cell((19, 2))
                .expect("border scrollbar cell")
                .symbol(),
            "│" | "█"
        ));
        assert_eq!(
            buffer.cell((18, 2)).expect("content edge cell").symbol(),
            " "
        );
    }

    #[test]
    fn inline_marks_render_with_terminal_modifiers() {
        let state = TuiState::new(
            App::from_markdown("x.md", "**bold** *ital* ~~gone~~ <sup>2</sup> <sub>2</sub>"),
            None,
        );
        let rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 4,
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 4)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();
        let line = &rendered.lines[0];
        let bold_x = line.find("bold").expect("bold segment") as u16;
        let ital_x = line.find("ital").expect("italic segment") as u16;
        let gone_x = line.find("gone").expect("strike segment") as u16;

        assert!(
            buffer
                .cell((1 + bold_x, 1))
                .expect("bold cell")
                .modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            buffer
                .cell((1 + ital_x, 1))
                .expect("italic cell")
                .modifier
                .contains(Modifier::ITALIC)
        );
        assert!(
            buffer
                .cell((1 + gone_x, 1))
                .expect("strike cell")
                .modifier
                .contains(Modifier::CROSSED_OUT)
        );
        assert!(
            (1..area.width.saturating_sub(1))
                .filter_map(|x| buffer.cell((x, 1)))
                .any(|cell| cell.symbol() == "²")
        );
        assert!(
            (1..area.width.saturating_sub(1))
                .filter_map(|x| buffer.cell((x, 1)))
                .any(|cell| cell.symbol() == "₂")
        );
    }

    #[test]
    fn horizontal_code_scroll_keeps_keyword_highlight() {
        let theme = Theme::dark_amber();
        let spans = highlight_code_window("return value", 2, 10, &theme);
        let first = spans.first().expect("highlighted keyword slice");
        assert_eq!(first.content.to_string(), "turn");
        assert_eq!(first.style.fg, Some(rgb(theme.accent_primary)));
    }

    #[test]
    fn clicking_toc_row_follows_heading_link() {
        let mut state = TuiState::new(
            App::from_markdown(
                "x.md",
                "## Table of contents\n\n1. [Project identity](#project-identity)\n\n# Project identity",
            ),
            None,
        );
        state.last_doc_area = Rect {
            x: 1,
            y: 1,
            width: 60,
            height: 12,
        };
        state.app.render_options = RenderOptions {
            width: 28,
            heading_width: 28,
            kitty_graphics: false,
            show_status: false,
            ..RenderOptions::default()
        };
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        let toc_item = rendered
            .display
            .items
            .iter()
            .find(|item| matches!(item.action, Some(DisplayAction::FollowLink { block: 2 })))
            .cloned()
            .expect("toc row");
        state.last_rendered = Some(rendered);
        let column = state.last_doc_area.x + 1 + toc_item.rect.x;
        let row = state.last_doc_area.y + 1 + toc_item.rect.y;

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert_eq!(
            state.app.editor.cursor,
            Cursor::Text {
                block: 2,
                offset: 0
            }
        );
    }

    #[test]
    fn explorer_and_outline_sidebar_are_wider() {
        let mut state = TuiState::new(App::from_markdown("x.md", "body"), None);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
        terminal
            .draw(|frame| draw(frame, &mut state))
            .expect("draw ui");

        assert_eq!(state.last_explorer_area.width, 43);
        assert_eq!(state.last_outline_area.width, 43);
    }

    #[test]
    fn dragging_panel_scrollbar_updates_scroll() {
        let mut state = TuiState::new(
            App::from_markdown(
                "x.md",
                "one\n\ntwo\n\nthree\n\nfour\n\nfive\n\nsix\n\nseven\n\neight",
            ),
            None,
        );
        state.last_doc_area = Rect {
            x: 1,
            y: 1,
            width: 40,
            height: 6,
        };
        state.last_rendered = Some(render_document(
            &state.app.editor.document,
            RenderOptions {
                width: 24,
                heading_width: 24,
                kitty_graphics: false,
                show_status: false,
                ..RenderOptions::default()
            },
        ));
        let column = state.last_doc_area.x + state.last_doc_area.width.saturating_sub(1);
        let start_row = state.last_doc_area.y + 2;
        let end_row = state.last_doc_area.y + state.last_doc_area.height.saturating_sub(2);

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: start_row,
                modifiers: KeyModifiers::empty(),
            },
        );
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column,
                row: end_row,
                modifiers: KeyModifiers::empty(),
            },
        );
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column,
                row: end_row,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert!(state.scroll > 0);
        assert!(state.panel_scroll_drag.is_none());
    }

    #[test]
    fn explorer_mode_controls_live_on_top_border() {
        let mut state = TuiState::new(App::from_markdown("x.md", "body"), None);
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 43,
            height: 10,
        };
        let (lines, hits) = explorer_lines(
            None,
            &state.app.file_name,
            state.explorer_mode,
            &state.collapsed_dirs,
            &theme,
        );
        state.explorer_hits = hits;
        state.last_explorer_area = area;
        let mut terminal = Terminal::new(TestBackend::new(43, 10)).expect("test terminal");
        terminal
            .draw(|frame| draw_explorer(frame, area, &mut state, &lines, 0, &theme))
            .expect("draw explorer");
        let flat = state
            .explorer_mode_hits
            .iter()
            .find(|hit| matches!(hit.action, ExplorerAction::ToggleMode(ExplorerMode::Flat)))
            .expect("flat hit");
        let column = area.x + flat.start;

        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: area.y,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert!(matches!(state.explorer_mode, ExplorerMode::Flat));
    }

    #[test]
    fn ordered_list_rows_use_blue_numbers_and_accent_text() {
        let state = TuiState::new(App::from_markdown("x.md", "1. alpha"), None);
        let rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 4,
        };
        let mut terminal = Terminal::new(TestBackend::new(30, 4)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer.cell((1, 1)).expect("number cell").fg,
            rgb(theme.link)
        );
        assert_eq!(
            buffer.cell((4, 1)).expect("text cell").fg,
            rgb(theme.accent_highlight)
        );
    }

    #[test]
    fn code_chrome_uses_page_background_and_light_border() {
        let state = TuiState::new(
            App::from_markdown("x.md", "```rust\nfn main() {}\n```"),
            None,
        );
        let mut rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        materialize_active_headline_fallback(&state, &mut rendered);
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 8,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer.cell((1, 1)).expect("top border cell").bg,
            rgb(theme.panel_bg)
        );
        assert_eq!(
            buffer.cell((1, 1)).expect("top border cell").fg,
            rgb(theme.border)
        );
    }

    #[test]
    fn blockquote_text_uses_muted_color() {
        let state = TuiState::new(App::from_markdown("x.md", "> quoted"), None);
        let rendered =
            render_document(&state.app.editor.document, state.app.render_options.clone());
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 24,
            height: 4,
        };
        let mut terminal = Terminal::new(TestBackend::new(24, 4)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer.cell((3, 1)).expect("quote text").fg,
            rgb(theme.text_secondary)
        );
    }

    #[test]
    fn toc_rows_keep_book_style_leaders_and_right_number() {
        let state = TuiState::new(
            App::from_markdown(
                "x.md",
                "1. [Project identity](#project-identity)\n\n# Project identity",
            ),
            None,
        );
        let rendered = render_document(
            &state.app.editor.document,
            RenderOptions {
                width: 28,
                heading_width: 28,
                kitty_graphics: false,
                show_status: false,
                ..RenderOptions::default()
            },
        );
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 32,
            height: 6,
        };
        let mut terminal = Terminal::new(TestBackend::new(32, 6)).expect("test terminal");
        terminal
            .draw(|frame| draw_document(frame, area, &state, &rendered, &theme))
            .expect("draw document");
        let buffer = terminal.backend().buffer();

        assert_eq!(buffer.cell((19, 1)).expect("dot leader").symbol(), ".");
        assert_eq!(buffer.cell((28, 1)).expect("page number").symbol(), "1");
        assert_eq!(
            buffer.cell((28, 1)).expect("page number").fg,
            rgb(theme.link)
        );
    }

    #[test]
    fn style_popup_anchors_above_selection() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 30,
        };
        let doc_area = Rect {
            x: 10,
            y: 6,
            width: 60,
            height: 18,
        };
        let popup = anchored_style_popup(
            area,
            doc_area,
            0,
            &[mdtui_render::Rect {
                x: 12,
                y: 10,
                width: 6,
                height: 1,
            }],
            40,
            5,
        );

        assert!(popup.y < doc_area.y + 1 + 10);
    }

    #[test]
    fn style_popup_active_chip_uses_inverted_colors() {
        let mut state = TuiState::new(App::from_markdown("x.md", "alpha"), None);
        state.app.editor.select_all();
        let theme = Theme::dark_amber();
        let area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 12,
        };
        let doc_area = Rect {
            x: 1,
            y: 1,
            width: 40,
            height: 8,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("test terminal");
        terminal
            .draw(|frame| {
                draw_style_popover(
                    frame,
                    area,
                    doc_area,
                    0,
                    &[mdtui_render::Rect {
                        x: 0,
                        y: 2,
                        width: 5,
                        height: 1,
                    }],
                    &mut state,
                    &theme,
                );
            })
            .expect("draw style popup");
        let popup = anchored_style_popup(
            area,
            doc_area,
            0,
            &[mdtui_render::Rect {
                x: 0,
                y: 2,
                width: 5,
                height: 1,
            }],
            popup_line_width(&style_popup_cells()),
            3,
        );
        let buffer = terminal.backend().buffer();
        let cell = buffer
            .cell((popup.x + 2, popup.y + 1))
            .expect("active bold cell");
        assert_eq!(cell.fg, rgb(theme.panel_bg));
        assert_eq!(cell.bg, rgb(theme.accent_highlight));
    }

    #[test]
    fn popup_code_action_turns_full_selection_into_code_block() {
        let mut state = TuiState::new(App::from_markdown("x.md", "alpha beta"), None);
        state.app.editor.select_all();

        apply_style_popup_action(&mut state, StylePopupAction::Code);

        assert!(matches!(
            state.app.editor.document.blocks[0],
            DocBlock::CodeBlock { .. }
        ));
    }

    #[test]
    fn ctrl_wrap_shortcuts_do_not_change_wrap_width() {
        let mut state = TuiState::new(App::from_markdown("x.md", "alpha"), None);
        let original = state.wrap_width;

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::CONTROL),
        );
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('='), KeyModifiers::CONTROL),
        );

        assert_eq!(state.wrap_width, original);
    }
}
