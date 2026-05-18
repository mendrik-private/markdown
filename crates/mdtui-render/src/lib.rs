use std::{
    collections::{HashSet, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use mdtui_core::{
    AlertLevel, Component, ComponentKind, ComponentSlot, CursorTarget, Document, LayoutBlock,
    SourceRange,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const CODE_HIGHLIGHT_CACHE_CAP: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.width)
            && y >= self.y
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Rgb,
    pub bg_soft: Rgb,
    pub bg_raised: Rgb,
    pub line: Rgb,
    pub line_soft: Rgb,
    pub fg: Rgb,
    pub fg_dim: Rgb,
    pub fg_mute: Rgb,
    pub fg_faint: Rgb,
    pub accent: Rgb,
    pub red: Rgb,
    pub yellow: Rgb,
    pub green: Rgb,
    pub teal: Rgb,
    pub blue: Rgb,
    pub purple: Rgb,
    pub pink: Rgb,
}

impl Theme {
    pub fn ghostty_default_dark() -> Self {
        Self {
            bg: Rgb(0x1d, 0x1b, 0x1a),
            bg_soft: Rgb(0x24, 0x21, 0x1f),
            bg_raised: Rgb(0x2a, 0x26, 0x24),
            line: Rgb(0x3a, 0x33, 0x2f),
            line_soft: Rgb(0x2f, 0x2a, 0x27),
            fg: Rgb(0xe8, 0xdf, 0xd3),
            fg_dim: Rgb(0xa8, 0x9c, 0x8a),
            fg_mute: Rgb(0x6b, 0x64, 0x59),
            fg_faint: Rgb(0x4a, 0x45, 0x3e),
            accent: Rgb(0xd9, 0x9a, 0x5e),
            red: Rgb(0xe0, 0x6c, 0x75),
            yellow: Rgb(0xe5, 0xc0, 0x7b),
            green: Rgb(0xa3, 0xb5, 0x65),
            teal: Rgb(0x7c, 0xb7, 0xa8),
            blue: Rgb(0x82, 0xaa, 0xdc),
            purple: Rgb(0xc0, 0x8b, 0xc0),
            pink: Rgb(0xd8, 0x8a, 0xa0),
        }
    }

    pub fn bg_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.bg.0, self.bg.1, self.bg.2)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Token {
    #[default]
    Normal,
    Muted,
    Faint,
    Accent,
    Link,
    Code,
    Border,
    Quote,
    Error,
    Warn,
    Success,
    Blue,
    Purple,
    Pink,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub token: Token,
    pub fg: Option<Rgb>,
    pub bg: Option<CellBg>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub struck: bool,
    pub reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellBg {
    Soft,
    Raised,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledCell {
    pub text: String,
    pub style: CellStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCellSpan {
    pub x: u16,
    pub width: u16,
    pub source: SourceRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitZone {
    pub rect: Rect,
    pub target: CursorTarget,
    pub z: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderLine {
    pub y_doc: usize,
    pub cells: Vec<StyledCell>,
    pub source_spans: Vec<SourceCellSpan>,
    pub hit_zones: Vec<HitZone>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDocument {
    pub lines: Vec<RenderLine>,
    pub graphics: Vec<RenderedGraphic>,
    pub pending_previews: Vec<PreviewRequest>,
    pub total_rows: usize,
    pub cursor: Option<(u16, usize)>,
    pub skipped_components: usize,
    pub block_index: Vec<BlockLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderedGraphicKind {
    Heading {
        level: u8,
        text: String,
    },
    LocalImage {
        alt: String,
        source: String,
        path: PathBuf,
    },
    Preview {
        label: String,
        source_hash: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedGraphic {
    pub kind: RenderedGraphicKind,
    pub image_id: u32,
    pub y_doc: usize,
    pub width_cells: u16,
    pub height_cells: u16,
    pub z_index: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockLayout {
    pub component: mdtui_core::ComponentId,
    pub start_row: usize,
    pub height: usize,
}

impl From<&LayoutBlock> for BlockLayout {
    fn from(value: &LayoutBlock) -> Self {
        Self {
            component: value.component,
            start_row: value.start_row,
            height: value.height,
        }
    }
}

impl From<&BlockLayout> for LayoutBlock {
    fn from(value: &BlockLayout) -> Self {
        Self {
            component: value.component,
            start_row: value.start_row,
            height: value.height,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRequest {
    pub label: String,
    pub source_hash: u64,
    pub source: String,
    pub width_cells: u16,
    pub height_cells: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderOptions {
    pub kitty_placeholders: bool,
    pub preview_graphics: bool,
    pub image_widget_previews: bool,
}

pub fn render_document(document: &Document, width: u16) -> RenderedDocument {
    render_document_window(document, width, 0, usize::MAX, 0)
}

pub fn render_document_window(
    document: &Document,
    width: u16,
    start_row: usize,
    height: usize,
    overscan: usize,
) -> RenderedDocument {
    render_document_window_with_options(
        document,
        width,
        start_row,
        height,
        overscan,
        RenderOptions::default(),
    )
}

pub fn render_document_window_with_options(
    document: &Document,
    width: u16,
    start_row: usize,
    height: usize,
    overscan: usize,
    options: RenderOptions,
) -> RenderedDocument {
    render_document_window_inner(document, None, width, start_row, height, overscan, options)
}

pub fn render_document_window_with_cache(
    document: &mut Document,
    width: u16,
    start_row: usize,
    height: usize,
    overscan: usize,
    options: RenderOptions,
) -> RenderedDocument {
    let source = document.source();
    let components = document.components.components.clone();
    let generation = document.dirty.generation;
    let cursor_byte = document.cursor.byte;
    let shell = RenderDocumentShell {
        source,
        path: document.path.clone(),
        components,
        generation,
        cursor_byte,
    };
    render_document_window_inner(
        &shell,
        Some(&mut document.layout_cache),
        width,
        start_row,
        height,
        overscan,
        options,
    )
}

trait RenderDocument {
    fn source_text(&self) -> String;
    fn document_path(&self) -> Option<PathBuf>;
    fn components(&self) -> &[Component];
    fn generation(&self) -> u64;
    fn cursor_byte(&self) -> usize;
}

impl RenderDocument for Document {
    fn source_text(&self) -> String {
        self.source()
    }

    fn document_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn components(&self) -> &[Component] {
        &self.components.components
    }

    fn generation(&self) -> u64 {
        self.dirty.generation
    }

    fn cursor_byte(&self) -> usize {
        self.cursor.byte
    }
}

struct RenderDocumentShell {
    source: String,
    path: Option<PathBuf>,
    components: Vec<Component>,
    generation: u64,
    cursor_byte: usize,
}

impl RenderDocument for RenderDocumentShell {
    fn source_text(&self) -> String {
        self.source.clone()
    }

    fn document_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn components(&self) -> &[Component] {
        &self.components
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn cursor_byte(&self) -> usize {
        self.cursor_byte
    }
}

fn render_document_window_inner(
    document: &impl RenderDocument,
    layout_cache: Option<&mut mdtui_core::LayoutCache>,
    width: u16,
    start_row: usize,
    height: usize,
    overscan: usize,
    options: RenderOptions,
) -> RenderedDocument {
    let visible_start = start_row.saturating_sub(overscan);
    let visible_end = start_row
        .saturating_add(height)
        .saturating_add(overscan)
        .max(visible_start.saturating_add(1));
    let mut builder = RenderBuilder {
        source: document.source_text(),
        document_path: document.document_path(),
        width: width.max(12),
        lines: Vec::new(),
        graphics: Vec::new(),
        pending_previews: Vec::new(),
        total_rows: 0,
        cursor_byte: document.cursor_byte(),
        cursor: None,
        visible_start,
        visible_end,
        options,
        skipped_components: 0,
    };

    let block_index = match layout_cache {
        Some(cache)
            if cache.generation == document.generation()
                && cache.preview_generation == preview_cache_generation()
                && cache.width == builder.width
                && cache.kitty_placeholders == options.kitty_placeholders
                && cache.preview_graphics == options.preview_graphics =>
        {
            cache.blocks.iter().map(BlockLayout::from).collect()
        }
        Some(cache) => {
            let built = builder.build_block_index(document.components());
            cache.generation = document.generation();
            cache.preview_generation = preview_cache_generation();
            cache.width = builder.width;
            cache.kitty_placeholders = options.kitty_placeholders;
            cache.preview_graphics = options.preview_graphics;
            cache.blocks = built.iter().map(LayoutBlock::from).collect();
            built
        }
        None => builder.build_block_index(document.components()),
    };
    let total_rows = block_index
        .last()
        .map(|block| block.start_row.saturating_add(block.height))
        .unwrap_or(0);
    let first_visible = first_visible_block(&block_index, visible_start);
    let mut rendered_count = 0_usize;
    for block in block_index.iter().skip(first_visible) {
        if block.start_row >= visible_end {
            break;
        }
        let Some(component) = document
            .components()
            .iter()
            .find(|component| component.id == block.component)
        else {
            continue;
        };
        builder.total_rows = block.start_row;
        builder.render_component(component);
        rendered_count = rendered_count.saturating_add(1);
    }
    builder.total_rows = total_rows;
    builder.skipped_components = block_index.len().saturating_sub(rendered_count);

    if builder.lines.is_empty() {
        builder.push_line(
            vec![cell("+ Insert block", muted())],
            Vec::new(),
            vec![HitZone {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width,
                    height: 1,
                },
                target: CursorTarget::Gap {
                    before: None,
                    after: None,
                },
                z: 0,
            }],
        );
    }

    RenderedDocument {
        lines: builder.lines,
        graphics: builder.graphics,
        pending_previews: builder.pending_previews,
        total_rows: builder.total_rows,
        cursor: builder.cursor,
        skipped_components: builder.skipped_components,
        block_index,
    }
}

pub fn hit_test(rendered: &RenderedDocument, x: u16, y: usize) -> Option<CursorTarget> {
    let line = rendered.lines.get(y)?;
    line.hit_zones
        .iter()
        .rev()
        .find(|zone| zone.rect.contains(x, 0))
        .map(|zone| zone.target.clone())
}

fn first_visible_block(block_index: &[BlockLayout], visible_start: usize) -> usize {
    block_index
        .partition_point(|block| block.start_row.saturating_add(block.height) <= visible_start)
}

pub fn warm_preview(request: &PreviewRequest) -> bool {
    let Ok(mut cache) = preview_cache().lock() else {
        return false;
    };
    let inserted = cache.insert(request.source_hash);
    if inserted {
        PREVIEW_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    inserted
}

pub fn clear_preview_cache() {
    if let Ok(mut cache) = preview_cache().lock() {
        cache.clear();
    }
    PREVIEW_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn preview_is_warm(source_hash: u64) -> bool {
    preview_cache()
        .lock()
        .map(|cache| cache.contains(&source_hash))
        .unwrap_or(false)
}

fn preview_cache() -> &'static Mutex<HashSet<u64>> {
    static CACHE: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn preview_cache_generation() -> u64 {
    PREVIEW_CACHE_GENERATION.load(Ordering::Relaxed)
}

static PREVIEW_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

struct RenderBuilder {
    source: String,
    document_path: Option<PathBuf>,
    width: u16,
    lines: Vec<RenderLine>,
    graphics: Vec<RenderedGraphic>,
    pending_previews: Vec<PreviewRequest>,
    total_rows: usize,
    cursor_byte: usize,
    cursor: Option<(u16, usize)>,
    visible_start: usize,
    visible_end: usize,
    options: RenderOptions,
    skipped_components: usize,
}

impl RenderBuilder {
    fn build_block_index(&self, components: &[Component]) -> Vec<BlockLayout> {
        let mut index = Vec::new();
        let mut start_row = 0_usize;
        for component in components {
            if matches!(component.kind, ComponentKind::ListItem { .. }) {
                continue;
            }
            let height = self.component_height(component);
            if height == 0 {
                continue;
            }
            index.push(BlockLayout {
                component: component.id,
                start_row,
                height,
            });
            start_row = start_row.saturating_add(height);
        }
        index
    }

    fn render_component(&mut self, component: &Component) {
        match &component.kind {
            ComponentKind::Gap => self.render_gap(component),
            ComponentKind::Heading { level } => self.render_heading(component, *level),
            ComponentKind::ThematicBreak => self.render_rule(component),
            ComponentKind::Paragraph => {
                let raw = self.slice(component).to_string();
                self.render_wrapped_inline(component, &raw, normal())
            }
            ComponentKind::BlockQuote => self.render_quote(component, None),
            ComponentKind::Alert { level } => self.render_quote(component, Some(level)),
            ComponentKind::List { ordered, .. } => self.render_list(component, *ordered),
            ComponentKind::CodeBlock { language, .. } => self.render_code(component, language),
            ComponentKind::DiagramBlock { language } => self.render_previewable(
                component,
                &format!("{language:?}").to_ascii_lowercase(),
                PreviewFallback::Code,
            ),
            ComponentKind::Table => self.render_table(component),
            ComponentKind::HtmlBlock => self.render_boxed(component, "html", Token::Muted),
            ComponentKind::MathBlock => {
                self.render_previewable(component, "math", PreviewFallback::Boxed)
            }
            ComponentKind::FootnoteDef => {
                self.render_wrapped(component, self.slice(component).trim().to_string(), faint())
            }
            ComponentKind::Image => self.render_image(component),
            ComponentKind::TableRow { .. }
            | ComponentKind::TableCell { .. }
            | ComponentKind::Link { .. }
            | ComponentKind::FootnoteRef
            | ComponentKind::ListItem { .. } => {}
        }
    }

    fn component_height(&self, component: &Component) -> usize {
        match &component.kind {
            ComponentKind::Gap | ComponentKind::ThematicBreak => 1,
            ComponentKind::Heading { level } => {
                let text = heading_display_text(self.slice(component), *level);
                if *level <= 2 && self.options.kitty_placeholders {
                    2
                } else {
                    let prefix = if *level <= 2 { " " } else { "" };
                    wrapped_line_count(&format!("{prefix}{text}"), self.width)
                        + usize::from(*level <= 2)
                }
            }
            ComponentKind::Paragraph => {
                wrapped_line_count(&inline_plain_text(self.slice(component)), self.width)
            }
            ComponentKind::BlockQuote => 1,
            ComponentKind::Alert { .. } => 1,
            ComponentKind::List { .. } => self.slice(component).lines().count().max(1),
            ComponentKind::CodeBlock { .. } => 2 + code_body(self.slice(component)).lines().count(),
            ComponentKind::DiagramBlock { .. } => {
                if self.preview_is_warm(component) {
                    PREVIEW_HEIGHT
                } else {
                    2 + code_body(self.slice(component)).lines().count()
                }
            }
            ComponentKind::Table => table_height(self.slice(component), self.width),
            ComponentKind::HtmlBlock => 2 + self.slice(component).lines().count(),
            ComponentKind::MathBlock => {
                if self.preview_is_warm(component) {
                    PREVIEW_HEIGHT
                } else {
                    2 + self.slice(component).lines().count()
                }
            }
            ComponentKind::FootnoteDef => {
                wrapped_line_count(self.slice(component).trim(), self.width)
            }
            ComponentKind::Image => {
                let raw = self.slice(component).trim();
                if let Some((_, src)) = image_label(raw)
                    && local_image_path(self.document_path.as_deref(), &src).is_some()
                    && (self.options.kitty_placeholders || self.options.image_widget_previews)
                {
                    return 4;
                }
                1
            }
            ComponentKind::TableRow { .. }
            | ComponentKind::TableCell { .. }
            | ComponentKind::Link { .. }
            | ComponentKind::FootnoteRef
            | ComponentKind::ListItem { .. } => 0,
        }
    }

    fn render_gap(&mut self, component: &Component) {
        let focused = component.source.contains(self.cursor_byte);
        let cells = if focused {
            vec![cell("+ Insert block", faint())]
        } else {
            Vec::new()
        };
        self.push_component_line(component, cells);
    }

    fn render_heading(&mut self, component: &Component, level: u8) {
        let display = heading_display_text(self.slice(component), level);
        let style = CellStyle {
            token: if level <= 2 {
                Token::Accent
            } else {
                Token::Normal
            },
            bold: true,
            ..CellStyle::default()
        };
        let prefix = match level {
            1 => " ",
            2 => " ",
            _ => "",
        };
        if level <= 2 {
            let image_id = graphic_image_id(&("heading", level, &display, self.width));
            self.push_graphic(RenderedGraphic {
                kind: RenderedGraphicKind::Heading {
                    level,
                    text: display.clone(),
                },
                image_id,
                y_doc: self.total_rows,
                width_cells: self.width,
                height_cells: 2,
                z_index: -10,
            });
            if self.options.kitty_placeholders {
                for row in 0..2 {
                    self.push_placeholder_component_line(component, image_id, row, self.width);
                }
                return;
            }
        }
        self.render_wrapped(component, format!("{prefix}{display}"), style);
        if level <= 2 {
            self.push_component_line(component, vec![cell("", style)]);
        }
    }

    fn render_rule(&mut self, component: &Component) {
        let width = usize::from(self.width.saturating_sub(2));
        self.push_component_line(component, vec![cell("─".repeat(width), border())]);
    }

    fn render_quote(&mut self, component: &Component, alert: Option<&AlertLevel>) {
        let raw = self
            .slice(component)
            .lines()
            .map(|line| line.trim_start().trim_start_matches('>').trim_start())
            .filter(|line| !line.starts_with("[!"))
            .collect::<Vec<_>>()
            .join(" ");
        let label = alert.map(alert_label);
        if let Some(label) = label {
            let mut cells = vec![cell("▌ ", quote()), cell(format!("{label} "), accent())];
            cells.extend(text_cells(&raw, muted()));
            self.push_component_line(component, cells);
            return;
        }

        let mut cells = vec![cell("▌ ", quote())];
        cells.extend(text_cells(
            &raw,
            CellStyle {
                italic: true,
                ..muted()
            },
        ));
        self.push_component_line(component, cells);
    }

    fn render_list(&mut self, component: &Component, ordered: bool) {
        let raw = self.slice(component).to_string();
        let component_start = self.total_rows;
        let line_count = raw.lines().count().max(1);
        let first_line = self
            .visible_start
            .saturating_sub(component_start)
            .min(line_count);
        let visible_count = self
            .visible_end
            .saturating_sub(component_start.saturating_add(first_line))
            .min(line_count.saturating_sub(first_line));
        if visible_count == 0 {
            self.total_rows = component_start.saturating_add(line_count);
            return;
        }
        self.total_rows = component_start.saturating_add(first_line);
        for (idx, line) in raw.lines().enumerate().skip(first_line).take(visible_count) {
            let trimmed = line.trim_start();
            let (marker, marker_style) = if ordered {
                (format!("{}.", idx + 1), accent())
            } else if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
                ("☑".to_string(), success())
            } else if trimmed.starts_with("- [ ]") {
                ("☐".to_string(), accent())
            } else {
                ("•".to_string(), accent())
            };
            let text = list_text(trimmed);
            let mut cells = vec![cell(format!("  {marker}  "), marker_style)];
            cells.extend(inline_cells(&text, normal()));
            self.push_component_line(component, cells);
        }
        self.total_rows = component_start.saturating_add(line_count);
    }

    fn render_code(&mut self, component: &Component, language: &str) {
        let raw = self.slice(component);
        let body = code_body(raw);
        let body_line_count = body.lines().count();
        let body_start = self.total_rows.saturating_add(1);
        let body_end = body_start.saturating_add(body_line_count);
        let title = if language.is_empty() {
            "code"
        } else {
            language
        };
        let max_body_width = body.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
        let title_width = UnicodeWidthStr::width(title);
        let surface_width = surface_width(
            max_body_width
                .saturating_add(8)
                .max(title_width.saturating_add(13)),
            self.width,
            34,
        );
        let title = truncate_to_width(title, surface_width.saturating_sub(13).max(1));
        let title_width = UnicodeWidthStr::width(title.as_str());
        let content_width = surface_width.saturating_sub(8);
        let title_filler = surface_width.saturating_sub(title_width.saturating_add(13));
        self.push_component_line(
            component,
            vec![cell(
                format!("┌─ {title} {} [Copy] ┐", "─".repeat(title_filler)),
                raised(border()),
            )],
        );

        let visible_body_start = body_start.max(self.visible_start).min(body_end);
        let visible_body_end = body_end.min(self.visible_end).max(visible_body_start);
        if self.total_rows < visible_body_start {
            self.total_rows = visible_body_start;
        }

        let first_visible_body_line = visible_body_start.saturating_sub(body_start);
        let visible_body_line_count = visible_body_end.saturating_sub(visible_body_start);
        let highlighted_lines = if body_line_count <= CODE_HIGHLIGHT_CACHE_CAP {
            Some(highlighted_code_lines(language, &body))
        } else {
            None
        };
        for (idx, line) in body
            .lines()
            .enumerate()
            .skip(first_visible_body_line)
            .take(visible_body_line_count)
        {
            let number = format!("{:>3} ", idx + 1);
            let mut cells = vec![cell("│ ", raised(border())), cell(number, raised(faint()))];
            let mut content_cells = if let Some(highlighted) = highlighted_lines
                .as_ref()
                .and_then(|highlighted_lines| highlighted_lines.get(idx))
            {
                with_bg(
                    truncate_cells_to_width(highlighted, content_width),
                    CellBg::Raised,
                )
            } else {
                text_cells(&truncate_to_width(line, content_width), raised(code()))
            };
            let used_content_width = cells_width(&content_cells) as usize;
            cells.append(&mut content_cells);
            if content_width > used_content_width {
                cells.push(cell(
                    " ".repeat(content_width - used_content_width),
                    raised(code()),
                ));
            }
            cells.push(cell(" │", raised(border())));
            self.push_component_line(component, cells);
        }

        if self.total_rows < body_end {
            self.total_rows = body_end;
        }
        self.push_component_line(
            component,
            vec![cell(
                format!("└{}┘", "─".repeat(surface_width.saturating_sub(2))),
                raised(border()),
            )],
        );
    }

    fn render_previewable(
        &mut self,
        component: &Component,
        label: &str,
        fallback: PreviewFallback,
    ) {
        if self.preview_is_warm(component) {
            let source_hash = stable_hash(self.slice(component));
            let width_cells = self.width.min(64);
            let image_id = graphic_image_id(&("preview", label, source_hash, width_cells));
            self.push_graphic(RenderedGraphic {
                kind: RenderedGraphicKind::Preview {
                    label: label.to_string(),
                    source_hash,
                },
                image_id,
                y_doc: self.total_rows,
                width_cells,
                height_cells: PREVIEW_HEIGHT as u16,
                z_index: -20,
            });
            if self.options.kitty_placeholders {
                for row in 0..PREVIEW_HEIGHT as u16 {
                    self.push_placeholder_component_line(component, image_id, row, width_cells);
                }
                return;
            }
        } else if self.options.preview_graphics {
            self.pending_previews.push(PreviewRequest {
                label: label.to_string(),
                source_hash: stable_hash(self.slice(component)),
                source: self.slice(component).to_string(),
                width_cells: self.width.min(64),
                height_cells: PREVIEW_HEIGHT as u16,
            });
        }

        match fallback {
            PreviewFallback::Code => self.render_code(component, label),
            PreviewFallback::Boxed => self.render_boxed(component, label, Token::Accent),
        }
    }

    fn render_table(&mut self, component: &Component) {
        let rows = table_rows(self.slice(component));
        if rows.is_empty() {
            self.render_wrapped(component, self.slice(component).to_string(), normal());
            return;
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(1);
        let mut widths = vec![3_usize; columns];
        for row in &rows {
            for (idx, value) in row.iter().enumerate() {
                widths[idx] = widths[idx]
                    .max(UnicodeWidthStr::width(value.as_str()))
                    .min(24);
            }
        }
        let component_start = self.total_rows;
        let total_height = table_render_height(rows.len());
        self.push_component_line_if_visible(
            component,
            component_start,
            table_border_cells('┌', '┬', '┐', &widths),
        );
        for (row_idx, row) in rows.iter().enumerate() {
            let row_y = component_start
                .saturating_add(1)
                .saturating_add(row_idx)
                .saturating_add(usize::from(row_idx > 0 && rows.len() > 1));
            let mut cells = vec![cell("│", border())];
            let row_style = if row_idx == 0 { accent() } else { normal() };
            for (idx, width) in widths.iter().enumerate() {
                let value = row.get(idx).map(String::as_str).unwrap_or("");
                cells.push(cell(format!(" {} ", fit_to_width(value, *width)), row_style));
                cells.push(cell("│", border()));
            }
            self.push_component_line_if_visible(component, row_y, cells);
            if row_idx == 0 && rows.len() > 1 {
                self.push_component_line_if_visible(
                    component,
                    component_start.saturating_add(2),
                    table_border_cells('├', '┼', '┤', &widths),
                );
            }
        }
        self.push_component_line_if_visible(
            component,
            component_start.saturating_add(total_height.saturating_sub(1)),
            table_border_cells('└', '┴', '┘', &widths),
        );
        self.total_rows = component_start.saturating_add(total_height);
    }

    fn render_boxed(&mut self, component: &Component, label: &str, token: Token) {
        let raw = self.slice(component).to_string();
        let lines = raw.lines().collect::<Vec<_>>();
        let component_start = self.total_rows;
        let body_start = component_start.saturating_add(1);
        let body_end = body_start.saturating_add(lines.len());
        let total_height = lines.len().saturating_add(2);
        self.push_component_line_if_visible(
            component,
            component_start,
            vec![cell(format!("┌─ {label} ─"), border())],
        );

        let visible_body_start = body_start.max(self.visible_start).min(body_end);
        let visible_body_end = body_end.min(self.visible_end).max(visible_body_start);
        let first_visible_line = visible_body_start.saturating_sub(body_start);
        let visible_line_count = visible_body_end.saturating_sub(visible_body_start);
        for (idx, line) in lines
            .iter()
            .enumerate()
            .skip(first_visible_line)
            .take(visible_line_count)
        {
            let style = CellStyle {
                token,
                ..CellStyle::default()
            };
            self.push_component_line_if_visible(
                component,
                body_start.saturating_add(idx),
                vec![cell("│ ", border()), cell(*line, style)],
            );
        }
        self.push_component_line_if_visible(
            component,
            component_start.saturating_add(total_height.saturating_sub(1)),
            vec![cell("└────", border())],
        );
        self.total_rows = component_start.saturating_add(total_height);
    }

    fn render_image(&mut self, component: &Component) {
        let raw = self.slice(component).trim();
        let text = if let Some((alt, src)) = image_label(raw) {
            if let Some(path) = local_image_path(self.document_path.as_deref(), &src) {
                let width_cells = self.width.min(48);
                let image_id =
                    graphic_image_id(&("image", path.to_string_lossy().as_ref(), width_cells));
                self.push_graphic(RenderedGraphic {
                    kind: RenderedGraphicKind::LocalImage {
                        alt: alt.clone(),
                        source: src.clone(),
                        path,
                    },
                    image_id,
                    y_doc: self.total_rows,
                    width_cells,
                    height_cells: 4,
                    z_index: -20,
                });
                if self.options.kitty_placeholders {
                    for row in 0..4 {
                        self.push_placeholder_component_line(component, image_id, row, width_cells);
                    }
                    return;
                }
                if self.options.image_widget_previews {
                    self.push_component_line(
                        component,
                        vec![cell(format!("[image] {alt}"), muted())],
                    );
                    for _ in 1..4 {
                        self.push_component_line(component, Vec::new());
                    }
                    return;
                }
            }
            format!("[image] {alt} -> {src}")
        } else {
            format!("[image] {raw}")
        };
        self.push_component_line(component, vec![cell(text, muted())]);
    }

    fn render_wrapped(&mut self, component: &Component, text: String, style: CellStyle) {
        self.render_wrapped_cells(component, text_cells(&text, style));
    }

    fn render_wrapped_inline(&mut self, component: &Component, text: &str, style: CellStyle) {
        self.render_wrapped_cells(component, inline_cells(text, style));
    }

    fn render_wrapped_cells(&mut self, component: &Component, cells: Vec<StyledCell>) {
        let wrap_width = usize::from(self.width.max(1));
        let mut current = Vec::new();
        let mut word = Vec::new();
        for cell in cells {
            if cell.text.chars().all(char::is_whitespace) {
                self.append_wrapped_word(component, wrap_width, &mut current, &mut word);
                continue;
            }
            word.push(cell);
        }
        self.append_wrapped_word(component, wrap_width, &mut current, &mut word);
        if current.is_empty() {
            self.push_component_line(component, Vec::new());
        } else {
            self.push_component_line(component, current);
        }
    }

    fn append_wrapped_word(
        &mut self,
        component: &Component,
        wrap_width: usize,
        current: &mut Vec<StyledCell>,
        word: &mut Vec<StyledCell>,
    ) {
        if word.is_empty() {
            return;
        }
        let current_width = cells_width(current) as usize;
        let word_width = cells_width(word) as usize;
        let separator = usize::from(!current.is_empty());
        if !current.is_empty() && current_width + separator + word_width > wrap_width {
            self.push_component_line(component, std::mem::take(current));
        } else if !current.is_empty() {
            current.push(cell(" ", normal()));
        }
        current.append(word);
    }

    fn push_component_line(&mut self, component: &Component, cells: Vec<StyledCell>) {
        let y = self.total_rows;
        let width = cells_width(&cells).min(self.width);
        let target = CursorTarget::Component {
            component: component.id,
            slot: ComponentSlot::Body,
        };
        let hit_zones = vec![HitZone {
            rect: Rect {
                x: 0,
                y: 0,
                width,
                height: 1,
            },
            target,
            z: 0,
        }];
        let source_spans = vec![SourceCellSpan {
            x: 0,
            width,
            source: component.source.clone(),
        }];
        if self.cursor.is_none() && component.source.contains(self.cursor_byte) {
            let local = self.cursor_byte.saturating_sub(component.source.start);
            let slice_end = component.source.start + local.min(self.slice(component).len());
            let prefix = &self.source[component.source.start..slice_end];
            self.cursor = Some((UnicodeWidthStr::width(prefix) as u16, y));
        }
        self.push_line(cells, source_spans, hit_zones);
    }

    fn push_component_line_if_visible(
        &mut self,
        component: &Component,
        y: usize,
        cells: Vec<StyledCell>,
    ) {
        if y < self.visible_start || y >= self.visible_end {
            return;
        }
        self.total_rows = y;
        self.push_component_line(component, cells);
    }

    fn push_placeholder_component_line(
        &mut self,
        component: &Component,
        image_id: u32,
        row: u16,
        width: u16,
    ) {
        let y = self.total_rows;
        let target = CursorTarget::Component {
            component: component.id,
            slot: ComponentSlot::Body,
        };
        let hit_zones = vec![HitZone {
            rect: Rect {
                x: 0,
                y: 0,
                width,
                height: 1,
            },
            target,
            z: 0,
        }];
        let source_spans = vec![SourceCellSpan {
            x: 0,
            width,
            source: component.source.clone(),
        }];
        if self.cursor.is_none() && component.source.contains(self.cursor_byte) {
            self.cursor = Some((0, y));
        }
        self.push_line(
            vec![StyledCell {
                text: kitty_placeholder_row(row, width),
                style: CellStyle {
                    fg: Some(rgb_from_image_id(image_id)),
                    ..CellStyle::default()
                },
            }],
            source_spans,
            hit_zones,
        );
    }

    fn push_line(
        &mut self,
        cells: Vec<StyledCell>,
        source_spans: Vec<SourceCellSpan>,
        hit_zones: Vec<HitZone>,
    ) {
        let y_doc = self.total_rows;
        self.total_rows = self.total_rows.saturating_add(1);
        if y_doc >= self.visible_start && y_doc < self.visible_end {
            self.lines.push(RenderLine {
                y_doc,
                cells,
                source_spans,
                hit_zones,
            });
        }
    }

    fn push_graphic(&mut self, graphic: RenderedGraphic) {
        let graphic_end = graphic
            .y_doc
            .saturating_add(usize::from(graphic.height_cells));
        if graphic_end > self.visible_start && graphic.y_doc < self.visible_end {
            self.graphics.push(graphic);
        }
    }

    fn slice<'a>(&'a self, component: &Component) -> &'a str {
        &self.source[component.source.start..component.source.end]
    }

    fn preview_is_warm(&self, component: &Component) -> bool {
        self.options.preview_graphics && preview_is_warm(stable_hash(self.slice(component)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewFallback {
    Code,
    Boxed,
}

const PREVIEW_HEIGHT: usize = 4;

fn alert_label(level: &AlertLevel) -> &'static str {
    match level {
        AlertLevel::Note => "NOTE",
        AlertLevel::Tip => "TIP",
        AlertLevel::Important => "IMPORTANT",
        AlertLevel::Warning => "WARNING",
        AlertLevel::Caution => "CAUTION",
    }
}

fn heading_display_text(raw: &str, level: u8) -> String {
    let text = raw
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim()
        .to_string();
    if level == 1 {
        text.to_uppercase()
    } else {
        text
    }
}

fn wrapped_line_count(text: &str, width: u16) -> usize {
    let wrap_width = usize::from(width.max(1));
    let mut current = String::new();
    let mut lines = 0_usize;
    for word in text.split_whitespace() {
        let next_width = UnicodeWidthStr::width(current.as_str())
            + if current.is_empty() { 0 } else { 1 }
            + UnicodeWidthStr::width(word);
        if !current.is_empty() && next_width > wrap_width {
            lines = lines.saturating_add(1);
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if current.is_empty() {
        lines.max(1)
    } else {
        lines.saturating_add(1)
    }
}

fn inline_plain_text(text: &str) -> String {
    inline_cells(text, normal())
        .into_iter()
        .map(|cell| cell.text)
        .collect()
}

fn inline_cells(text: &str, base: CellStyle) -> Vec<StyledCell> {
    let mut cells = Vec::new();
    let mut index = 0_usize;
    let mut style = base;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("~~") {
            style.struck = !style.struck;
            index += 2;
            continue;
        }
        if rest.starts_with("**") {
            style.bold = !style.bold;
            index += 2;
            continue;
        }
        let Some(grapheme) = UnicodeSegmentation::graphemes(rest, true).next() else {
            break;
        };
        let Some(ch) = grapheme.chars().next() else {
            break;
        };
        match ch {
            '`' => {
                style.token = if style.token == Token::Code {
                    base.token
                } else {
                    Token::Code
                };
                index += ch.len_utf8();
            }
            '*' | '_' => {
                style.italic = !style.italic;
                index += ch.len_utf8();
            }
            '[' => {
                if let Some((label, consumed)) = inline_link_label(rest) {
                    let mut link_style = style;
                    link_style.token = Token::Link;
                    link_style.underlined = true;
                    cells.extend(inline_cells(label, link_style));
                    index += consumed;
                } else {
                    cells.push(cell(ch.to_string(), style));
                    index += ch.len_utf8();
                }
            }
            ':' => {
                if let Some((emoji, consumed)) = emoji_shortcode(rest) {
                    cells.extend(text_cells(emoji, style));
                    index += consumed;
                } else {
                    cells.push(cell(ch.to_string(), style));
                    index += ch.len_utf8();
                }
            }
            _ => {
                cells.push(cell(grapheme.to_string(), style));
                index += grapheme.len();
            }
        }
    }
    cells
}

fn inline_link_label(rest: &str) -> Option<(&str, usize)> {
    let label_end = rest.find(']')?;
    let label = &rest[1..label_end];
    let after_label = &rest[label_end + 1..];
    if let Some(destination) = after_label.strip_prefix('(') {
        let close = destination.find(')')?;
        return Some((label, label_end + 1 + close + 2));
    }
    if let Some(reference) = after_label.strip_prefix('[') {
        let close = reference.find(']')?;
        return Some((label, label_end + 1 + close + 2));
    }
    None
}

fn emoji_shortcode(rest: &str) -> Option<(&'static str, usize)> {
    let after_colon = rest.strip_prefix(':')?;
    let end = after_colon.find(':')?;
    let name = &after_colon[..end];
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '+' || ch == '-')
    {
        return None;
    }
    let emoji = match name {
        "+1" | "thumbsup" => "👍",
        "-1" | "thumbsdown" => "👎",
        "brain" => "🧠",
        "bulb" => "💡",
        "caution" => "⚠️",
        "fire" => "🔥",
        "heart" => "❤️",
        "memo" => "📝",
        "rocket" => "🚀",
        "smile" | "smiley" => "😄",
        "sparkles" => "✨",
        "tada" => "🎉",
        "warning" => "⚠️",
        "white_check_mark" => "✅",
        "x" => "❌",
        _ => return None,
    };
    Some((emoji, name.len() + 2))
}

fn table_rows(raw: &str) -> Vec<Vec<String>> {
    raw.lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .map(parse_table_row)
        .filter(|row| {
            !row.iter().all(|cell| {
                cell.chars()
                    .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
            })
        })
        .collect()
}

fn table_height(raw: &str, width: u16) -> usize {
    let rows = table_rows(raw);
    if rows.is_empty() {
        return wrapped_line_count(raw, width);
    }
    table_render_height(rows.len())
}

fn table_render_height(row_count: usize) -> usize {
    if row_count > 1 {
        row_count + 3
    } else {
        row_count + 2
    }
}

fn image_label(raw: &str) -> Option<(String, String)> {
    let rest = raw.strip_prefix("![")?;
    let alt_end = rest.find("](")?;
    let alt = rest[..alt_end].to_string();
    let after = &rest[alt_end + 2..];
    let close = after.find(')')?;
    let src = after[..close]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    Some((alt, src))
}

fn graphic_image_id(value: &impl Hash) -> u32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let id = (hasher.finish() & 0x00ff_ffff) as u32;
    id.max(1)
}

fn rgb_from_image_id(image_id: u32) -> Rgb {
    Rgb(
        ((image_id >> 16) & 0xff) as u8,
        ((image_id >> 8) & 0xff) as u8,
        (image_id & 0xff) as u8,
    )
}

fn kitty_placeholder_row(row: u16, width: u16) -> String {
    let mut text = String::new();
    if width == 0 {
        return text;
    }
    text.push_str(&kitty_placeholder_cell(row, true));
    for _ in 1..width {
        text.push('\u{10eeee}');
    }
    text
}

fn kitty_placeholder_cell(row: u16, include_column_zero: bool) -> String {
    let mut text = String::from("\u{10eeee}");
    text.push(kitty_placeholder_diacritic(row));
    if include_column_zero {
        text.push(kitty_placeholder_diacritic(0));
    }
    text
}

fn kitty_placeholder_diacritic(value: u16) -> char {
    const DIACRITICS: [char; 8] = [
        '\u{0305}', '\u{030d}', '\u{030e}', '\u{0310}', '\u{0312}', '\u{033d}', '\u{033e}',
        '\u{033f}',
    ];
    DIACRITICS
        .get(usize::from(value))
        .copied()
        .unwrap_or('\u{0305}')
}

fn local_image_path(document_path: Option<&Path>, source: &str) -> Option<PathBuf> {
    if !is_local_image_source(source) || !is_supported_local_image_source(source) {
        return None;
    }
    let path = PathBuf::from(source);
    if path.is_absolute() {
        return Some(path);
    }
    let base = document_path
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    Some(base.join(path))
}

fn is_local_image_source(source: &str) -> bool {
    !(source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:")
        || source.starts_with('#'))
}

fn is_supported_local_image_source(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn code_body(raw: &str) -> String {
    let mut lines = raw.lines();
    let first = lines.next().unwrap_or_default();
    if first.trim_start().starts_with("```") || first.trim_start().starts_with("~~~") {
        lines
            .take_while(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("```") && !trimmed.starts_with("~~~")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        raw.to_string()
    }
}

fn list_text(line: &str) -> String {
    let without_marker = ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
        .or_else(|| {
            let dot = line.find(". ")?;
            Some(&line[dot + 2..])
        })
        .unwrap_or(line);
    without_marker
        .strip_prefix("[ ] ")
        .or_else(|| without_marker.strip_prefix("[x] "))
        .or_else(|| without_marker.strip_prefix("[X] "))
        .unwrap_or(without_marker)
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodeHighlightKey {
    language: String,
    body_hash: u64,
}

type HighlightedCodeLines = Vec<Vec<StyledCell>>;
type CodeHighlightCache = VecDeque<(CodeHighlightKey, HighlightedCodeLines)>;

fn highlighted_code_lines(language: &str, body: &str) -> Vec<Vec<StyledCell>> {
    let key = CodeHighlightKey {
        language: language.trim().to_ascii_lowercase(),
        body_hash: stable_hash(body),
    };
    if let Some(lines) = cached_highlight(&key) {
        return lines;
    }
    let highlighted = highlight_code_lines_uncached(language, body);
    store_highlight(key, highlighted.clone());
    highlighted
}

fn cached_highlight(key: &CodeHighlightKey) -> Option<Vec<Vec<StyledCell>>> {
    highlight_cache()
        .lock()
        .ok()?
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, lines)| lines.clone())
}

fn store_highlight(key: CodeHighlightKey, lines: Vec<Vec<StyledCell>>) {
    let Ok(mut cache) = highlight_cache().lock() else {
        return;
    };
    if let Some(index) = cache.iter().position(|(candidate, _)| candidate == &key) {
        cache.remove(index);
    }
    cache.push_front((key, lines));
    while cache.len() > CODE_HIGHLIGHT_CACHE_CAP {
        cache.pop_back();
    }
}

fn highlight_cache() -> &'static Mutex<CodeHighlightCache> {
    static CACHE: OnceLock<Mutex<CodeHighlightCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn highlight_code_lines_uncached(language: &str, body: &str) -> Vec<Vec<StyledCell>> {
    let syntax_set = syntax_set();
    let syntax = syntax_set
        .find_syntax_by_token(language)
        .or_else(|| syntax_set.find_syntax_by_extension(language))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let theme = theme_set()
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set().themes.values().next());
    let Some(theme) = theme else {
        return body.lines().map(|line| text_cells(line, code())).collect();
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    body.lines()
        .map(|line| {
            highlighter
                .highlight_line(line, syntax_set)
                .map(|ranges| {
                    ranges
                        .into_iter()
                        .flat_map(|(style, text)| text_cells(text, syntect_cell_style(style)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|_| text_cells(line, code()))
        })
        .collect()
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn syntect_cell_style(style: SyntectStyle) -> CellStyle {
    let color = style.foreground;
    let token = if color.b > color.r.saturating_add(30) && color.b > color.g {
        Token::Blue
    } else if color.r > 180 && color.b > 140 {
        Token::Purple
    } else if color.r > 180 && color.g < 150 {
        Token::Pink
    } else if color.g > color.r && color.g > color.b {
        Token::Success
    } else if color.r > 180 && color.g > 150 {
        Token::Warn
    } else {
        Token::Code
    };
    CellStyle {
        token,
        fg: None,
        bg: None,
        bold: style
            .font_style
            .contains(syntect::highlighting::FontStyle::BOLD),
        italic: style
            .font_style
            .contains(syntect::highlighting::FontStyle::ITALIC),
        underlined: style
            .font_style
            .contains(syntect::highlighting::FontStyle::UNDERLINE),
        struck: false,
        reversed: false,
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn table_border_cells(left: char, mid: char, right: char, widths: &[usize]) -> Vec<StyledCell> {
    let mut cells = vec![cell(left.to_string(), border())];
    for (idx, width) in widths.iter().enumerate() {
        cells.push(cell("─".repeat(width + 2), border()));
        let separator = if idx + 1 == widths.len() { right } else { mid };
        cells.push(cell(separator.to_string(), border()));
    }
    cells
}

fn fit_to_width(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    let used = UnicodeWidthStr::width(truncated.as_str());
    format!("{truncated}{}", " ".repeat(width.saturating_sub(used)))
}

fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .map(|cell| UnicodeWidthStr::width(cell.text.as_str()))
        .sum::<usize>()
        .try_into()
        .unwrap_or(u16::MAX)
}

fn surface_width(content_width: usize, viewport_width: u16, min_width: usize) -> usize {
    let viewport_width = usize::from(viewport_width).max(12);
    content_width.max(min_width).min(viewport_width)
}

fn truncate_to_width(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut truncated = String::new();
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        truncated.push_str(grapheme);
    }
    truncated
}

fn truncate_cells_to_width(cells: &[StyledCell], width: usize) -> Vec<StyledCell> {
    let mut used = 0;
    let mut truncated = Vec::new();
    for cell in cells {
        let cell_width = UnicodeWidthStr::width(cell.text.as_str());
        if used + cell_width <= width {
            used += cell_width;
            truncated.push(cell.clone());
            continue;
        }
        for grapheme in UnicodeSegmentation::graphemes(cell.text.as_str(), true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if used + grapheme_width > width {
                return truncated;
            }
            used += grapheme_width;
            truncated.push(StyledCell {
                text: grapheme.to_string(),
                style: cell.style,
            });
        }
    }
    truncated
}

fn text_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    UnicodeSegmentation::graphemes(text, true)
        .map(|grapheme| {
            let width = UnicodeWidthStr::width(grapheme);
            cell(
                if width == 0 {
                    String::new()
                } else {
                    grapheme.to_string()
                },
                style,
            )
        })
        .collect()
}

fn cell(text: impl Into<String>, style: CellStyle) -> StyledCell {
    StyledCell {
        text: text.into(),
        style,
    }
}

fn with_bg(mut cells: Vec<StyledCell>, bg: CellBg) -> Vec<StyledCell> {
    for cell in &mut cells {
        cell.style.bg = Some(bg);
    }
    cells
}

fn raised(mut style: CellStyle) -> CellStyle {
    style.bg = Some(CellBg::Raised);
    style
}

fn normal() -> CellStyle {
    CellStyle::default()
}

fn muted() -> CellStyle {
    CellStyle {
        token: Token::Muted,
        ..CellStyle::default()
    }
}

fn faint() -> CellStyle {
    CellStyle {
        token: Token::Faint,
        ..CellStyle::default()
    }
}

fn accent() -> CellStyle {
    CellStyle {
        token: Token::Accent,
        bold: true,
        ..CellStyle::default()
    }
}

fn code() -> CellStyle {
    CellStyle {
        token: Token::Code,
        ..CellStyle::default()
    }
}

fn border() -> CellStyle {
    CellStyle {
        token: Token::Border,
        ..CellStyle::default()
    }
}

fn quote() -> CellStyle {
    CellStyle {
        token: Token::Quote,
        bold: true,
        ..CellStyle::default()
    }
}

fn success() -> CellStyle {
    CellStyle {
        token: Token::Success,
        bold: true,
        ..CellStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use mdtui_core::Document;

    use super::*;

    #[test]
    fn renders_main_markdown_shapes() {
        let document = Document::new(None, "# Title\n\n- [ ] task\n\n| A |\n| - |\n| B |\n");
        let rendered = render_document(&document, 50);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        assert!(text.contains("TITLE"));
        assert!(text.contains("☐"));
        assert!(text.contains("┌"));
    }

    #[test]
    fn renders_visual_checkboxes_blockquotes_and_table_headers() {
        let document = Document::new(
            None,
            "- [x] done\n- [ ] todo\n\n> Simplicity wins.\n\n| Feature | Status |\n| --- | --- |\n| Tasks | Yes |\n",
        );
        let rendered = render_document(&document, 80);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        assert!(text.contains("☑"));
        assert!(text.contains("☐"));
        assert!(text.contains("▌ "));
        assert!(text.contains("Simplicity wins."));
        assert!(rendered.lines.iter().any(|line| {
            line.cells
                .iter()
                .any(|cell| cell.text.contains("Feature") && cell.style.token == Token::Accent)
        }));
    }

    #[test]
    fn tables_keep_borders_while_code_and_alerts_render_as_raised_surfaces() {
        let document = Document::new(
            None,
            "| Name | Role |\n| --- | --- |\n| Andreas | Developer |\n\n```rust\nfn main() {}\n```\n\n> [!NOTE]\n> Alerts render cleanly.\n",
        );
        let rendered = render_document(&document, 80);

        let has_table_border = rendered.lines.iter().any(|line| {
            line.cells
                .iter()
                .any(|cell| cell.text.contains("┌") && cell.style.token == Token::Border)
        });
        let table_cells_have_no_raised_bg = rendered.lines.iter().all(|line| {
            line.cells
                .iter()
                .filter(|cell| cell.text.contains("Name") || cell.text.contains("Andreas"))
                .all(|cell| cell.style.bg.is_none())
        });
        let has_raised_code_cell = rendered.lines.iter().any(|line| {
            let text = line
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            text.contains("fn main")
                && line
                    .cells
                    .iter()
                    .any(|cell| cell.style.bg == Some(CellBg::Raised))
        });
        let has_raised_alert = rendered.lines.iter().any(|line| {
            line.cells
                .iter()
                .any(|cell| cell.text.contains("NOTE") && cell.style.bg == Some(CellBg::Raised))
        });

        assert!(has_table_border);
        assert!(table_cells_have_no_raised_bg);
        assert!(has_raised_code_cell);
        assert!(!has_raised_alert);
    }

    #[test]
    fn emoji_graphemes_keep_display_width_in_tables_and_inline_text() {
        let document = Document::new(
            None,
            "| Symbol | Meaning |\n| --- | --- |\n| ✅ | pass |\n| ⚠️ | warn |\n| 你好 | cjk |\n| 🧪🧪🧪🧪🧪🧪🧪🧪🧪🧪🧪🧪🧪 | long |\n\nIcons: ✅ ⚠️ 🧪 你好\n",
        );
        let rendered = render_document(&document, 80);
        let table_lines = rendered
            .lines
            .iter()
            .filter(|line| {
                line.cells
                    .iter()
                    .any(|cell| cell.text.contains("│") || cell.text.contains("─"))
            })
            .collect::<Vec<_>>();
        let widths = table_lines
            .iter()
            .map(|line| {
                line.cells
                    .iter()
                    .map(|cell| UnicodeWidthStr::width(cell.text.as_str()))
                    .sum::<usize>()
            })
            .collect::<Vec<_>>();

        assert!(widths.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(rendered.lines.iter().any(|line| {
            line.cells
                .iter()
                .any(|cell| cell.text == "⚠️" && UnicodeWidthStr::width(cell.text.as_str()) == 2)
        }));
        assert!(rendered.lines.iter().any(|line| {
            let text = line
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            text.contains("你好") && UnicodeWidthStr::width(text.as_str()) == widths[0]
        }));
    }

    #[test]
    fn h1_h2_emit_graphic_requests_with_text_fallback() {
        let document = Document::new(None, "# Title\n\n## Section\n\n### Small\n");
        let rendered = render_document(&document, 50);

        assert_eq!(rendered.graphics.len(), 2);
        assert!(rendered.graphics.iter().all(|graphic| graphic.image_id > 0));
        assert!(matches!(
            rendered.graphics[0].kind,
            RenderedGraphicKind::Heading { level: 1, .. }
        ));
        assert!(rendered.graphics.iter().all(|graphic| graphic.z_index < 0));
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        assert!(text.contains("TITLE"));
        assert!(text.contains("Small"));
    }

    #[test]
    fn kitty_placeholder_mode_renders_scroll_safe_heading_cells() {
        let document = Document::new(None, "# Title\n");
        let rendered = render_document_window_with_options(
            &document,
            8,
            0,
            usize::MAX,
            0,
            RenderOptions {
                kitty_placeholders: true,
                preview_graphics: false,
                image_widget_previews: false,
            },
        );

        assert_eq!(rendered.graphics.len(), 1);
        assert_eq!(rendered.lines.len(), 2);
        assert!(
            rendered
                .lines
                .iter()
                .flat_map(|line| line.cells.iter())
                .all(|cell| cell.style.fg.is_some())
        );
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        assert!(text.contains('\u{10eeee}'));
        assert!(!text.contains("TITLE"));
    }

    #[test]
    fn styled_fallback_mode_keeps_heading_text_without_placeholders() {
        let document = Document::new(None, "# Title\n");
        let rendered = render_document_window_with_options(
            &document,
            20,
            0,
            usize::MAX,
            0,
            RenderOptions {
                kitty_placeholders: false,
                preview_graphics: false,
                image_widget_previews: false,
            },
        );
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();

        assert_eq!(rendered.graphics.len(), 1);
        assert!(text.contains("TITLE"));
        assert!(!text.contains('\u{10eeee}'));
    }

    #[test]
    fn render_window_keeps_visible_heading_graphics_only() {
        let document = Document::new(None, "# Top\n\n---\n\n## Visible\n");
        let rendered = render_document_window(&document, 80, 3, 3, 0);

        assert_eq!(rendered.graphics.len(), 1);
        assert!(matches!(
            rendered.graphics[0].kind,
            RenderedGraphicKind::Heading { level: 2, .. }
        ));
    }

    #[test]
    fn local_png_images_emit_graphic_requests_with_card_fallback() {
        let document = Document::new(
            Some(PathBuf::from("/repo/docs/readme.md")),
            "![Plot](assets/chart.png)\n\n![Remote](https://example.com/image.png)\n",
        );
        let rendered = render_document(&document, 80);

        assert_eq!(rendered.graphics.len(), 1);
        assert_eq!(rendered.graphics[0].z_index, -20);
        assert!(matches!(
            &rendered.graphics[0].kind,
            RenderedGraphicKind::LocalImage { alt, source, path }
                if alt == "Plot"
                    && source == "assets/chart.png"
                    && path == &PathBuf::from("/repo/docs/assets/chart.png")
        ));
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        assert!(text.contains("[image] Plot -> assets/chart.png"));
        assert!(text.contains("[image] Remote -> https://example.com/image.png"));
    }

    #[test]
    fn diagram_previews_are_requested_then_rendered_when_warm() {
        clear_preview_cache();
        let document = Document::new(None, "```mermaid\ngraph TD;\n```\n");
        let cold = render_document_window_with_options(
            &document,
            80,
            0,
            usize::MAX,
            0,
            RenderOptions {
                kitty_placeholders: true,
                preview_graphics: true,
                image_widget_previews: false,
            },
        );

        assert_eq!(cold.pending_previews.len(), 1);
        assert!(cold.graphics.is_empty());
        assert!(warm_preview(&cold.pending_previews[0]));

        let warm = render_document_window_with_options(
            &document,
            80,
            0,
            usize::MAX,
            0,
            RenderOptions {
                kitty_placeholders: true,
                preview_graphics: true,
                image_widget_previews: false,
            },
        );

        assert!(warm.pending_previews.is_empty());
        assert_eq!(warm.graphics.len(), 1);
        assert!(matches!(
            warm.graphics[0].kind,
            RenderedGraphicKind::Preview { .. }
        ));
        assert_eq!(warm.lines.len(), 4);
        assert!(
            warm.lines
                .iter()
                .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
                .collect::<String>()
                .contains('\u{10eeee}')
        );
        clear_preview_cache();
    }

    #[test]
    fn renders_code_with_syntax_highlight_tokens() {
        let document = Document::new(None, "```rust\nfn main() {\n    let n = 1;\n}\n```\n");
        let rendered = render_document(&document, 80);
        let tokens = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.style.token))
            .collect::<Vec<_>>();

        assert!(tokens.iter().any(|token| {
            matches!(
                token,
                Token::Blue | Token::Purple | Token::Pink | Token::Success | Token::Warn
            )
        }));
    }

    #[test]
    fn renders_gfm_inline_strikethrough_links_and_emoji_shortcodes() {
        let document = Document::new(
            None,
            "This is ~~gone~~ :rocket: [docs](https://example.com)\n",
        );
        let rendered = render_document(&document, 80);
        let cells = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter())
            .collect::<Vec<_>>();
        let text = cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>();

        assert_eq!(text, "This is gone 🚀 docs");
        assert!(
            cells
                .iter()
                .any(|cell| cell.text == "g" && cell.style.struck)
        );
        assert!(cells.iter().any(|cell| {
            cell.text == "d" && cell.style.token == Token::Link && cell.style.underlined
        }));
    }

    #[test]
    fn code_highlight_cache_reuses_key() {
        let body = "fn main() {\n    let n = 1;\n}\n";
        let first = highlighted_code_lines("rust", body);
        let second = highlighted_code_lines("rust", body);

        assert_eq!(first, second);
    }

    #[test]
    fn render_document_window_stores_only_visible_rows_with_overscan() {
        let source = (0..100).map(|_| "---\n").collect::<String>();
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 50, 5, 2);

        assert_eq!(rendered.total_rows, 100);
        assert_eq!(rendered.block_index.len(), 100);
        assert_eq!(rendered.block_index[50].start_row, 50);
        assert!(rendered.lines.len() <= 9);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 48));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 57));
    }

    #[test]
    fn large_document_window_output_stays_bounded() {
        let source = (0..10_000).map(|_| "---\n").collect::<String>();
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 5_000, 25, 50);

        assert_eq!(rendered.total_rows, 10_000);
        assert!(rendered.skipped_components > 9_000);
        assert!(rendered.lines.len() <= 125);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 4_950));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 5_075));
    }

    #[test]
    fn render_window_skips_large_offscreen_blocks_but_preserves_rows() {
        let body = (0..100)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let source = format!("```rust\n{body}```\n---\n");
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 102, 1, 0);

        assert_eq!(rendered.total_rows, 103);
        assert_eq!(rendered.block_index.len(), 2);
        assert_eq!(rendered.block_index[0].start_row, 0);
        assert_eq!(rendered.block_index[0].height, 102);
        assert_eq!(rendered.block_index[1].start_row, 102);
        assert_eq!(rendered.skipped_components, 1);
        assert_eq!(rendered.lines.len(), 1);
        assert_eq!(rendered.lines[0].y_doc, 102);
        assert!(
            rendered.lines[0]
                .cells
                .iter()
                .any(|cell| cell.text.contains('─'))
        );
    }

    #[test]
    fn render_window_virtualizes_inside_large_code_blocks() {
        let body = (0..10_000)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let source = format!("```rust\n{body}```\n");
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 5_000, 5, 0);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();

        assert_eq!(rendered.total_rows, 10_002);
        assert_eq!(rendered.block_index.len(), 1);
        assert!(rendered.lines.len() <= 5);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 5_000));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 5_005));
        assert!(text.contains("line 4999"));
        assert!(!text.contains("line 0"));
        assert!(!text.contains("line 9999"));
    }

    #[test]
    fn render_window_virtualizes_inside_large_lists() {
        let source = (0..10_000)
            .map(|index| format!("- item {index}\n"))
            .collect::<String>();
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 5_000, 5, 0);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();

        assert_eq!(rendered.total_rows, 10_000);
        assert_eq!(rendered.block_index.len(), 1);
        assert!(rendered.lines.len() <= 5);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 5_000));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 5_005));
        assert!(text.contains("item 5000"));
        assert!(!text.contains("item 0"));
        assert!(!text.contains("item 9999"));
    }

    #[test]
    fn render_window_virtualizes_inside_large_tables() {
        let mut source = String::from("| Col |\n| --- |\n");
        for index in 0..10_000 {
            source.push_str(&format!("| row {index} |\n"));
        }
        let document = Document::new(None, source);
        let rendered = render_document_window(&document, 80, 5_000, 5, 0);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();

        assert_eq!(rendered.block_index.len(), 1);
        assert!(rendered.lines.len() <= 5);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 5_000));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 5_005));
        assert!(text.contains("row 4996") || text.contains("row 4997"));
        assert!(!text.contains("row 0"));
        assert!(!text.contains("row 9999"));
    }

    #[test]
    fn render_window_virtualizes_inside_large_boxed_blocks() {
        let body = (0..10_000)
            .map(|index| format!("x_{} = y_{}\n", index, index))
            .collect::<String>();
        let document = Document::new(None, format!("$$\n{body}$$\n"));
        let rendered = render_document_window(&document, 80, 5_000, 5, 0);
        let text = rendered
            .lines
            .iter()
            .flat_map(|line| line.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();

        assert_eq!(rendered.total_rows, 10_004);
        assert_eq!(rendered.block_index.len(), 1);
        assert!(rendered.lines.len() <= 5);
        assert!(rendered.lines.iter().all(|line| line.y_doc >= 5_000));
        assert!(rendered.lines.iter().all(|line| line.y_doc < 5_005));
        assert!(text.contains("x_4998") || text.contains("x_4999"));
        assert!(!text.contains("x_0"));
        assert!(!text.contains("x_9999"));
    }

    #[test]
    fn render_window_with_cache_persists_block_index_on_document() {
        let mut document = Document::new(None, "# Title\n\n---\n\nparagraph\n");
        let first =
            render_document_window_with_cache(&mut document, 80, 0, 3, 0, RenderOptions::default());
        let cached_blocks = document.layout_cache.blocks.clone();

        assert_eq!(first.block_index.len(), cached_blocks.len());
        assert_eq!(document.layout_cache.generation, document.dirty.generation);
        assert_eq!(document.layout_cache.width, 80);
        assert!(!cached_blocks.is_empty());

        let second =
            render_document_window_with_cache(&mut document, 80, 3, 3, 0, RenderOptions::default());

        assert_eq!(second.block_index.len(), cached_blocks.len());
        assert_eq!(document.layout_cache.blocks, cached_blocks);
    }

    #[test]
    #[ignore = "manual benchmark smoke test for large document window rendering"]
    fn large_document_window_render_benchmark_matrix_smoke() {
        let cases = [
            (
                "paragraph-components",
                (0..10_000)
                    .map(|index| format!("paragraph {index}\n\n"))
                    .collect::<String>(),
                5_000,
                40,
                80,
                200,
            ),
            (
                "large-code-block",
                format!(
                    "```rust\n{}```\n",
                    (0..10_000)
                        .map(|index| format!("let value_{index} = {index};\n"))
                        .collect::<String>()
                ),
                5_000,
                40,
                80,
                220,
            ),
            (
                "large-list",
                (0..10_000)
                    .map(|index| format!("- item {index}\n"))
                    .collect::<String>(),
                5_000,
                40,
                80,
                220,
            ),
            (
                "large-table",
                format!(
                    "| Col |\n| --- |\n{}",
                    (0..10_000)
                        .map(|index| format!("| row {index} |\n"))
                        .collect::<String>()
                ),
                5_000,
                40,
                80,
                220,
            ),
            (
                "large-boxed-math",
                format!(
                    "$$\n{}$$\n",
                    (0..10_000)
                        .map(|index| format!("x_{} = y_{}\n", index, index))
                        .collect::<String>()
                ),
                5_000,
                40,
                80,
                220,
            ),
        ];

        for (name, source, start_row, height, overscan, max_lines) in cases {
            let document = Document::new(None, source.clone());
            let started = std::time::Instant::now();
            let rendered = render_document_window(&document, 100, start_row, height, overscan);
            let uncached_elapsed = started.elapsed();

            let mut cached_document = Document::new(None, source);
            let _ = render_document_window_with_cache(
                &mut cached_document,
                100,
                start_row,
                height,
                overscan,
                RenderOptions::default(),
            );
            let started = std::time::Instant::now();
            let cached = render_document_window_with_cache(
                &mut cached_document,
                100,
                start_row.saturating_add(10),
                height,
                overscan,
                RenderOptions::default(),
            );
            let cached_elapsed = started.elapsed();

            assert!(rendered.lines.len() <= max_lines, "{name} uncached lines");
            assert!(cached.lines.len() <= max_lines, "{name} cached lines");
            assert_eq!(rendered.total_rows, cached.total_rows, "{name} total rows");
            eprintln!("{name}: uncached={uncached_elapsed:?} cached={cached_elapsed:?}");
        }
    }
}
