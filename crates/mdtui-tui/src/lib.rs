mod tabbar;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use image::ImageReader;
use mdtui_core::{ComponentKind, Document};
use mdtui_markdown::{export_gfm, import_gfm};
use mdtui_render::{
    CellBg, CellStyle, PreviewRequest, RenderLine, RenderOptions, RenderedDocument,
    RenderedGraphic, RenderedGraphicKind, Rgb, SourceCellSpan, Theme, Token, render_document,
    render_document_window_with_cache, warm_preview,
};
use mdtui_terminal::{
    ClipboardMethod, GraphicsCacheKey, KittyGraphicsCache, TerminalCapabilities, TuiTerminal,
    cached_external_preview_raster, copy_to_clipboard_or_osc52, external_preview_cache_generation,
    graphics_theme_hash, heading_height_px, heading_width_px, image_preview_height_px,
    image_preview_width_px, kitty_create_virtual_placement, kitty_transmit_rgba,
    queue_external_preview_render, raster_heading_rgba, raster_local_image_rgba,
    raster_preview_rgba,
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap},
};
use ratatui_image::{Image, Resize, picker::Picker};
use serde::{Deserialize, Serialize};
use tui_checkbox::Checkbox;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const SESSION_FILE: &str = ".mdtui-session.toml";
const DOCUMENT_X_PADDING: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Help,
    Command,
    Search,
    LinkPrompt,
    CodeLanguagePrompt,
    HeadingPrompt,
    FootnoteLabelPrompt,
    ImagePrompt,
    TableActions,
    StylePalette,
    InsertMenu,
    ConfirmQuit,
    ConfirmClose,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CodeFocus {
    #[default]
    None,
    Language,
    Body,
    CopyButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableSelection {
    Row(usize),
    Column(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineStyleAction {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertBlockAction {
    Paragraph,
    Heading,
    CodeBlock,
    Quote,
    Alert,
    List,
    Table,
    Image,
    LinkReference,
    Math,
    Diagram,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub title: String,
    pub path: Option<PathBuf>,
    pub document: Document,
    pub scroll_y: usize,
}

#[derive(Clone, Debug)]
pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub mode: Mode,
    pub command_input: String,
    pub command_selection: usize,
    pub search_input: String,
    pub link_input: String,
    pub code_language_input: String,
    pub heading_input: String,
    pub footnote_label_input: String,
    pub image_input: String,
    pub message: String,
    pub theme: Theme,
    pub last_frame_ms: f64,
    pub last_parse_ms: f64,
    pub should_quit: bool,
    pub clipboard: Option<String>,
    pub code_focus: CodeFocus,
    pub table_selection: Option<TableSelection>,
    pub graphics: Vec<RenderedGraphic>,
    pub pending_previews: Vec<PreviewRequest>,
    pub kitty_graphics: bool,
    pub last_rendered: Option<RenderedDocument>,
    pub last_content_area: Option<Rect>,
    pub last_document_area: Option<Rect>,
    pub last_tab_area: Option<Rect>,
    pub last_screen_area: Option<Rect>,
    pub pending_close: Option<usize>,
    pub mouse_selection_anchor: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub active: usize,
    pub tabs: Vec<SessionTab>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTab {
    pub path: PathBuf,
    pub cursor_source_byte: usize,
    pub scroll_y: usize,
    pub pinned: bool,
}

impl App {
    pub fn open_initial(path: Option<PathBuf>) -> Result<Self> {
        let tab = match path {
            Some(path) => load_tab(&path)?,
            None => return Ok(Self::restore_or_empty()),
        };
        Ok(Self {
            tabs: vec![tab],
            active: 0,
            mode: Mode::Edit,
            command_input: String::new(),
            command_selection: 0,
            search_input: String::new(),
            link_input: String::new(),
            code_language_input: String::new(),
            heading_input: String::new(),
            footnote_label_input: String::new(),
            image_input: String::new(),
            message: "direct edit  Ctrl-S save  Ctrl-Q quit  ? help".to_string(),
            theme: Theme::ghostty_default_dark(),
            last_frame_ms: 0.0,
            last_parse_ms: 0.0,
            should_quit: false,
            clipboard: None,
            code_focus: CodeFocus::None,
            table_selection: None,
            graphics: Vec::new(),
            pending_previews: Vec::new(),
            kitty_graphics: false,
            last_rendered: None,
            last_content_area: None,
            last_document_area: None,
            last_tab_area: None,
            last_screen_area: None,
            pending_close: None,
            mouse_selection_anchor: None,
        })
    }

    pub fn restore_or_empty() -> Self {
        let restored = restore_session();
        let active = restored
            .as_ref()
            .map(|session| session.active)
            .unwrap_or_default();
        let mut tabs = restored.as_ref().map(tabs_from_session).unwrap_or_default();
        if tabs.is_empty() {
            tabs.push(Tab {
                title: "untitled.md".to_string(),
                path: None,
                document: import_gfm("").document,
                scroll_y: 0,
            });
        }
        let active = active.min(tabs.len().saturating_sub(1));
        Self {
            tabs,
            active,
            mode: Mode::Edit,
            command_input: String::new(),
            command_selection: 0,
            search_input: String::new(),
            link_input: String::new(),
            code_language_input: String::new(),
            heading_input: String::new(),
            footnote_label_input: String::new(),
            image_input: String::new(),
            message: "direct edit  Ctrl-S save  Ctrl-Q quit  ? help".to_string(),
            theme: Theme::ghostty_default_dark(),
            last_frame_ms: 0.0,
            last_parse_ms: 0.0,
            should_quit: false,
            clipboard: None,
            code_focus: CodeFocus::None,
            table_selection: None,
            graphics: Vec::new(),
            pending_previews: Vec::new(),
            kitty_graphics: false,
            last_rendered: None,
            last_content_area: None,
            last_document_area: None,
            last_tab_area: None,
            last_screen_area: None,
            pending_close: None,
            mouse_selection_anchor: None,
        }
    }

    pub fn current(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn current_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn save_current(&mut self) {
        let source = export_gfm(&self.current().document);
        let Some(path) = self.current().path.clone() else {
            self.message = "no path: use :w path/to/file.md".to_string();
            return;
        };
        match fs::write(&path, source) {
            Ok(()) => {
                self.current_mut().document.mark_saved();
                self.message = format!("saved {}", path.display());
            }
            Err(error) => {
                self.message = format!("save failed: {error}");
            }
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.path.as_ref() == Some(&path))
        {
            self.active = index;
            return;
        }
        match load_tab(&path) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.active = self.tabs.len().saturating_sub(1);
                self.message = format!("opened {}", path.display());
            }
            Err(error) => {
                self.message = format!("open failed: {error}");
            }
        }
    }

    pub fn close_current(&mut self) {
        if self.tabs.len() <= 1 {
            self.request_quit();
            return;
        }
        if self.current().document.dirty.is_dirty {
            self.pending_close = Some(self.active);
            self.mode = Mode::ConfirmClose;
            self.message = format!("discard unsaved changes in {}? y/n", self.current().title);
            return;
        }
        self.force_close_current();
    }

    pub fn request_quit(&mut self) {
        if self.tabs.iter().any(|tab| tab.document.dirty.is_dirty) {
            self.mode = Mode::ConfirmQuit;
            self.message = "discard unsaved changes and quit? y/n".to_string();
        } else {
            self.should_quit = true;
        }
    }

    fn force_close_current(&mut self) {
        if self.tabs.len() <= 1 {
            self.should_quit = true;
            return;
        }
        self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    fn confirm_close(&mut self) {
        let index = self
            .pending_close
            .take()
            .unwrap_or(self.active)
            .min(self.tabs.len().saturating_sub(1));
        self.active = index;
        self.force_close_current();
        self.mode = Mode::Edit;
        self.message = "tab closed".to_string();
    }

    fn cancel_confirmation(&mut self) {
        self.pending_close = None;
        self.mode = Mode::Edit;
        self.message = "cancelled".to_string();
    }

    pub fn session(&self) -> Session {
        Session {
            active: self.active.min(self.tabs.len().saturating_sub(1)),
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| {
                    Some(SessionTab {
                        path: tab.path.clone()?,
                        cursor_source_byte: tab.document.cursor.byte,
                        scroll_y: tab.scroll_y,
                        pinned: false,
                    })
                })
                .collect(),
        }
    }

    pub fn persist_session(&self) -> Result<()> {
        persist_session(&self.session())
    }

    fn execute_command(&mut self) {
        let command = self.command_input.trim().to_string();
        self.command_input.clear();
        self.mode = Mode::Edit;
        if command.is_empty() {
            return;
        }
        let mut parts = command.split_whitespace();
        match parts.next() {
            Some("q" | "quit") => self.request_quit(),
            Some("w" | "write") => {
                if let Some(path) = parts.next() {
                    let path = PathBuf::from(path);
                    self.current_mut().path = Some(path.clone());
                    self.current_mut().title = title_for_path(&path);
                }
                self.save_current();
            }
            Some("o" | "open") => {
                if let Some(path) = parts.next() {
                    self.open_path(PathBuf::from(path));
                } else {
                    self.message = "usage: :open path/to/file.md".to_string();
                }
            }
            Some("new") => {
                self.tabs.push(Tab {
                    title: "untitled.md".to_string(),
                    path: None,
                    document: import_gfm("").document,
                    scroll_y: 0,
                });
                self.active = self.tabs.len().saturating_sub(1);
            }
            Some(other) => {
                self.message = format!("unknown command: {other}");
            }
            None => {}
        }
    }
}

pub fn run(terminal: &mut TuiTerminal, app: &mut App) -> Result<()> {
    let mut dirty = true;
    let capabilities = TerminalCapabilities::detect();
    app.kitty_graphics = capabilities.kitty_graphics;
    let mut graphics_cache = KittyGraphicsCache::new();
    let mut external_preview_generation = external_preview_cache_generation();
    while !app.should_quit {
        let current_preview_generation = external_preview_cache_generation();
        if current_preview_generation != external_preview_generation {
            external_preview_generation = current_preview_generation;
            dirty = true;
        }
        if dirty {
            let started = Instant::now();
            terminal.draw(|frame| draw(frame, app))?;
            upload_rendered_graphics(
                terminal.backend_mut(),
                &capabilities,
                &mut graphics_cache,
                &app.graphics,
                &app.theme,
            )?;
            app.last_frame_ms = started.elapsed().as_secs_f64() * 1000.0;
            dirty = warm_rendered_previews(&app.pending_previews, &app.theme);
        }

        if event::poll(Duration::from_millis(250))? {
            let event = event::read()?;
            dirty = handle_event(app, event);
            if let Some(text) = app.clipboard.take() {
                let len = text.len();
                let method = copy_to_clipboard_or_osc52(terminal.backend_mut(), &text)?;
                app.message = match method {
                    ClipboardMethod::Native => {
                        format!("copied code body to native clipboard ({len} bytes)")
                    }
                    ClipboardMethod::Osc52Fallback => {
                        format!("copied code body via OSC 52 fallback ({len} bytes)")
                    }
                };
                dirty = true;
            }
        }
    }
    app.persist_session()?;
    Ok(())
}

fn upload_rendered_graphics<W: Write>(
    writer: &mut W,
    capabilities: &TerminalCapabilities,
    cache: &mut KittyGraphicsCache,
    graphics: &[RenderedGraphic],
    theme: &Theme,
) -> std::io::Result<()> {
    if !capabilities.kitty_graphics {
        return Ok(());
    }
    let theme_hash = graphics_theme_hash(theme);
    for graphic in graphics {
        match &graphic.kind {
            RenderedGraphicKind::Heading { level, text } => {
                let key = GraphicsCacheKey::heading(*level, text, graphic.width_cells, theme_hash);
                if cache.get(&key).is_some() {
                    continue;
                }
                let image = cache.get_or_insert_with_id(
                    key,
                    Some(graphic.image_id),
                    heading_width_px(graphic.width_cells),
                    heading_height_px(*level),
                    graphic.z_index,
                );
                let rgba = raster_heading_rgba(text, *level, graphic.width_cells, theme);
                write!(writer, "{}", kitty_transmit_rgba(image, &rgba))?;
                write!(
                    writer,
                    "{}",
                    kitty_create_virtual_placement(
                        image,
                        graphic.width_cells,
                        graphic.height_cells
                    )
                )?;
            }
            RenderedGraphicKind::LocalImage { path, .. } => {
                let key = GraphicsCacheKey::local_image(
                    &path.to_string_lossy(),
                    graphic.width_cells,
                    theme_hash,
                );
                if cache.get(&key).is_some() {
                    continue;
                }
                let Ok(png) = fs::read(path) else {
                    continue;
                };
                let Some(raster) = raster_local_image_rgba(
                    &png,
                    image_preview_width_px(graphic.width_cells),
                    image_preview_height_px(graphic.height_cells),
                ) else {
                    continue;
                };
                let image = cache.get_or_insert_with_id(
                    key,
                    Some(graphic.image_id),
                    raster.width_px,
                    raster.height_px,
                    graphic.z_index,
                );
                write!(writer, "{}", kitty_transmit_rgba(image, &raster.rgba))?;
                write!(
                    writer,
                    "{}",
                    kitty_create_virtual_placement(
                        image,
                        graphic.width_cells,
                        graphic.height_cells
                    )
                )?;
            }
            RenderedGraphicKind::Preview { label, source_hash } => {
                let external = cached_external_preview_raster(
                    label,
                    *source_hash,
                    graphic.width_cells,
                    graphic.height_cells,
                    theme_hash,
                );
                let key = if external.is_some() {
                    GraphicsCacheKey::external_preview(
                        label,
                        *source_hash,
                        graphic.width_cells,
                        theme_hash,
                    )
                } else {
                    GraphicsCacheKey::preview(label, *source_hash, graphic.width_cells, theme_hash)
                };
                if cache.get(&key).is_some() {
                    continue;
                }
                let (width_px, height_px, rgba) = if let Some(raster) = external {
                    (raster.width_px, raster.height_px, raster.rgba)
                } else {
                    (
                        image_preview_width_px(graphic.width_cells),
                        image_preview_height_px(graphic.height_cells),
                        raster_preview_rgba(
                            label,
                            *source_hash,
                            graphic.width_cells,
                            graphic.height_cells,
                            theme,
                        ),
                    )
                };
                let image = cache.get_or_insert_with_id(
                    key,
                    Some(graphic.image_id),
                    width_px,
                    height_px,
                    graphic.z_index,
                );
                write!(writer, "{}", kitty_transmit_rgba(image, &rgba))?;
                write!(
                    writer,
                    "{}",
                    kitty_create_virtual_placement(
                        image,
                        graphic.width_cells,
                        graphic.height_cells
                    )
                )?;
            }
        }
    }
    writer.flush()
}

fn warm_rendered_previews(previews: &[PreviewRequest], preview_theme: &Theme) -> bool {
    let mut dirty = false;
    for preview in previews {
        queue_external_preview_render(
            &preview.label,
            preview.source_hash,
            &preview.source,
            preview.width_cells,
            preview.height_cells,
            preview_theme,
        );
        dirty |= warm_preview(preview);
    }
    dirty
}

pub fn handle_event(app: &mut App, event: Event) -> bool {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
            app.mouse_selection_anchor = None;
            handle_key(app, key);
            true
        }
        Event::Paste(text) => {
            app.mouse_selection_anchor = None;
            paste_text(app, &text)
        }
        Event::Mouse(mouse) if app.mode == Mode::StylePalette => {
            handle_style_palette_mouse(app, mouse.kind, mouse.column, mouse.row);
            true
        }
        Event::Mouse(mouse) if app.mode == Mode::InsertMenu => {
            handle_insert_menu_mouse(app, mouse.kind, mouse.column, mouse.row);
            true
        }
        Event::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left)
                    if click_selection_style_palette_at(app, mouse.column, mouse.row) => {}
                MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left)
                    if click_scrollbar_at(app, mouse.column, mouse.row) => {}
                MouseEventKind::Down(MouseButton::Left)
                    if click_document_at(app, mouse.column, mouse.row) => {}
                MouseEventKind::Drag(MouseButton::Left)
                    if drag_document_at(app, mouse.column, mouse.row) => {}
                MouseEventKind::Up(MouseButton::Left) => {
                    app.mouse_selection_anchor = None;
                }
                MouseEventKind::ScrollDown => {
                    app.mouse_selection_anchor = None;
                    app.current_mut().scroll_y = app.current().scroll_y.saturating_add(3);
                }
                MouseEventKind::ScrollUp => {
                    app.mouse_selection_anchor = None;
                    app.current_mut().scroll_y = app.current().scroll_y.saturating_sub(3);
                }
                MouseEventKind::Down(MouseButton::Left)
                    if click_tab_at(app, mouse.column, mouse.row) =>
                {
                    app.mouse_selection_anchor = None;
                }
                MouseEventKind::Down(MouseButton::Middle)
                    if close_tab_at(app, mouse.column, mouse.row) =>
                {
                    app.mouse_selection_anchor = None;
                }
                _ => {
                    app.mouse_selection_anchor = None;
                }
            }
            true
        }
        Event::Resize(_, _) => {
            app.mouse_selection_anchor = None;
            true
        }
        _ => false,
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    match app.mode {
        Mode::Help => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => app.mode = Mode::Edit,
                _ => {}
            }
            return;
        }
        Mode::ConfirmQuit => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => app.should_quit = true,
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_confirmation(),
                _ => {}
            }
            return;
        }
        Mode::ConfirmClose => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_close(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_confirmation(),
                _ => {}
            }
            return;
        }
        Mode::Command => {
            match key.code {
                KeyCode::Esc => {
                    app.command_input.clear();
                    app.command_selection = 0;
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => execute_command_palette(app),
                KeyCode::Up => move_palette_selection(app, false),
                KeyCode::Down => move_palette_selection(app, true),
                KeyCode::Backspace => {
                    app.command_input.pop();
                    app.command_selection = 0;
                }
                KeyCode::Char(ch) => {
                    app.command_input.push(ch);
                    app.command_selection = 0;
                }
                _ => {}
            }
            return;
        }
        Mode::Search => {
            match key.code {
                KeyCode::Esc => {
                    app.search_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    search_current(app, key.modifiers.contains(KeyModifiers::SHIFT));
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.search_input.pop();
                }
                KeyCode::Char(ch) => app.search_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::LinkPrompt => {
            match key.code {
                KeyCode::Esc => {
                    app.link_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    let input = app.link_input.clone();
                    app.link_input.clear();
                    let fields = parse_link_prompt(&input);
                    let changed = if fields.rich {
                        app.current_mut().document.set_link_at_cursor(
                            &fields.label,
                            &fields.destination,
                            fields.title.as_deref(),
                            fields.reference_label.as_deref(),
                        )
                    } else if let Some(target) = app.current().document.link_at_cursor() {
                        app.current_mut().document.set_link_at_cursor(
                            &target.label,
                            &fields.destination,
                            target.title.as_deref(),
                            target.reference_label.as_deref(),
                        )
                    } else {
                        app.current_mut()
                            .document
                            .insert_link_at_cursor(&fields.destination)
                    };
                    app.message = if changed {
                        "link updated".to_string()
                    } else {
                        "link edit expects url or label|url|title|ref".to_string()
                    };
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.link_input.pop();
                }
                KeyCode::Char(ch) => app.link_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::CodeLanguagePrompt => {
            match key.code {
                KeyCode::Esc => {
                    app.code_language_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    let language = app.code_language_input.clone();
                    app.code_language_input.clear();
                    if app
                        .current_mut()
                        .document
                        .set_code_language_at_cursor(&language)
                    {
                        app.message = "code language updated".to_string();
                    } else {
                        app.message = "cursor is not on a fenced code block".to_string();
                    }
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.code_language_input.pop();
                }
                KeyCode::Char(ch) => app.code_language_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::HeadingPrompt => {
            match key.code {
                KeyCode::Esc => {
                    app.heading_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    let input = app.heading_input.clone();
                    app.heading_input.clear();
                    let (level, text) = parse_heading_prompt(&input);
                    if app
                        .current_mut()
                        .document
                        .set_heading_at_cursor(level, &text)
                    {
                        app.message = "heading updated".to_string();
                    } else {
                        app.message = "heading edit expects level|text".to_string();
                    }
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.heading_input.pop();
                }
                KeyCode::Char(ch) => app.heading_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::FootnoteLabelPrompt => {
            match key.code {
                KeyCode::Esc => {
                    app.footnote_label_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    let label = app.footnote_label_input.clone();
                    app.footnote_label_input.clear();
                    if app.current_mut().document.rename_footnote_at_cursor(&label) {
                        app.message = "footnote label updated".to_string();
                    } else {
                        app.message = "cursor is not on a footnote label".to_string();
                    }
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.footnote_label_input.pop();
                }
                KeyCode::Char(ch) => app.footnote_label_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::ImagePrompt => {
            match key.code {
                KeyCode::Esc => {
                    app.image_input.clear();
                    app.mode = Mode::Edit;
                }
                KeyCode::Enter => {
                    let input = app.image_input.clone();
                    app.image_input.clear();
                    let (alt, src, title) = parse_image_prompt(&input);
                    if app
                        .current_mut()
                        .document
                        .set_image_at_cursor(&alt, &src, title.as_deref())
                    {
                        app.message = "image updated".to_string();
                    } else {
                        app.message = "image edit expects alt|src|title".to_string();
                    }
                    app.mode = Mode::Edit;
                }
                KeyCode::Backspace => {
                    app.image_input.pop();
                }
                KeyCode::Char(ch) => app.image_input.push(ch),
                _ => {}
            }
            return;
        }
        Mode::TableActions => {
            match key.code {
                KeyCode::Esc => app.mode = Mode::Edit,
                KeyCode::Char('R') => {
                    select_table_row(app);
                    app.mode = Mode::Edit;
                }
                KeyCode::Char('C') => {
                    select_table_column(app);
                    app.mode = Mode::Edit;
                }
                KeyCode::Char('r') => {
                    if app.current_mut().document.remove_table_row_at_cursor() {
                        app.message = "table row removed".to_string();
                    }
                    app.table_selection = None;
                    app.mode = Mode::Edit;
                }
                KeyCode::Char('c') => {
                    if app.current_mut().document.remove_table_column_at_cursor() {
                        app.message = "table column removed".to_string();
                    }
                    app.table_selection = None;
                    app.mode = Mode::Edit;
                }
                KeyCode::Char('n') => {
                    if app.current_mut().document.normalize_table_at_cursor() {
                        app.message = "table normalized".to_string();
                    }
                    app.mode = Mode::Edit;
                }
                _ => {}
            }
            return;
        }
        Mode::StylePalette => {
            match key.code {
                KeyCode::Esc => app.mode = Mode::Edit,
                KeyCode::Char('b') | KeyCode::Char('B') => {
                    apply_inline_style_action(app, InlineStyleAction::Bold);
                }
                KeyCode::Char('i') | KeyCode::Char('I') => {
                    apply_inline_style_action(app, InlineStyleAction::Italic);
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    apply_inline_style_action(app, InlineStyleAction::Strike);
                }
                KeyCode::Char('`') | KeyCode::Char('c') | KeyCode::Char('C') => {
                    apply_inline_style_action(app, InlineStyleAction::Code);
                }
                KeyCode::Char('k') | KeyCode::Char('K') => {
                    apply_inline_style_action(app, InlineStyleAction::Link);
                }
                _ => {}
            }
            return;
        }
        Mode::InsertMenu => {
            match key.code {
                KeyCode::Esc => app.mode = Mode::Edit,
                KeyCode::Char(ch) => {
                    if let Some(action) = insert_action_for_key(ch) {
                        apply_insert_block_action(app, action);
                    }
                }
                _ => {}
            }
            return;
        }
        Mode::Edit => {}
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(level) = heading_level_key(ch) {
                    let changed = app
                        .current_mut()
                        .document
                        .set_heading_level_at_cursor(level);
                    set_message_if(app, changed, "heading level changed");
                }
            }
            KeyCode::Char('q') => app.request_quit(),
            KeyCode::Char('s') => app.save_current(),
            KeyCode::Char('n') => {
                app.tabs.push(Tab {
                    title: "untitled.md".to_string(),
                    path: None,
                    document: import_gfm("").document,
                    scroll_y: 0,
                });
                app.active = app.tabs.len().saturating_sub(1);
            }
            KeyCode::Char('o') => {
                app.command_input = "open ".to_string();
                app.command_selection = 0;
                app.mode = Mode::Command;
            }
            KeyCode::Char('p') => {
                app.command_input.clear();
                app.command_selection = 0;
                app.mode = Mode::Command;
            }
            KeyCode::Char('f') => app.mode = Mode::Search,
            KeyCode::Char('k') => open_link_prompt(app),
            KeyCode::Char('e') => app.mode = Mode::StylePalette,
            KeyCode::Char('a') => {
                app.current_mut().document.select_all();
                app.message = "document selected".to_string();
            }
            KeyCode::Char('b') => {
                let changed = app.current_mut().document.toggle_strong_at_cursor();
                set_message_if(app, changed, "bold toggled");
            }
            KeyCode::Char('i') => {
                let changed = app.current_mut().document.toggle_emphasis_at_cursor();
                set_message_if(app, changed, "italic toggled");
            }
            KeyCode::Char('`') => {
                let changed = app.current_mut().document.toggle_inline_code_at_cursor();
                set_message_if(app, changed, "inline code toggled");
            }
            KeyCode::Char('l') => {
                app.code_language_input.clear();
                app.mode = Mode::CodeLanguagePrompt;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                queue_code_body_copy(app);
            }
            KeyCode::Char('z') => {
                let _changed = app.current_mut().document.undo();
            }
            KeyCode::Char('y') => {
                let _changed = app.current_mut().document.redo();
            }
            KeyCode::Right
                if key.modifiers.contains(KeyModifiers::ALT) && is_table_context(app) =>
            {
                app.table_selection = None;
                let changed = app
                    .current_mut()
                    .document
                    .change_table_column_alignment_at_cursor(true);
                set_message_if(app, changed, "table column alignment changed");
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) && is_table_context(app) => {
                app.table_selection = None;
                let changed = app
                    .current_mut()
                    .document
                    .change_table_column_alignment_at_cursor(false);
                set_message_if(app, changed, "table column alignment changed");
            }
            KeyCode::Right => app.current_mut().document.move_word_right(),
            KeyCode::Left => app.current_mut().document.move_word_left(),
            KeyCode::BackTab if key.modifiers.contains(KeyModifiers::SHIFT) => previous_tab(app),
            KeyCode::Tab => next_tab(app),
            _ => {}
        }
        keep_cursor_visible(app, 20);
        return;
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Enter if is_footnote_context(app) => {
                let changed = app
                    .current_mut()
                    .document
                    .create_footnote_definition_at_cursor();
                set_message_if(app, changed, "footnote definition ready");
            }
            KeyCode::Enter if is_gap_context(app) => app.mode = Mode::InsertMenu,
            KeyCode::Enter => app.current_mut().document.alt_enter(),
            KeyCode::Down if is_table_context(app) => {
                app.table_selection = None;
                let changed = app.current_mut().document.insert_table_row_at_cursor(false);
                set_message_if(app, changed, "table row inserted");
            }
            KeyCode::Up if is_table_context(app) => {
                app.table_selection = None;
                let changed = app.current_mut().document.insert_table_row_at_cursor(true);
                set_message_if(app, changed, "table row inserted");
            }
            KeyCode::Right if is_table_context(app) => {
                app.table_selection = None;
                let changed = app
                    .current_mut()
                    .document
                    .insert_table_column_at_cursor(false);
                set_message_if(app, changed, "table column inserted");
            }
            KeyCode::Left if is_table_context(app) => {
                app.table_selection = None;
                let changed = app
                    .current_mut()
                    .document
                    .insert_table_column_at_cursor(true);
                set_message_if(app, changed, "table column inserted");
            }
            KeyCode::Down | KeyCode::Right => app.current_mut().document.structural_move(true),
            KeyCode::Up | KeyCode::Left => app.current_mut().document.structural_move(false),
            _ => {}
        }
        keep_cursor_visible(app, 20);
        return;
    }

    if key.modifiers.contains(KeyModifiers::SHIFT) {
        let handled = match key.code {
            KeyCode::Left => {
                app.current_mut().document.move_left_select();
                true
            }
            KeyCode::Right => {
                app.current_mut().document.move_right_select();
                true
            }
            KeyCode::Up => {
                app.current_mut().document.move_up_select();
                true
            }
            KeyCode::Down => {
                app.current_mut().document.move_down_select();
                true
            }
            _ => false,
        };
        if handled {
            keep_cursor_visible(app, 20);
            return;
        }
    }

    match key.code {
        KeyCode::Char('?') => app.mode = Mode::Help,
        KeyCode::Char(':') => {
            app.command_input.clear();
            app.command_selection = 0;
            app.mode = Mode::Command;
        }
        KeyCode::Char('/') => app.mode = Mode::Search,
        KeyCode::Char(' ') if is_task_context(app) => {
            let changed = app.current_mut().document.toggle_task_at_cursor();
            set_message_if(app, changed, "task toggled");
        }
        KeyCode::Char('e') | KeyCode::F(2) => open_component_popup(app),
        KeyCode::Left => app.current_mut().document.move_left(),
        KeyCode::Right => app.current_mut().document.move_right(),
        KeyCode::Up => app.current_mut().document.move_up(),
        KeyCode::Down => app.current_mut().document.move_down(),
        KeyCode::PageUp => {
            app.current_mut().scroll_y = app.current().scroll_y.saturating_sub(20);
        }
        KeyCode::PageDown => {
            app.current_mut().scroll_y = app.current().scroll_y.saturating_add(20);
        }
        KeyCode::Enter if app.code_focus == CodeFocus::Language && is_code_context(app) => {
            cycle_code_focus(app, true);
        }
        KeyCode::Enter if is_code_copy_focus(app) => {
            queue_code_body_copy(app);
        }
        KeyCode::Enter if is_table_context(app) => {
            app.table_selection = None;
            let changed = app.current_mut().document.enter_table_cell();
            set_message_if(app, changed, "table cell advanced");
        }
        KeyCode::Enter if is_footnote_context(app) => {
            let changed = app
                .current_mut()
                .document
                .jump_to_footnote_definition_at_cursor();
            set_message_if(app, changed, "jumped to footnote definition");
        }
        KeyCode::Enter => app.current_mut().document.enter(),
        KeyCode::Backspace if is_table_context(app) => {
            app.table_selection = None;
            if !app
                .current_mut()
                .document
                .remove_empty_table_row_at_cursor()
            {
                app.current_mut().document.backspace();
            } else {
                app.message = "empty table row removed".to_string();
            }
        }
        KeyCode::Backspace if is_code_copy_focus(app) => {
            app.message = "copy button focused; Enter copies code body".to_string();
        }
        KeyCode::Backspace => {
            app.code_focus = CodeFocus::None;
            app.current_mut().document.backspace();
        }
        KeyCode::Delete if is_code_copy_focus(app) => {
            app.message = "copy button focused; Enter copies code body".to_string();
        }
        KeyCode::Delete if app.table_selection.is_some() => delete_table_selection(app),
        KeyCode::Delete => {
            app.code_focus = CodeFocus::None;
            app.table_selection = None;
            app.current_mut().document.delete();
        }
        KeyCode::Tab if is_table_context(app) => {
            app.table_selection = None;
            let changed = app.current_mut().document.move_table_cell(true);
            set_message_if(app, changed, "next table cell");
        }
        KeyCode::BackTab if is_table_context(app) => {
            app.table_selection = None;
            let changed = app.current_mut().document.move_table_cell(false);
            set_message_if(app, changed, "previous table cell");
        }
        KeyCode::Tab if is_code_context(app) => cycle_code_focus(app, true),
        KeyCode::BackTab if is_code_context(app) => cycle_code_focus(app, false),
        KeyCode::Tab | KeyCode::BackTab => app.current_mut().document.structural_move(true),
        KeyCode::Char(_) if is_code_copy_focus(app) => {
            app.message = "copy button focused; Enter copies code body".to_string();
        }
        KeyCode::Char(ch) => {
            app.code_focus = code_focus_after_text_edit(app);
            app.current_mut().document.insert_char(ch);
        }
        KeyCode::Esc => {}
        _ => {}
    }
    keep_cursor_visible(app, 20);
}

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let size = frame.area();
    let theme = app.theme.clone();
    app.last_screen_area = Some(size);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(size);

    render_tabs(frame, layout[0], app, &theme);
    app.last_tab_area = Some(layout[0]);
    let document_area = layout[1];
    let content_area = Rect {
        x: document_area.x,
        y: document_area.y.saturating_sub(1),
        width: document_area.width.saturating_sub(1),
        height: document_area.height.saturating_add(1),
    };
    let rendered = {
        let width = document_text_area(content_area).width;
        let inner_height = usize::from(content_area.height.saturating_sub(2));
        let scroll_y = app.current().scroll_y;
        let kitty_graphics = app.kitty_graphics;
        render_document_window_with_cache(
            &mut app.current_mut().document,
            width,
            scroll_y,
            inner_height,
            inner_height.saturating_mul(2),
            RenderOptions {
                kitty_placeholders: kitty_graphics,
                preview_graphics: kitty_graphics,
                image_widget_previews: true,
            },
        )
    };
    app.graphics = rendered.graphics.clone();
    app.pending_previews = rendered.pending_previews.clone();
    app.last_content_area = Some(content_area);
    app.last_document_area = Some(document_area);
    app.last_rendered = Some(rendered.clone());
    render_document_panel(frame, content_area, app, &theme, &rendered);
    render_scrollbar(
        frame,
        document_area,
        app.current().scroll_y,
        rendered.total_rows,
        &theme,
    );
    render_status(frame, layout[2], app, &theme);

    if app.mode == Mode::Help {
        render_help(frame, size, &theme);
    } else if app.mode == Mode::Command {
        render_command_palette(frame, size, app, &theme);
    } else if app.mode == Mode::Search {
        render_prompt(frame, size, "/", &app.search_input, &theme);
    } else if app.mode == Mode::LinkPrompt {
        render_prompt(frame, size, "label|url|title|ref ", &app.link_input, &theme);
    } else if app.mode == Mode::CodeLanguagePrompt {
        render_prompt(frame, size, "language ", &app.code_language_input, &theme);
    } else if app.mode == Mode::HeadingPrompt {
        render_prompt(frame, size, "level|heading ", &app.heading_input, &theme);
    } else if app.mode == Mode::FootnoteLabelPrompt {
        render_prompt(frame, size, "footnote ", &app.footnote_label_input, &theme);
    } else if app.mode == Mode::ImagePrompt {
        render_prompt(frame, size, "alt|src|title ", &app.image_input, &theme);
    } else if app.mode == Mode::TableActions {
        render_table_actions(frame, size, &theme);
    } else if app.mode == Mode::StylePalette {
        render_style_palette(frame, size, &theme);
    } else if app.mode == Mode::InsertMenu {
        render_insert_menu(frame, size, &theme);
    } else if app.mode == Mode::ConfirmQuit {
        render_prompt(frame, size, "Discard changes and quit? y/n ", "", &theme);
    } else if app.mode == Mode::ConfirmClose {
        render_prompt(
            frame,
            size,
            "Discard changes and close tab? y/n ",
            "",
            &theme,
        );
    } else if app.current().document.selection_range().is_some() {
        render_style_palette(frame, size, &theme);
    }

    set_cursor(frame, content_area, app, &rendered);
}

fn render_tabs(frame: &mut Frame<'_>, area: Rect, app: &App, theme: &Theme) {
    let labels = app
        .tabs
        .iter()
        .map(|tab| {
            let dirty = if tab.document.dirty.is_dirty {
                " *"
            } else {
                ""
            };
            format!("{}{}", tab.title, dirty)
        })
        .collect::<Vec<_>>();
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let fallback = ["untitled.md"];
    let tabs = if refs.is_empty() {
        fallback.as_slice()
    } else {
        refs.as_slice()
    };
    tabbar::render(
        frame,
        area,
        theme,
        tabs,
        app.active.min(tabs.len().saturating_sub(1)),
        color(theme.bg_raised),
    );
}

fn render_document_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: &Theme,
    rendered: &RenderedDocument,
) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg)));
    let inner = document_panel_inner(area);
    frame.render_widget(block, area);
    let text_area = document_text_area(area);

    let scroll = app
        .current()
        .scroll_y
        .min(rendered.total_rows.saturating_sub(1));
    let selection = app.current().document.selection_range();
    let mut scroll_view = ScrollView::new(Size::new(text_area.width.max(1), inner.height.max(1)))
        .scrollbars_visibility(ScrollbarVisibility::Never);
    let scroll_area = Rect {
        x: 0,
        y: 0,
        width: text_area.width,
        height: inner.height,
    };
    for line in &rendered.lines {
        if line.y_doc < scroll {
            continue;
        }
        let row = line.y_doc - scroll;
        if row >= usize::from(inner.height) {
            break;
        }
        render_line(
            scroll_view.buf_mut(),
            scroll_area,
            row as u16,
            line,
            selection.as_ref(),
            theme,
        );
    }
    let mut scroll_state = ScrollViewState::default();
    frame.render_stateful_widget(scroll_view, text_area, &mut scroll_state);
    render_ratatui_image_previews(frame, text_area, app, rendered);
}

fn render_line(
    buffer: &mut Buffer,
    area: Rect,
    row: u16,
    line: &RenderLine,
    selection: Option<&std::ops::Range<usize>>,
    theme: &Theme,
) {
    if row >= area.height {
        return;
    }
    let mut x = area.x;
    let y = area.y + row;
    for cell in &line.cells {
        if x >= area.x.saturating_add(area.width) {
            break;
        }
        if let Some(checked) = task_checkbox_cell(cell.text.as_str()) {
            render_checkbox_cell(buffer, area, x, y, checked, cell.style, theme);
            x = x.saturating_add(UnicodeWidthStr::width(cell.text.as_str()) as u16);
            continue;
        }
        for grapheme in UnicodeSegmentation::graphemes(cell.text.as_str(), true) {
            if x >= area.x.saturating_add(area.width) {
                break;
            }
            let width = UnicodeWidthStr::width(grapheme) as u16;
            if width == 0 {
                continue;
            }
            let mut style = cell.style;
            if let Some(selection) = selection {
                style.reversed |= line_columns_intersect_selection(
                    line,
                    x.saturating_sub(area.x),
                    width,
                    selection,
                );
            }
            buffer.set_string(x, y, grapheme, style_for(style, theme));
            x = x.saturating_add(width);
        }
    }
}

fn task_checkbox_cell(text: &str) -> Option<bool> {
    let trimmed = text.trim();
    if trimmed == "☑" {
        Some(true)
    } else if trimmed == "☐" {
        Some(false)
    } else {
        None
    }
}

fn render_checkbox_cell(
    buffer: &mut Buffer,
    area: Rect,
    x: u16,
    y: u16,
    checked: bool,
    style: CellStyle,
    theme: &Theme,
) {
    let base = style_for(style, theme);
    let right = area.x.saturating_add(area.width);
    if x < right {
        buffer.set_string(x, y, " ", base);
    }
    if x.saturating_add(1) < right {
        buffer.set_string(x.saturating_add(1), y, " ", base);
    }
    let checkbox_x = x.saturating_add(2);
    if checkbox_x >= right {
        return;
    }
    let checkbox_width = right.saturating_sub(checkbox_x).min(3);
    let checkbox = Checkbox::new("", checked)
        .checked_symbol("☑ ")
        .unchecked_symbol("☐ ")
        .style(base)
        .checkbox_style(base);
    checkbox.render(
        Rect {
            x: checkbox_x,
            y,
            width: checkbox_width,
            height: 1,
        },
        buffer,
    );
}

fn render_ratatui_image_previews(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    rendered: &RenderedDocument,
) {
    if app.kitty_graphics {
        return;
    }
    let scroll = app.current().scroll_y;
    for graphic in &rendered.graphics {
        let RenderedGraphicKind::LocalImage { path, .. } = &graphic.kind else {
            continue;
        };
        if graphic
            .y_doc
            .saturating_add(usize::from(graphic.height_cells))
            <= scroll
            || graphic.y_doc >= scroll.saturating_add(usize::from(area.height))
        {
            continue;
        }
        let row = graphic.y_doc.saturating_sub(scroll) as u16;
        let preview_area = Rect {
            x: area.x,
            y: area.y.saturating_add(row),
            width: graphic.width_cells.min(area.width).max(1),
            height: graphic
                .height_cells
                .min(area.height.saturating_sub(row))
                .max(1),
        };
        let Ok(reader) = ImageReader::open(path) else {
            continue;
        };
        let Ok(decoded) = reader.decode() else {
            continue;
        };
        let picker = Picker::from_fontsize((8, 16));
        let Ok(protocol) = picker.new_protocol(decoded, preview_area, Resize::Fit(None)) else {
            continue;
        };
        frame.render_widget(Image::new(&protocol), preview_area);
    }
}

fn line_columns_intersect_selection(
    line: &RenderLine,
    x: u16,
    width: u16,
    selection: &std::ops::Range<usize>,
) -> bool {
    let cell_end = x.saturating_add(width.max(1));
    line.source_spans.iter().any(|span| {
        let span_width = span.width.max(1);
        let span_end = span.x.saturating_add(span_width);
        let overlap_start = x.max(span.x);
        let overlap_end = cell_end.min(span_end);
        if overlap_start >= overlap_end {
            return false;
        }
        let source = source_range_for_columns(span, overlap_start, overlap_end);
        source.start < selection.end && source.end > selection.start
    })
}

fn source_range_for_columns(
    span: &SourceCellSpan,
    start_x: u16,
    end_x: u16,
) -> std::ops::Range<usize> {
    let source_len = span.source.end.saturating_sub(span.source.start);
    if source_len == 0 {
        return span.source.start..span.source.end;
    }
    let width = usize::from(span.width.max(1));
    let start_offset = usize::from(start_x.saturating_sub(span.x)).min(width);
    let end_offset = usize::from(end_x.saturating_sub(span.x)).min(width);
    let start = span
        .source
        .start
        .saturating_add(source_len.saturating_mul(start_offset) / width);
    let mut end = span
        .source
        .start
        .saturating_add(source_len.saturating_mul(end_offset) / width);
    if end <= start && end_offset > start_offset {
        end = start.saturating_add(1).min(span.source.end);
    }
    start..end
}

fn render_scrollbar(frame: &mut Frame<'_>, area: Rect, scroll: usize, total: usize, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let x = area.x + area.width.saturating_sub(1);
    let height = area.height.max(1);
    for offset in 0..height {
        frame.buffer_mut().set_string(
            x,
            area.y + offset,
            "│",
            Style::default()
                .fg(color(theme.line_soft))
                .bg(color(theme.bg)),
        );
    }
    if total <= usize::from(height) {
        return;
    }
    let thumb_height = ((usize::from(height) * usize::from(height)) / total).max(1) as u16;
    let max_scroll = total.saturating_sub(usize::from(height)).max(1);
    let travel = height.saturating_sub(thumb_height);
    let thumb_y = area.y + ((scroll.min(max_scroll) * usize::from(travel)) / max_scroll) as u16;
    for offset in 0..thumb_height {
        frame.buffer_mut().set_string(
            x,
            thumb_y + offset,
            "█",
            Style::default().fg(color(theme.accent)).bg(color(theme.bg)),
        );
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App, theme: &Theme) {
    let snapshot = app.current().document.cursor_snapshot();
    let component = code_focus_label(app)
        .or_else(|| table_selection_label(app))
        .unwrap_or_else(|| {
            snapshot
                .component
                .as_ref()
                .map(component_label)
                .unwrap_or("Gap")
        });
    let parser = if app.current().document.parse.ok {
        "GFM+TS ok"
    } else {
        "GFM+TS partial"
    };
    let dirty = if app.current().document.dirty.is_dirty {
        "*"
    } else {
        ""
    };
    let mode = match app.mode {
        Mode::Edit => "EDIT",
        Mode::Help => "HELP",
        Mode::Command => "CMD",
        Mode::Search => "SEARCH",
        Mode::LinkPrompt => "LINK",
        Mode::CodeLanguagePrompt => "LANG",
        Mode::HeadingPrompt => "HEADING",
        Mode::FootnoteLabelPrompt => "FOOTNOTE",
        Mode::ImagePrompt => "IMAGE",
        Mode::TableActions => "TABLE",
        Mode::StylePalette => "STYLE",
        Mode::InsertMenu => "INSERT",
        Mode::ConfirmQuit => "QUIT?",
        Mode::ConfirmClose => "CLOSE?",
    };
    let text = format!(
        " {mode}  {}{dirty}  Ln {} Col {}  {}  {} {}b/{}i  {} diagnostics  {:.1}ms frame  {}",
        app.current().title,
        snapshot.line,
        snapshot.column,
        component,
        parser,
        app.current().document.parse.tree_sitter.block_node_count,
        app.current().document.parse.tree_sitter.inline_tree_count,
        app.current().document.parse.diagnostics.len(),
        app.last_frame_ms,
        app.message
    );
    Paragraph::new(text)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_soft)),
        )
        .render(area, frame.buffer_mut());
}

fn render_help(frame: &mut Frame<'_>, size: Rect, theme: &Theme) {
    let width = bounded_popup_extent(size.width, 40, 72);
    let height = bounded_popup_extent(size.height, 12, 20);
    let area = centered_rect(size, width, height);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    let text = vec![
        Line::from("Navigation"),
        Line::from(
            "  arrows move visually   Alt+arrows move by component   Ctrl+arrows move words",
        ),
        Line::from("Editing"),
        Line::from("  type inserts text   Enter splits line   Alt+Enter inserts hard break"),
        Line::from("  Space toggles task boxes   Ctrl+K links current word   F2 edits component"),
        Line::from("  Headings: F2 edits level/text   Ctrl+Alt+1..6 changes level"),
        Line::from("  Gap: Alt+Enter opens insert menu for paragraph/heading/code/table/media"),
        Line::from("  Selection shows style palette   click or B/I/S/C/K applies inline style"),
        Line::from("  Links: F2 edits label|url|title|ref fields"),
        Line::from("  Tables: Alt+arrows insert rows/columns   F2 opens table actions"),
        Line::from("  Tables: F2 R/C selects row/column   Delete removes selected target"),
        Line::from("  Tables: Ctrl+Alt+Left/Right changes focused column alignment"),
        Line::from("  Footnotes: Enter jumps   Alt+Enter creates missing definition   F2 renames"),
        Line::from("  Code: Tab cycles language/body/copy   Ctrl+L language   Ctrl+Shift+C copy"),
        Line::from("  Ctrl+Z undo   Ctrl+Y redo   Ctrl+S save   Ctrl+F or / search"),
        Line::from("  Search prompt: Enter next match   Shift+Enter previous match"),
        Line::from("Tabs"),
        Line::from(
            "  Ctrl+N new tab   Ctrl+Tab next tab   mouse click selects tab   middle click closes",
        ),
        Line::from(
            "  Mouse: click document moves cursor   drag selects   checkbox toggles   scrollbar jumps",
        ),
        Line::from("Commands"),
        Line::from("  : or Ctrl+P fuzzy palette   :open path   :write [path]   :quit"),
        Line::from("Misc"),
        Line::from("  / search   Esc closes popups   Ctrl+Q quits"),
    ];
    Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn render_table_actions(frame: &mut Frame<'_>, size: Rect, theme: &Theme) {
    let area = centered_rect(
        size,
        bounded_popup_extent(size.width, 34, 48),
        9.min(size.height),
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Table ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    let text = vec![
        Line::from("R  select focused row"),
        Line::from("C  select focused column"),
        Line::from("r  remove focused row"),
        Line::from("c  remove focused column"),
        Line::from("n  normalize table"),
        Line::from("Esc close"),
    ];
    Paragraph::new(text)
        .block(block)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        )
        .render(area, frame.buffer_mut());
}

fn insert_menu_area(size: Rect) -> Rect {
    centered_rect(
        size,
        bounded_popup_extent(size.width, 38, 54),
        14.min(size.height),
    )
}

fn render_insert_menu(frame: &mut Frame<'_>, size: Rect, theme: &Theme) {
    let area = insert_menu_area(size);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Insert Block ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    let rows = insert_menu_entries()
        .iter()
        .map(|(key, label, _action)| {
            Line::from(vec![
                Span::styled(
                    format!("{key} "),
                    Style::default()
                        .fg(color(theme.accent))
                        .bg(color(theme.bg_raised))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*label, Style::default().fg(color(theme.fg))),
            ])
        })
        .chain(std::iter::once(Line::from(Span::styled(
            "Esc close",
            Style::default().fg(color(theme.fg_dim)),
        ))))
        .collect::<Vec<_>>();
    Paragraph::new(rows)
        .block(block)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        )
        .render(area, frame.buffer_mut());
}

fn style_palette_area(size: Rect) -> Rect {
    let width = bounded_popup_extent(size.width, 34, 52);
    let height = 7.min(size.height);
    Rect {
        x: size.x + size.width.saturating_sub(width + 4),
        y: size.y + (size.height / 3).saturating_sub(2),
        width,
        height,
    }
}

fn render_style_palette(frame: &mut Frame<'_>, size: Rect, theme: &Theme) {
    let area = style_palette_area(size);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Inline Style ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    let rows = vec![
        Line::from(vec![
            Span::styled("  B  ", style_button(theme, true, false, false, false)),
            Span::raw(" "),
            Span::styled("  I  ", style_button(theme, false, true, false, false)),
            Span::raw(" "),
            Span::styled("  S  ", style_button(theme, false, false, false, true)),
            Span::raw(" "),
            Span::styled(" </> ", style_button(theme, false, false, false, false)),
            Span::raw(" "),
            Span::styled(" 🔗 ", style_button(theme, false, false, true, false)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", swatch(theme.accent)),
            Span::raw(" "),
            Span::styled("  ", swatch(theme.yellow)),
            Span::raw(" "),
            Span::styled("  ", swatch(theme.green)),
            Span::raw(" "),
            Span::styled("  ", swatch(theme.blue)),
            Span::raw(" "),
            Span::styled("  ", swatch(theme.pink)),
        ]),
        Line::from(Span::styled(
            " Click buttons or press B/I/S/C/K   Esc close",
            Style::default().fg(color(theme.fg_dim)),
        )),
    ];
    Paragraph::new(rows)
        .block(block)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        )
        .render(area, frame.buffer_mut());
}

fn handle_style_palette_mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    app.mouse_selection_anchor = None;
    let MouseEventKind::Down(MouseButton::Left) = kind else {
        return;
    };
    let Some(size) = app.last_screen_area else {
        return;
    };
    let area = style_palette_area(size);
    if let Some(action) = style_palette_action_at(area, column, row) {
        apply_inline_style_action(app, action);
    } else if !rect_contains(area, column, row) {
        app.mode = Mode::Edit;
    }
}

fn click_selection_style_palette_at(app: &mut App, column: u16, row: u16) -> bool {
    if app.mode != Mode::Edit || app.current().document.selection_range().is_none() {
        return false;
    }
    let Some(size) = app.last_screen_area else {
        return false;
    };
    let area = style_palette_area(size);
    if let Some(action) = style_palette_action_at(area, column, row) {
        app.mouse_selection_anchor = None;
        apply_inline_style_action(app, action);
        return true;
    }
    rect_contains(area, column, row)
}

fn style_palette_action_at(area: Rect, column: u16, row: u16) -> Option<InlineStyleAction> {
    let inner = document_inner(area);
    if row != inner.y || column < inner.x || column >= inner.x.saturating_add(inner.width) {
        return None;
    }
    let buttons = [
        ("  B  ", InlineStyleAction::Bold),
        ("  I  ", InlineStyleAction::Italic),
        ("  S  ", InlineStyleAction::Strike),
        (" </> ", InlineStyleAction::Code),
        (" 🔗 ", InlineStyleAction::Link),
    ];
    let mut x = inner.x;
    for (label, action) in buttons {
        let width = UnicodeWidthStr::width(label) as u16;
        if column >= x && column < x.saturating_add(width) {
            return Some(action);
        }
        x = x.saturating_add(width).saturating_add(1);
    }
    None
}

fn apply_inline_style_action(app: &mut App, action: InlineStyleAction) {
    match action {
        InlineStyleAction::Bold => {
            let changed = app.current_mut().document.toggle_strong_at_cursor();
            set_message_if(app, changed, "bold toggled");
            app.mode = Mode::Edit;
        }
        InlineStyleAction::Italic => {
            let changed = app.current_mut().document.toggle_emphasis_at_cursor();
            set_message_if(app, changed, "italic toggled");
            app.mode = Mode::Edit;
        }
        InlineStyleAction::Strike => {
            let changed = app.current_mut().document.toggle_strikethrough_at_cursor();
            set_message_if(app, changed, "strikethrough toggled");
            app.mode = Mode::Edit;
        }
        InlineStyleAction::Code => {
            let changed = app.current_mut().document.toggle_inline_code_at_cursor();
            set_message_if(app, changed, "inline code toggled");
            app.mode = Mode::Edit;
        }
        InlineStyleAction::Link => open_link_prompt(app),
    }
}

fn insert_menu_entries() -> &'static [(char, &'static str, InsertBlockAction)] {
    &[
        ('p', "Paragraph", InsertBlockAction::Paragraph),
        ('h', "Heading", InsertBlockAction::Heading),
        ('c', "Code block", InsertBlockAction::CodeBlock),
        ('q', "Quote", InsertBlockAction::Quote),
        ('a', "Alert", InsertBlockAction::Alert),
        ('l', "List", InsertBlockAction::List),
        ('t', "Table", InsertBlockAction::Table),
        ('i', "Image", InsertBlockAction::Image),
        ('r', "Link reference", InsertBlockAction::LinkReference),
        ('m', "Math", InsertBlockAction::Math),
        ('d', "Diagram", InsertBlockAction::Diagram),
    ]
}

fn insert_action_for_key(key: char) -> Option<InsertBlockAction> {
    let key = key.to_ascii_lowercase();
    insert_menu_entries()
        .iter()
        .find_map(|(entry_key, _label, action)| (*entry_key == key).then_some(*action))
}

fn handle_insert_menu_mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    app.mouse_selection_anchor = None;
    let MouseEventKind::Down(MouseButton::Left) = kind else {
        return;
    };
    let Some(size) = app.last_screen_area else {
        return;
    };
    let area = insert_menu_area(size);
    if let Some(action) = insert_menu_action_at(area, column, row) {
        apply_insert_block_action(app, action);
    } else if !rect_contains(area, column, row) {
        app.mode = Mode::Edit;
    }
}

fn insert_menu_action_at(area: Rect, column: u16, row: u16) -> Option<InsertBlockAction> {
    let inner = document_inner(area);
    if column < inner.x
        || column >= inner.x.saturating_add(inner.width)
        || row < inner.y
        || row >= inner.y.saturating_add(inner.height)
    {
        return None;
    }
    let index = usize::from(row.saturating_sub(inner.y));
    insert_menu_entries()
        .get(index)
        .map(|(_key, _label, action)| *action)
}

fn apply_insert_block_action(app: &mut App, action: InsertBlockAction) {
    let (markdown, cursor_offset, label) = insert_block_template(action);
    app.current_mut()
        .document
        .insert_block_at_cursor(markdown, cursor_offset);
    app.mode = Mode::Edit;
    app.message = format!("inserted {label}");
    keep_cursor_visible(app, 20);
}

fn insert_block_template(action: InsertBlockAction) -> (&'static str, usize, &'static str) {
    match action {
        InsertBlockAction::Paragraph => ("Paragraph", 0, "paragraph"),
        InsertBlockAction::Heading => ("## Heading", "## ".len(), "heading"),
        InsertBlockAction::CodeBlock => ("```text\n\n```", "```text\n".len(), "code block"),
        InsertBlockAction::Quote => ("> Quote", "> ".len(), "quote"),
        InsertBlockAction::Alert => ("> [!NOTE]\n> Note", "> [!NOTE]\n> ".len(), "alert"),
        InsertBlockAction::List => ("- Item", "- ".len(), "list"),
        InsertBlockAction::Table => (
            "| Header | Value |\n| --- | --- |\n| Cell | Cell |",
            "| ".len(),
            "table",
        ),
        InsertBlockAction::Image => ("![Alt text](path/to/image.png)", "![".len(), "image"),
        InsertBlockAction::LinkReference => {
            ("[label]: https://example.com", "[".len(), "link reference")
        }
        InsertBlockAction::Math => ("$$\n\n$$", "$$\n".len(), "math"),
        InsertBlockAction::Diagram => (
            "```mermaid\ngraph TD\n    A[Start] --> B[End]\n```",
            "```mermaid\n".len(),
            "diagram",
        ),
    }
}

fn render_command_palette(frame: &mut Frame<'_>, size: Rect, app: &App, theme: &Theme) {
    let entries = filtered_palette_entries(app);
    let visible = entries.len().min(7);
    let width = bounded_popup_extent(size.width, 44, 76);
    let height = (visible as u16).saturating_add(4).min(size.height);
    let area = Rect {
        x: size.x + (size.width.saturating_sub(width)) / 2,
        y: size.y + size.height.saturating_sub(height) / 3,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Command Palette ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    let selected = app.command_selection.min(entries.len().saturating_sub(1));
    let mut rows = vec![Line::from(vec![
        Span::styled(": ", Style::default().fg(color(theme.accent))),
        Span::styled(
            app.command_input.as_str(),
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        ),
    ])];
    if entries.is_empty() {
        rows.push(Line::from(Span::styled(
            "  no matching commands",
            Style::default().fg(color(theme.fg_dim)),
        )));
    } else {
        for (idx, entry) in entries.iter().take(visible).enumerate() {
            let active = idx == selected;
            let row_style = if active {
                Style::default()
                    .fg(color(theme.bg))
                    .bg(color(theme.accent))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(color(theme.fg))
                    .bg(color(theme.bg_raised))
            };
            rows.push(Line::from(vec![
                Span::styled(if active { " > " } else { "   " }, row_style),
                Span::styled(entry.label, row_style),
                Span::raw("  "),
                Span::styled(entry.hint, Style::default().fg(color(theme.fg_dim))),
            ]));
        }
    }
    rows.push(Line::from(Span::styled(
        " Up/Down select   Enter run   Esc close",
        Style::default().fg(color(theme.fg_dim)),
    )));
    Paragraph::new(rows)
        .block(block)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        )
        .render(area, frame.buffer_mut());
}

fn style_button(theme: &Theme, bold: bool, italic: bool, underlined: bool, struck: bool) -> Style {
    let mut modifier = Modifier::empty();
    if bold {
        modifier |= Modifier::BOLD;
    }
    if italic {
        modifier |= Modifier::ITALIC;
    }
    if underlined {
        modifier |= Modifier::UNDERLINED;
    }
    if struck {
        modifier |= Modifier::CROSSED_OUT;
    }
    Style::default()
        .fg(color(theme.fg))
        .bg(color(theme.bg_soft))
        .add_modifier(modifier)
}

fn swatch(rgb: Rgb) -> Style {
    Style::default().bg(color(rgb))
}

fn render_prompt(frame: &mut Frame<'_>, size: Rect, prefix: &str, input: &str, theme: &Theme) {
    let width = bounded_popup_extent(size.width, 30, 72);
    let area = Rect {
        x: size.x + (size.width.saturating_sub(width)) / 2,
        y: size.y + 2,
        width,
        height: 3,
    };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color(theme.accent)))
        .style(Style::default().bg(color(theme.bg_raised)));
    Paragraph::new(format!("{prefix}{input}"))
        .block(block)
        .style(
            Style::default()
                .fg(color(theme.fg))
                .bg(color(theme.bg_raised)),
        )
        .render(area, frame.buffer_mut());
}

fn set_cursor(frame: &mut Frame<'_>, area: Rect, app: &App, rendered: &RenderedDocument) {
    if app.mode != Mode::Edit {
        return;
    }
    let inner = document_text_area(area);
    let Some((x, y_doc)) = rendered.cursor else {
        return;
    };
    if y_doc < app.current().scroll_y {
        return;
    }
    let y = y_doc - app.current().scroll_y;
    if y >= usize::from(inner.height) {
        return;
    }
    frame.set_cursor_position(Position {
        x: inner.x + x.min(inner.width.saturating_sub(1)),
        y: inner.y + y as u16,
    });
}

fn keep_cursor_visible(app: &mut App, viewport_guess: usize) {
    let width = 80;
    let rendered = render_document(&app.current().document, width);
    if let Some((_, y)) = rendered.cursor {
        let scroll = app.current().scroll_y;
        if y < scroll {
            app.current_mut().scroll_y = y;
        } else if y >= scroll.saturating_add(viewport_guess) {
            app.current_mut().scroll_y = y.saturating_sub(viewport_guess.saturating_sub(1));
        }
    }
}

fn click_tab_at(app: &mut App, column: u16, row: u16) -> bool {
    let Some(index) = tab_at(app, column, row) else {
        return false;
    };
    app.active = index;
    true
}

fn close_tab_at(app: &mut App, column: u16, row: u16) -> bool {
    let Some(index) = tab_at(app, column, row) else {
        return false;
    };
    app.active = index;
    app.close_current();
    true
}

fn tab_at(app: &App, column: u16, row: u16) -> Option<usize> {
    let area = app.last_tab_area?;
    let labels = app
        .tabs
        .iter()
        .map(|tab| {
            if tab.document.dirty.is_dirty {
                format!("{} *", tab.title)
            } else {
                tab.title.clone()
            }
        })
        .collect::<Vec<_>>();
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let target = tabbar::hit_test(area, refs.as_slice(), column, row);
    if let Some(index) = target {
        return (index < app.tabs.len()).then_some(index);
    }
    None
}

fn next_tab(app: &mut App) {
    if !app.tabs.is_empty() {
        app.active = (app.active + 1) % app.tabs.len();
    }
}

fn previous_tab(app: &mut App) {
    if !app.tabs.is_empty() {
        app.active = app.active.checked_sub(1).unwrap_or(app.tabs.len() - 1);
    }
}

fn restore_session() -> Option<Session> {
    restore_session_from(&session_path())
}

fn restore_session_from(path: &Path) -> Option<Session> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn persist_session(session: &Session) -> Result<()> {
    persist_session_to(&session_path(), session)
}

fn persist_session_to(path: &Path, session: &Session) -> Result<()> {
    let text = toml::to_string_pretty(session)?;
    fs::write(path, text)?;
    Ok(())
}

fn session_path() -> PathBuf {
    PathBuf::from(SESSION_FILE)
}

fn tabs_from_session(session: &Session) -> Vec<Tab> {
    session
        .tabs
        .iter()
        .filter_map(|saved| {
            let mut tab = load_tab(&saved.path).ok()?;
            tab.scroll_y = saved.scroll_y;
            tab.document
                .set_cursor_byte(saved.cursor_source_byte.min(tab.document.len_bytes()));
            Some(tab)
        })
        .collect()
}

fn current_component_kind(app: &App) -> Option<ComponentKind> {
    let document = &app.current().document;
    document
        .components
        .component_at_byte(document.cursor.byte)
        .map(|component| component.kind.clone())
}

fn is_task_context(app: &App) -> bool {
    matches!(
        current_component_kind(app),
        Some(ComponentKind::ListItem { checked: Some(_) })
    )
}

fn is_table_context(app: &App) -> bool {
    matches!(current_component_kind(app), Some(ComponentKind::Table))
}

fn is_gap_context(app: &App) -> bool {
    matches!(current_component_kind(app), Some(ComponentKind::Gap))
}

fn is_code_context(app: &App) -> bool {
    matches!(
        current_component_kind(app),
        Some(ComponentKind::CodeBlock { .. } | ComponentKind::DiagramBlock { .. })
    )
}

fn is_code_copy_focus(app: &App) -> bool {
    app.code_focus == CodeFocus::CopyButton && is_code_context(app)
}

fn is_footnote_context(app: &App) -> bool {
    app.current().document.footnote_label_at_cursor().is_some()
}

fn select_table_row(app: &mut App) {
    if let Some((row, _col)) = app.current().document.table_cell_at_cursor() {
        app.table_selection = Some(TableSelection::Row(row));
        app.message = format!("table row {} selected; Delete removes it", row + 1);
    } else {
        app.table_selection = None;
    }
}

fn select_table_column(app: &mut App) {
    if let Some((_row, col)) = app.current().document.table_cell_at_cursor() {
        app.table_selection = Some(TableSelection::Column(col));
        app.message = format!("table column {} selected; Delete removes it", col + 1);
    } else {
        app.table_selection = None;
    }
}

fn delete_table_selection(app: &mut App) {
    let Some(selection) = app.table_selection.take() else {
        return;
    };
    let changed = match selection {
        TableSelection::Row(row) => app.current_mut().document.remove_table_row(row),
        TableSelection::Column(col) => app.current_mut().document.remove_table_column(col),
    };
    app.message = if changed {
        match selection {
            TableSelection::Row(_) => "selected table row removed".to_string(),
            TableSelection::Column(_) => "selected table column removed".to_string(),
        }
    } else {
        "table selection could not be removed".to_string()
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DocumentHit {
    byte: usize,
    x: u16,
}

fn click_document_at(app: &mut App, column: u16, row: u16) -> bool {
    if app.mode != Mode::Edit {
        return false;
    }
    let Some(hit) = document_hit_at(app, column, row) else {
        return false;
    };
    app.table_selection = None;
    app.code_focus = CodeFocus::None;
    app.mouse_selection_anchor = Some(hit.byte);
    app.current_mut().document.set_cursor_byte(hit.byte);
    if hit.x <= 3 && is_task_context(app) {
        let changed = app.current_mut().document.toggle_task_at_cursor();
        set_message_if(app, changed, "task toggled");
    } else {
        app.message = "cursor moved".to_string();
    }
    true
}

fn drag_document_at(app: &mut App, column: u16, row: u16) -> bool {
    if app.mode != Mode::Edit {
        return false;
    }
    let Some(hit) = document_hit_at(app, column, row) else {
        return false;
    };
    let anchor = app
        .mouse_selection_anchor
        .unwrap_or(app.current().document.cursor.byte);
    app.table_selection = None;
    app.code_focus = CodeFocus::None;
    app.current_mut().document.set_selection(anchor, hit.byte);
    keep_cursor_visible(app, 20);
    app.message = if app.current().document.selection_range().is_some() {
        "selection active".to_string()
    } else {
        "cursor moved".to_string()
    };
    true
}

fn document_hit_at(app: &App, column: u16, row: u16) -> Option<DocumentHit> {
    let area = app.last_content_area?;
    let inner = document_text_area(area);
    if !rect_contains(inner, column, row) {
        return None;
    }
    let rendered = app.last_rendered.as_ref()?;
    let x = column.saturating_sub(inner.x);
    let y_doc = app
        .current()
        .scroll_y
        .saturating_add(usize::from(row.saturating_sub(inner.y)));
    let byte = rendered_byte_at(rendered, x, y_doc)?;
    Some(DocumentHit { byte, x })
}

fn click_scrollbar_at(app: &mut App, column: u16, row: u16) -> bool {
    let Some(area) = app.last_document_area else {
        return false;
    };
    let Some(rendered) = app.last_rendered.as_ref() else {
        return false;
    };
    if area.width == 0
        || area.height == 0
        || column != area.x.saturating_add(area.width.saturating_sub(1))
        || row < area.y
        || row >= area.y.saturating_add(area.height)
        || rendered.total_rows <= usize::from(area.height)
    {
        return false;
    }
    let max_scroll = rendered
        .total_rows
        .saturating_sub(usize::from(area.height))
        .max(1);
    let row_offset = usize::from(row.saturating_sub(area.y));
    app.mouse_selection_anchor = None;
    app.current_mut().scroll_y = (row_offset * max_scroll) / usize::from(area.height.max(1));
    app.message = format!("scroll {}", app.current().scroll_y);
    true
}

fn rendered_byte_at(rendered: &RenderedDocument, x: u16, y_doc: usize) -> Option<usize> {
    let line = rendered.lines.iter().find(|line| line.y_doc == y_doc)?;
    line.source_spans
        .iter()
        .find(|span| x >= span.x && x < span.x.saturating_add(span.width.max(1)))
        .map(|span| {
            let width = usize::from(span.width.max(1));
            let offset = usize::from(x.saturating_sub(span.x)).min(width.saturating_sub(1));
            let source_len = span.source.end.saturating_sub(span.source.start);
            span.source
                .start
                .saturating_add((source_len.saturating_mul(offset)) / width)
        })
}

fn document_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn document_panel_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn document_text_area(area: Rect) -> Rect {
    let inner = document_panel_inner(area);
    Rect {
        x: inner.x.saturating_add(DOCUMENT_X_PADDING),
        y: inner.y,
        width: inner
            .width
            .saturating_sub(DOCUMENT_X_PADDING.saturating_mul(2)),
        height: inner.height,
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn paste_text(app: &mut App, text: &str) -> bool {
    if is_code_copy_focus(app) {
        app.message = "copy button focused; Enter copies code body".to_string();
        return true;
    }
    app.table_selection = None;
    app.code_focus = code_focus_after_text_edit(app);
    app.current_mut().document.insert_text(text);
    true
}

fn code_focus_after_text_edit(app: &App) -> CodeFocus {
    match app.code_focus {
        CodeFocus::Language | CodeFocus::Body if is_code_context(app) => app.code_focus,
        _ => CodeFocus::None,
    }
}

fn cycle_code_focus(app: &mut App, forward: bool) {
    if !is_code_context(app) {
        app.code_focus = CodeFocus::None;
        return;
    }
    let next = match (app.code_focus, forward) {
        (CodeFocus::None, true) => CodeFocus::Language,
        (CodeFocus::Language, true) => CodeFocus::Body,
        (CodeFocus::Body, true) => CodeFocus::CopyButton,
        (CodeFocus::CopyButton, true) => CodeFocus::Language,
        (CodeFocus::None, false) => CodeFocus::CopyButton,
        (CodeFocus::Language, false) => CodeFocus::CopyButton,
        (CodeFocus::Body, false) => CodeFocus::Language,
        (CodeFocus::CopyButton, false) => CodeFocus::Body,
    };
    let focused = match next {
        CodeFocus::Language => app.current_mut().document.focus_code_language_at_cursor(),
        CodeFocus::Body => app.current_mut().document.focus_code_body_at_cursor(false),
        CodeFocus::CopyButton => app.current_mut().document.focus_code_body_at_cursor(true),
        CodeFocus::None => false,
    };
    if focused {
        app.code_focus = next;
        app.message = match next {
            CodeFocus::Language => "code language field focused; type or Ctrl-L edits".to_string(),
            CodeFocus::Body => "code body field focused".to_string(),
            CodeFocus::CopyButton => "code copy button focused; Enter copies body".to_string(),
            CodeFocus::None => String::new(),
        };
    } else {
        app.code_focus = CodeFocus::None;
    }
}

fn queue_code_body_copy(app: &mut App) {
    if let Some(body) = app.current().document.code_body_at_cursor() {
        let len = body.len();
        app.clipboard = Some(body);
        app.message = format!("queued code body clipboard copy ({len} bytes)");
    } else {
        app.message = "cursor is not on a code block".to_string();
    }
}

fn search_current(app: &mut App, backwards: bool) {
    let query = app.search_input.clone();
    if query.is_empty() {
        app.message = "search query is empty".to_string();
        return;
    }
    let source = app.current().document.source();
    let matches = source
        .match_indices(&query)
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        app.message = format!("no search matches: {query}");
        return;
    }

    let cursor = app.current().document.cursor.byte;
    let target_index = if backwards {
        matches
            .iter()
            .rposition(|byte| *byte < cursor)
            .unwrap_or_else(|| matches.len().saturating_sub(1))
    } else {
        matches.iter().position(|byte| *byte > cursor).unwrap_or(0)
    };
    let target = matches[target_index];
    app.current_mut().document.set_cursor_byte(target);
    keep_cursor_visible(app, 6);
    app.message = format!(
        "search {}/{}: {}",
        target_index.saturating_add(1),
        matches.len(),
        query
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteCommand {
    Save,
    Open,
    WriteAs,
    New,
    CloseTab,
    NextTab,
    PreviousTab,
    Search,
    InsertLink,
    EditComponent,
    NormalizeTable,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PaletteEntry {
    command: PaletteCommand,
    label: &'static str,
    hint: &'static str,
}

fn execute_command_palette(app: &mut App) {
    if should_execute_legacy_command(&app.command_input) {
        app.execute_command();
        return;
    }
    let entries = filtered_palette_entries(app);
    let Some(entry) = entries.get(app.command_selection.min(entries.len().saturating_sub(1)))
    else {
        app.message = "no command matches".to_string();
        return;
    };
    let command = entry.command;
    app.command_input.clear();
    app.command_selection = 0;
    app.mode = Mode::Edit;
    match command {
        PaletteCommand::Save => app.save_current(),
        PaletteCommand::Open => {
            app.command_input = "open ".to_string();
            app.mode = Mode::Command;
        }
        PaletteCommand::WriteAs => {
            app.command_input = "write ".to_string();
            app.mode = Mode::Command;
        }
        PaletteCommand::New => {
            app.tabs.push(Tab {
                title: "untitled.md".to_string(),
                path: None,
                document: import_gfm("").document,
                scroll_y: 0,
            });
            app.active = app.tabs.len().saturating_sub(1);
            app.message = "new tab".to_string();
        }
        PaletteCommand::CloseTab => app.close_current(),
        PaletteCommand::NextTab => next_tab(app),
        PaletteCommand::PreviousTab => previous_tab(app),
        PaletteCommand::Search => app.mode = Mode::Search,
        PaletteCommand::InsertLink => open_link_prompt(app),
        PaletteCommand::EditComponent => open_component_popup(app),
        PaletteCommand::NormalizeTable => {
            let changed = app.current_mut().document.normalize_table_at_cursor();
            set_message_if(app, changed, "table normalized");
        }
        PaletteCommand::Quit => app.request_quit(),
    }
}

fn should_execute_legacy_command(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed.contains(char::is_whitespace)
        || matches!(trimmed, "q" | "quit" | "w" | "write" | "o" | "open" | "new")
}

fn move_palette_selection(app: &mut App, forward: bool) {
    let len = filtered_palette_entries(app).len();
    if len == 0 {
        app.command_selection = 0;
    } else if forward {
        app.command_selection = (app.command_selection + 1) % len;
    } else {
        app.command_selection = app.command_selection.checked_sub(1).unwrap_or(len - 1);
    }
}

fn filtered_palette_entries(app: &App) -> Vec<PaletteEntry> {
    let query = app.command_input.trim();
    let mut entries = palette_entries(app)
        .into_iter()
        .filter(|entry| query.is_empty() || palette_match_score(entry, query).is_some())
        .collect::<Vec<_>>();
    if !query.is_empty() {
        entries.sort_by_key(|entry| palette_match_score(entry, query).unwrap_or(usize::MAX))
    }
    entries
}

fn palette_entries(app: &App) -> Vec<PaletteEntry> {
    let mut entries = vec![
        PaletteEntry {
            command: PaletteCommand::Save,
            label: "Save Document",
            hint: "Ctrl-S",
        },
        PaletteEntry {
            command: PaletteCommand::Open,
            label: "Open File...",
            hint: ":open path",
        },
        PaletteEntry {
            command: PaletteCommand::WriteAs,
            label: "Write As...",
            hint: ":write path",
        },
        PaletteEntry {
            command: PaletteCommand::New,
            label: "New Tab",
            hint: "Ctrl-N",
        },
        PaletteEntry {
            command: PaletteCommand::CloseTab,
            label: "Close Tab",
            hint: "middle click tab",
        },
        PaletteEntry {
            command: PaletteCommand::NextTab,
            label: "Next Tab",
            hint: "Ctrl-Tab",
        },
        PaletteEntry {
            command: PaletteCommand::PreviousTab,
            label: "Previous Tab",
            hint: "Ctrl-Shift-Tab",
        },
        PaletteEntry {
            command: PaletteCommand::Search,
            label: "Search Document",
            hint: "Ctrl-F or /",
        },
        PaletteEntry {
            command: PaletteCommand::InsertLink,
            label: "Insert or Edit Link",
            hint: "Ctrl-K",
        },
        PaletteEntry {
            command: PaletteCommand::EditComponent,
            label: "Edit Focused Component",
            hint: "F2 or e",
        },
        PaletteEntry {
            command: PaletteCommand::Quit,
            label: "Quit",
            hint: "Ctrl-Q",
        },
    ];
    if is_table_context(app) {
        entries.push(PaletteEntry {
            command: PaletteCommand::NormalizeTable,
            label: "Normalize Focused Table",
            hint: "table action",
        });
    }
    entries
}

fn fuzzy_match(value: &str, query: &str) -> bool {
    let mut chars = value.chars().flat_map(char::to_lowercase);
    for needle in query.chars().flat_map(char::to_lowercase) {
        if needle.is_whitespace() {
            continue;
        }
        if !chars.any(|ch| ch == needle) {
            return false;
        }
    }
    true
}

fn palette_match_score(entry: &PaletteEntry, query: &str) -> Option<usize> {
    let query = query.to_ascii_lowercase();
    let label = entry.label.to_ascii_lowercase();
    let hint = entry.hint.to_ascii_lowercase();
    if label.contains(&query) {
        Some(0)
    } else if hint.contains(&query) {
        Some(1)
    } else if fuzzy_match(entry.label, &query) || fuzzy_match(entry.hint, &query) {
        Some(2)
    } else {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LinkPromptFields {
    label: String,
    destination: String,
    title: Option<String>,
    reference_label: Option<String>,
    rich: bool,
}

fn parse_link_prompt(input: &str) -> LinkPromptFields {
    if !input.contains('|') {
        return LinkPromptFields {
            destination: input.trim().to_string(),
            ..LinkPromptFields::default()
        };
    }
    let mut parts = input.splitn(4, '|');
    let label = parts.next().unwrap_or_default().trim().to_string();
    let destination = parts.next().unwrap_or_default().trim().to_string();
    let title = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let reference_label = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    LinkPromptFields {
        label,
        destination,
        title,
        reference_label,
        rich: true,
    }
}

fn open_link_prompt(app: &mut App) {
    app.link_input = app
        .current()
        .document
        .link_at_cursor()
        .map(|link| {
            format!(
                "{}|{}|{}|{}",
                link.label,
                link.destination,
                link.title.unwrap_or_default(),
                link.reference_label.unwrap_or_default()
            )
        })
        .unwrap_or_default();
    app.mode = Mode::LinkPrompt;
}

fn open_component_popup(app: &mut App) {
    if let Some(label) = app.current().document.footnote_label_at_cursor() {
        app.footnote_label_input = label;
        app.mode = Mode::FootnoteLabelPrompt;
        return;
    }
    match current_component_kind(app) {
        Some(ComponentKind::CodeBlock { language, .. }) => {
            app.code_language_input = language;
            app.mode = Mode::CodeLanguagePrompt;
        }
        Some(ComponentKind::DiagramBlock { language }) => {
            app.code_language_input = format!("{language:?}").to_ascii_lowercase();
            app.mode = Mode::CodeLanguagePrompt;
        }
        Some(ComponentKind::Table) => app.mode = Mode::TableActions,
        Some(ComponentKind::Image) => {
            if let Some(image) = app.current().document.image_at_cursor() {
                app.image_input = format!(
                    "{}|{}|{}",
                    image.alt,
                    image.source,
                    image.title.unwrap_or_default()
                );
            }
            app.mode = Mode::ImagePrompt;
        }
        Some(ComponentKind::Heading { .. }) => {
            if let Some(heading) = app.current().document.heading_at_cursor() {
                app.heading_input = format!("{}|{}", heading.level, heading.text);
            }
            app.mode = Mode::HeadingPrompt;
        }
        Some(ComponentKind::Paragraph) => open_link_prompt(app),
        _ => {
            app.message = "no component popup for this target".to_string();
        }
    }
}

fn set_message_if(app: &mut App, changed: bool, message: &str) {
    if changed {
        app.message = message.to_string();
    }
}

fn parse_heading_prompt(input: &str) -> (u8, String) {
    let mut parts = input.splitn(2, '|');
    let level = parts
        .next()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(1)
        .clamp(1, 6);
    let text = parts.next().unwrap_or_default().trim().to_string();
    (level, text)
}

fn heading_level_key(ch: char) -> Option<u8> {
    match ch {
        '1'..='6' => Some(ch as u8 - b'0'),
        _ => None,
    }
}

fn parse_image_prompt(input: &str) -> (String, String, Option<String>) {
    let mut parts = input.splitn(3, '|');
    let alt = parts.next().unwrap_or_default().trim().to_string();
    let src = parts.next().unwrap_or_default().trim().to_string();
    let title = parts
        .next()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    (alt, src, title)
}

fn load_tab(path: &Path) -> Result<Tab> {
    let source = fs::read_to_string(path)?;
    let mut import = import_gfm(&source);
    import.document.path = Some(path.to_path_buf());
    Ok(Tab {
        title: title_for_path(path),
        path: Some(path.to_path_buf()),
        document: import.document,
        scroll_y: 0,
    })
}

fn title_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn component_label(kind: &ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Paragraph => "Paragraph",
        ComponentKind::Heading { .. } => "Heading",
        ComponentKind::ThematicBreak => "Rule",
        ComponentKind::BlockQuote => "Quote",
        ComponentKind::Alert { .. } => "Alert",
        ComponentKind::List { .. } => "List",
        ComponentKind::ListItem { .. } => "List item",
        ComponentKind::CodeBlock { .. } => "Code",
        ComponentKind::Table => "Table",
        ComponentKind::TableRow { .. } => "Table row",
        ComponentKind::TableCell { .. } => "Table cell",
        ComponentKind::Link { .. } => "Link",
        ComponentKind::Image => "Image",
        ComponentKind::FootnoteRef => "Footnote ref",
        ComponentKind::FootnoteDef => "Footnote",
        ComponentKind::HtmlBlock => "HTML",
        ComponentKind::MathBlock => "Math",
        ComponentKind::DiagramBlock { .. } => "Diagram",
        ComponentKind::Gap => "Gap",
    }
}

fn code_focus_label(app: &App) -> Option<&'static str> {
    if !is_code_context(app) {
        return None;
    }
    match app.code_focus {
        CodeFocus::Language => Some("Code language"),
        CodeFocus::Body => Some("Code body"),
        CodeFocus::CopyButton => Some("Code copy"),
        CodeFocus::None => None,
    }
}

fn table_selection_label(app: &App) -> Option<&'static str> {
    if !is_table_context(app) {
        return None;
    }
    match app.table_selection {
        Some(TableSelection::Row(_)) => Some("Table row selected"),
        Some(TableSelection::Column(_)) => Some("Table column selected"),
        None => None,
    }
}

fn centered_rect(size: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: size.x + size.width.saturating_sub(width) / 2,
        y: size.y + size.height.saturating_sub(height) / 2,
        width: width.min(size.width),
        height: height.min(size.height),
    }
}

fn bounded_popup_extent(value: u16, preferred_min: u16, max: u16) -> u16 {
    if value < preferred_min {
        value
    } else {
        value.min(max)
    }
}

fn style_for(style: CellStyle, theme: &Theme) -> Style {
    let fg = style.fg.unwrap_or(match style.token {
        Token::Normal => theme.fg,
        Token::Muted => theme.fg_dim,
        Token::Faint => theme.fg_faint,
        Token::Accent => theme.accent,
        Token::Link => theme.blue,
        Token::Code => theme.teal,
        Token::Border => theme.line,
        Token::Quote => theme.accent,
        Token::Error => theme.red,
        Token::Warn => theme.yellow,
        Token::Success => theme.green,
        Token::Blue => theme.blue,
        Token::Purple => theme.purple,
        Token::Pink => theme.pink,
    });
    let mut modifier = Modifier::empty();
    if style.bold {
        modifier |= Modifier::BOLD;
    }
    if style.italic {
        modifier |= Modifier::ITALIC;
    }
    if style.underlined {
        modifier |= Modifier::UNDERLINED;
    }
    if style.struck {
        modifier |= Modifier::CROSSED_OUT;
    }
    if style.reversed {
        modifier |= Modifier::REVERSED;
    }
    let bg = match style.bg {
        Some(CellBg::Soft) => theme.bg_soft,
        Some(CellBg::Raised) => theme.bg_raised,
        None => theme.bg,
    };
    Style::default()
        .fg(color(fg))
        .bg(color(bg))
        .add_modifier(modifier)
}

fn color(rgb: Rgb) -> Color {
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

    use super::*;

    #[test]
    fn typing_edits_underlying_markdown() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.source(), "a");
    }

    #[test]
    fn command_write_without_path_reports_helpful_message() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.command_input = "write".to_string();
        app.execute_command();
        assert!(app.message.contains("no path"));
    }

    #[test]
    fn command_palette_fuzzy_runs_selected_command_and_preserves_typed_commands() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.mode = Mode::Command;
        app.command_input = "sea".to_string();

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::Search);

        app.mode = Mode::Command;
        app.command_input = "write".to_string();
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::Edit);
        assert!(app.message.contains("no path"));
    }

    #[test]
    fn mouse_click_moves_cursor_and_toggles_task_checkbox() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("- [ ] task\n").document;
        install_mouse_projection(&mut app, 0);

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 2,
                modifiers: KeyModifiers::empty(),
            }),
        );

        assert_eq!(app.current().document.source(), "- [x] task\n");
        assert!(app.message.contains("task toggled"));
    }

    #[test]
    fn mouse_drag_creates_source_backed_selection() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        install_mouse_projection(&mut app, 0);

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 2,
                modifiers: KeyModifiers::empty(),
            }),
        );
        assert_eq!(app.mouse_selection_anchor, Some(0));

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 9,
                row: 2,
                modifiers: KeyModifiers::empty(),
            }),
        );

        let selection = app
            .current()
            .document
            .selection_range()
            .unwrap_or_else(|| panic!("missing drag selection"));
        assert_eq!(selection.start, 0);
        assert!(selection.end > selection.start);
        assert!(app.message.contains("selection active"));

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 9,
                row: 2,
                modifiers: KeyModifiers::empty(),
            }),
        );
        assert_eq!(app.mouse_selection_anchor, None);
        assert!(app.current().document.selection_range().is_some());
    }

    #[test]
    fn render_line_highlights_only_selected_cells() {
        let theme = Theme::ghostty_default_dark();
        let area = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        };
        let mut buffer = Buffer::empty(area);
        let line = RenderLine {
            y_doc: 0,
            cells: "alpha beta"
                .chars()
                .map(|ch| mdtui_render::StyledCell {
                    text: ch.to_string(),
                    style: CellStyle::default(),
                })
                .collect(),
            source_spans: vec![SourceCellSpan {
                x: 0,
                width: 10,
                source: mdtui_core::SourceRange { start: 0, end: 10 },
            }],
            hit_zones: Vec::new(),
        };

        render_line(&mut buffer, area, 0, &line, Some(&(6..10)), &theme);

        assert!(
            !buffer
                .cell((0, 0))
                .unwrap_or_else(|| panic!("missing unselected cell"))
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            buffer
                .cell((6, 0))
                .unwrap_or_else(|| panic!("missing selected cell"))
                .modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn style_palette_mouse_click_dispatches_inline_action() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        app.current_mut().document.set_cursor_byte(7);
        app.mode = Mode::StylePalette;
        let screen = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        app.last_screen_area = Some(screen);
        let inner = document_inner(style_palette_area(screen));

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: inner.x + 1,
                row: inner.y,
                modifiers: KeyModifiers::empty(),
            }),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "alpha **beta**");
        assert!(app.message.contains("bold toggled"));
    }

    #[test]
    fn selection_toolbar_mouse_click_styles_selected_text() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        app.current_mut().document.set_selection(6, 10);
        let screen = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        app.last_screen_area = Some(screen);
        let inner = document_inner(style_palette_area(screen));

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: inner.x + 1,
                row: inner.y,
                modifiers: KeyModifiers::empty(),
            }),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "alpha **beta**");
    }

    #[test]
    fn alt_enter_on_gap_opens_insert_menu_and_inserts_heading() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("").document;
        app.current_mut().document.set_cursor_byte(0);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        );
        assert_eq!(app.mode, Mode::InsertMenu);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty())),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "## Heading\n");
        assert_eq!(app.current().document.cursor.byte, "## ".len());
        assert!(app.message.contains("inserted heading"));
    }

    #[test]
    fn insert_menu_mouse_click_inserts_table_template() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("").document;
        app.current_mut().document.set_cursor_byte(0);
        app.mode = Mode::InsertMenu;
        let screen = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        app.last_screen_area = Some(screen);
        let inner = document_inner(insert_menu_area(screen));
        let table_row = insert_menu_entries()
            .iter()
            .position(|(_, _, action)| *action == InsertBlockAction::Table)
            .unwrap_or_default() as u16;

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: inner.x,
                row: inner.y + table_row,
                modifiers: KeyModifiers::empty(),
            }),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert!(
            app.current()
                .document
                .source()
                .contains("| Header | Value |")
        );
        assert!(app.message.contains("inserted table"));
    }

    #[test]
    fn scrollbar_mouse_click_jumps_scroll_position() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document =
            import_gfm(&(0..80).map(|_| "---\n").collect::<String>()).document;
        install_mouse_projection(&mut app, 0);

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 20,
                row: 5,
                modifiers: KeyModifiers::empty(),
            }),
        );

        assert!(app.current().scroll_y > 0);
    }

    #[test]
    fn ctrl_shift_tab_switches_to_previous_tab() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.tabs.push(Tab {
            title: "second.md".to_string(),
            path: None,
            document: import_gfm("second").document,
            scroll_y: 0,
        });
        app.active = 0;

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(
                KeyCode::BackTab,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
        );

        assert_eq!(app.active, 1);
    }

    #[test]
    fn dirty_quit_requires_confirmation() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document.insert_text("unsaved");

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.mode, Mode::ConfirmQuit);
        assert!(!app.should_quit);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::Edit);
        assert!(!app.should_quit);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty())),
        );
        assert!(app.should_quit);
    }

    #[test]
    fn dirty_tab_close_requires_confirmation() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.tabs.push(Tab {
            title: "second.md".to_string(),
            path: None,
            document: import_gfm("second").document,
            scroll_y: 0,
        });
        app.active = 1;
        app.current_mut().document.insert_text(" dirty");

        app.close_current();
        assert_eq!(app.mode, Mode::ConfirmClose);
        assert_eq!(app.tabs.len(), 2);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.tabs.len(), 1);
    }

    #[test]
    fn shift_arrow_selection_replaces_typed_text_and_ctrl_a_selects_all() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha").document;
        app.current_mut().document.set_cursor_byte(0);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)),
        );
        assert_eq!(app.current().document.selection_range(), Some(0..1));
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.source(), "Alpha");
        assert_eq!(app.current().document.selection_range(), None);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        );
        assert_eq!(
            app.current().document.selection_range(),
            Some(0..app.current().document.len_bytes())
        );
    }

    #[test]
    fn search_prompt_jumps_next_and_previous_matches() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta\nbeta gamma\n").document;
        app.current_mut().document.set_cursor_byte(0);
        app.mode = Mode::Search;
        app.search_input = "beta".to_string();

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.cursor.byte, 6);
        assert!(app.message.contains("search 1/2"));

        app.mode = Mode::Search;
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.cursor.byte, 11);
        assert!(app.message.contains("search 2/2"));

        app.mode = Mode::Search;
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
        );
        assert_eq!(app.current().document.cursor.byte, 6);
        assert!(app.message.contains("search 1/2"));
    }

    #[test]
    fn uploads_heading_graphics_only_when_kgp_is_available() {
        let theme = Theme::ghostty_default_dark();
        let graphics = vec![RenderedGraphic {
            kind: RenderedGraphicKind::Heading {
                level: 1,
                text: "TITLE".to_string(),
            },
            image_id: 42,
            y_doc: 0,
            width_cells: 10,
            height_cells: 2,
            z_index: -10,
        }];
        let mut cache = KittyGraphicsCache::new();
        let mut out = Vec::new();
        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: false,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(out.is_empty());

        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: true,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let output = String::from_utf8_lossy(&out);
        assert!(output.contains("\x1b_Ga=t"));
        assert!(output.contains("\x1b_Ga=p,U=1,i=42,c=10,r=2"));
        let len_after_first_upload = out.len();

        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: true,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(out.len(), len_after_first_upload);
    }

    #[test]
    fn uploads_local_png_graphics_when_file_is_available() {
        let theme = Theme::ghostty_default_dark();
        let path =
            std::env::temp_dir().join(format!("mdtui-test-image-{}.png", std::process::id()));
        let png = [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 10, b'I', b'D',
            b'A', b'T', 0x78, 0x9c, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0d, 0x0a, 0x2d, 0xb4, 0, 0, 0, 0,
            b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
        ];
        fs::write(&path, png).unwrap_or_else(|error| panic!("{error}"));
        let graphics = vec![RenderedGraphic {
            kind: RenderedGraphicKind::LocalImage {
                alt: "preview".to_string(),
                source: "image.png".to_string(),
                path: path.clone(),
            },
            image_id: 43,
            y_doc: 0,
            width_cells: 12,
            height_cells: 4,
            z_index: -20,
        }];
        let mut cache = KittyGraphicsCache::new();
        let mut out = Vec::new();

        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: true,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let output = String::from_utf8_lossy(&out);
        assert!(output.contains("f=32"));
        assert!(output.contains("\x1b_Ga=p,U=1,i=43,c=12,r=4"));
        assert!(output.contains("z=-20"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn uploads_warm_preview_graphics_when_available() {
        let theme = Theme::ghostty_default_dark();
        let graphics = vec![RenderedGraphic {
            kind: RenderedGraphicKind::Preview {
                label: "math".to_string(),
                source_hash: 42,
            },
            image_id: 44,
            y_doc: 0,
            width_cells: 12,
            height_cells: 4,
            z_index: -20,
        }];
        let mut cache = KittyGraphicsCache::new();
        let mut out = Vec::new();

        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: true,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let output = String::from_utf8_lossy(&out);
        assert!(output.contains("f=32"));
        assert!(output.contains("\x1b_Ga=p,U=1,i=44,c=12,r=4"));
    }

    #[test]
    fn uploads_cached_external_preview_raster_before_placeholder_fallback() {
        mdtui_terminal::clear_external_preview_cache();
        let theme = Theme::ghostty_default_dark();
        let source_hash = 424_242;
        let theme_hash = graphics_theme_hash(&theme);
        mdtui_terminal::store_external_preview_raster(
            "math",
            source_hash,
            12,
            4,
            theme_hash,
            mdtui_terminal::RasterImage {
                width_px: 2,
                height_px: 1,
                rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
            },
        );
        let graphics = vec![RenderedGraphic {
            kind: RenderedGraphicKind::Preview {
                label: "math".to_string(),
                source_hash,
            },
            image_id: 45,
            y_doc: 0,
            width_cells: 12,
            height_cells: 4,
            z_index: -20,
        }];
        let mut cache = KittyGraphicsCache::new();
        let mut out = Vec::new();

        upload_rendered_graphics(
            &mut out,
            &TerminalCapabilities {
                kitty_graphics: true,
                terminal_program: None,
                term: None,
            },
            &mut cache,
            &graphics,
            &theme,
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let output = String::from_utf8_lossy(&out);
        assert!(output.contains("s=2,v=1,i=45"));
        assert!(output.contains("\x1b_Ga=p,U=1,i=45,c=12,r=4"));
        mdtui_terminal::clear_external_preview_cache();
    }

    #[test]
    fn space_toggles_task_checkbox_in_rendered_context() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("- [ ] task\n").document;
        app.current_mut().document.set_cursor_byte(4);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty())),
        );

        assert_eq!(app.current().document.source(), "- [x] task\n");
    }

    #[test]
    fn ctrl_k_prompt_links_current_word() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("visit site").document;
        app.current_mut().document.set_cursor_byte(7);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
        );
        for ch in "https://example.com".chars() {
            handle_event(
                &mut app,
                Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty())),
            );
        }
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert_eq!(
            app.current().document.source(),
            "visit [site](https://example.com)"
        );
    }

    #[test]
    fn f2_link_prompt_edits_rich_link_fields() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document =
            import_gfm("visit [site](https://old.example \"Old\")\n").document;
        app.current_mut().document.set_cursor_byte(8);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::LinkPrompt);
        assert_eq!(app.link_input, "site|https://old.example|Old|");

        app.link_input = "docs|https://new.example|New|ref".to_string();
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert_eq!(
            app.current().document.source(),
            "visit [docs][ref]\n\n[ref]: https://new.example \"New\"\n"
        );
    }

    #[test]
    fn ctrl_b_i_and_backtick_toggle_current_word_marks() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        app.current_mut().document.set_cursor_byte(7);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.current().document.source(), "alpha **beta**");
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.current().document.source(), "alpha _beta_");
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('`'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.current().document.source(), "alpha `beta`");
    }

    #[test]
    fn ctrl_e_style_palette_uses_existing_inline_actions() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        app.current_mut().document.set_cursor_byte(7);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.mode, Mode::StylePalette);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty())),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "alpha **beta**");
    }

    #[test]
    fn style_palette_toggles_strikethrough() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("alpha beta").document;
        app.current_mut().document.set_cursor_byte(7);
        app.mode = Mode::StylePalette;

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty())),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "alpha ~~beta~~");
    }

    #[test]
    fn footnote_keys_create_jump_and_rename() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("body[^one]\n").document;
        app.current_mut().document.set_cursor_byte(5);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        );
        assert!(app.current().document.source().contains("[^one]: "));

        app.current_mut().document.set_cursor_byte(5);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert!(app.message.contains("footnote definition"));

        app.current_mut().document.set_cursor_byte(5);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::FootnoteLabelPrompt);
        app.footnote_label_input.clear();
        for ch in "two".chars() {
            handle_event(
                &mut app,
                Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty())),
            );
        }
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert!(app.current().document.source().contains("body[^two]"));
    }

    #[test]
    fn image_f2_prompt_edits_alt_source_and_title() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("![old](missing.png)\n").document;
        app.current_mut().document.set_cursor_byte(2);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::ImagePrompt);
        app.image_input = "new|https://example.com/img.png|caption".to_string();
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert_eq!(
            app.current().document.source(),
            "![new](https://example.com/img.png \"caption\")\n"
        );
    }

    #[test]
    fn heading_f2_prompt_edits_level_and_text() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("# Old title\n").document;
        app.current_mut().document.set_cursor_byte(3);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        assert_eq!(app.mode, Mode::HeadingPrompt);
        assert_eq!(app.heading_input, "1|Old title");

        app.heading_input = "3|New title".to_string();
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.current().document.source(), "### New title\n");
        assert!(app.message.contains("heading updated"));
    }

    #[test]
    fn ctrl_alt_digit_changes_heading_level() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("# Title\n").document;
        app.current_mut().document.set_cursor_byte(3);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(
                KeyCode::Char('4'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            )),
        );

        assert_eq!(app.current().document.source(), "#### Title\n");
        assert!(app.message.contains("heading level changed"));
    }

    #[test]
    fn table_alt_arrows_edit_rows_and_columns() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("| A | B |\n| - | - |\n| 1 | 2 |\n").document;
        let cursor = app
            .current()
            .document
            .source()
            .find('1')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);
        assert!(is_table_context(&app));

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT)),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)),
        );

        let source = app.current().document.source();
        assert!(source.lines().count() > 3);
        assert!(
            source
                .lines()
                .next()
                .unwrap_or_default()
                .matches('|')
                .count()
                >= 4
        );
    }

    #[test]
    fn table_tab_enter_and_backspace_are_cell_aware() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("| A | B |\n| - | - |\n| 1 | 2 |\n").document;
        let cursor = app
            .current()
            .document
            .source()
            .find('1')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.table_cell_at_cursor(), Some((1, 1)));

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.table_cell_at_cursor(), Some((2, 1)));

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        );
        assert_eq!(app.current().document.table_cell_at_cursor(), Some((2, 0)));

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty())),
        );
        assert_eq!(app.current().document.source().lines().count(), 3);
    }

    #[test]
    fn ctrl_alt_arrows_change_table_alignment() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("| A | B |\n| --- | --- |\n| 1 | 2 |\n").document;
        let cursor = app
            .current()
            .document
            .source()
            .find('1')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(
                KeyCode::Right,
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            )),
        );

        assert!(app.current().document.source().contains(":--"));
        assert!(app.message.contains("alignment"));
    }

    #[test]
    fn table_actions_select_and_delete_row_or_column() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document =
            import_gfm("| A | B | C |\n| - | - | - |\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n").document;
        let cursor = app
            .current()
            .document
            .source()
            .find('2')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT)),
        );
        assert_eq!(app.table_selection, Some(TableSelection::Column(1)));
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty())),
        );
        assert!(!app.current().document.source().contains(" B "));
        assert!(!app.current().document.source().contains(" 2 "));

        let cursor = app
            .current()
            .document
            .source()
            .find('4')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        assert_eq!(app.table_selection, Some(TableSelection::Row(2)));
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty())),
        );
        assert!(!app.current().document.source().contains(" 4 "));
    }

    #[test]
    fn table_actions_normalize_focused_table() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document =
            import_gfm("before\n\n| A|B |\n|-|-|\n| 1|2|\n\nafter").document;
        let cursor = app
            .current()
            .document
            .source()
            .find('1')
            .unwrap_or_default();
        app.current_mut().document.set_cursor_byte(cursor);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty())),
        );
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty())),
        );

        assert!(app.current().document.source().contains("| A   | B   |"));
        assert!(app.message.contains("normalized"));
    }

    #[test]
    fn ctrl_l_prompt_edits_code_language() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("```rust\nfn main() {}\n```\n").document;
        app.current_mut().document.set_cursor_byte(5);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
        );
        for ch in "python".chars() {
            handle_event(
                &mut app,
                Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty())),
            );
        }
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert!(app.current().document.source().starts_with("```python\n"));
    }

    #[test]
    fn tab_cycles_code_language_body_and_copy_button() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("```rust\nfn main() {}\n```\n").document;
        app.current_mut().document.set_cursor_byte(5);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
        );
        assert_eq!(app.code_focus, CodeFocus::Language);
        assert_eq!(app.current().document.cursor.byte, "```".len());

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
        );
        assert_eq!(app.code_focus, CodeFocus::Body);
        assert_eq!(
            app.current().document.cursor.byte,
            app.current()
                .document
                .source()
                .find("fn main")
                .unwrap_or_default()
        );

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::empty())),
        );
        assert!(app.current().document.source().contains("\nXfn main"));

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
        );
        assert_eq!(app.code_focus, CodeFocus::CopyButton);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
        );

        assert_eq!(app.clipboard.as_deref(), Some("Xfn main() {}"));
    }

    #[test]
    fn ctrl_shift_c_queues_code_body_clipboard_copy() {
        let mut app = App::open_initial(None).unwrap_or_else(|error| panic!("{error}"));
        app.current_mut().document = import_gfm("```rust\nfn main() {}\n```\n").document;
        app.current_mut().document.set_cursor_byte(5);

        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
        );

        assert_eq!(app.clipboard.as_deref(), Some("fn main() {}"));
        assert!(app.message.contains("queued"));
    }

    #[test]
    fn session_persists_paths_cursor_and_scroll() {
        let base =
            std::env::temp_dir().join(format!("mdtui-session-test-{}.toml", std::process::id()));
        let session = Session {
            active: 1,
            tabs: vec![SessionTab {
                path: PathBuf::from("/tmp/example.md"),
                cursor_source_byte: 42,
                scroll_y: 7,
                pinned: false,
            }],
        };

        persist_session_to(&base, &session).unwrap_or_else(|error| panic!("{error}"));
        let restored = restore_session_from(&base).unwrap_or_else(|| panic!("missing session"));
        let _ = fs::remove_file(&base);

        assert_eq!(restored, session);
    }

    fn install_mouse_projection(app: &mut App, scroll_y: usize) {
        app.current_mut().scroll_y = scroll_y;
        let content_area = Rect {
            x: 0,
            y: 1,
            width: 20,
            height: 6,
        };
        app.last_content_area = Some(content_area);
        app.last_document_area = Some(Rect {
            x: 0,
            y: 1,
            width: 21,
            height: 6,
        });
        app.last_rendered = Some(render_document(
            &app.current().document,
            document_text_area(content_area).width,
        ));
    }
}
