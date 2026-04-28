use mdtui_core::{
    Block, Cursor, Document, Editor, Inline, List, ListItem, Selection, Table, UiFocus, char_len,
    inline_text,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    TextRun,
    CursorTarget,
    SelectionRange,
    Adornment,
    TableGrid,
    ImagePlacement,
    HeadlinePlacement,
    RawHtmlAtom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayAction {
    CopyCodeBlock { block: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayItem {
    pub kind: DisplayKind,
    pub rect: Rect,
    pub cursor: Option<Cursor>,
    pub action: Option<DisplayAction>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub lines: Vec<String>,
    pub display: DisplayList,
    pub kitty_commands: Vec<String>,
}

impl Rendered {
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub width: u16,
    pub heading_width: u16,
    pub kitty_graphics: bool,
    pub columns: u8,
    pub show_status: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 80,
            heading_width: 0,
            kitty_graphics: false,
            columns: 1,
            show_status: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub app_bg: &'static str,
    pub panel_bg: &'static str,
    pub panel_raised: &'static str,
    pub active_row: &'static str,
    pub border: &'static str,
    pub border_strong: &'static str,
    pub accent_primary: &'static str,
    pub accent_highlight: &'static str,
    pub text_primary: &'static str,
    pub text_secondary: &'static str,
    pub text_muted: &'static str,
    pub success: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
    pub link: &'static str,
}

impl Theme {
    pub fn dark_amber() -> Self {
        Self {
            app_bg: "#0f0c08",
            panel_bg: "#17120c",
            panel_raised: "#21180f",
            active_row: "#5a3518",
            border: "#4a3420",
            border_strong: "#d89a4a",
            accent_primary: "#e6a85a",
            accent_highlight: "#f1b96d",
            text_primary: "#ead8bd",
            text_secondary: "#b99f7a",
            text_muted: "#7d6a50",
            success: "#9fca55",
            warning: "#e0b64f",
            error: "#d66a45",
            link: "#7da6c8",
        }
    }
}

pub fn render_document(document: &Document, options: RenderOptions) -> Rendered {
    let mut ctx = RenderContext::new(options);
    if options.columns > 1 {
        ctx.render_columns(document);
    } else {
        for (block_index, block) in document.blocks.iter().enumerate() {
            ctx.render_block(block_index, block);
        }
    }
    Rendered {
        lines: ctx.lines,
        display: ctx.display,
        kitty_commands: ctx.kitty_commands,
    }
}

pub fn render_editor(editor: &Editor, file_name: &str, options: RenderOptions) -> Rendered {
    let mut rendered = render_document(&editor.document, options);
    if editor.show_style_popover && editor.selection.is_some() && editor.focus == UiFocus::Document
    {
        rendered
            .lines
            .push("╭─ Style ─────────────────────────────╮".to_string());
        rendered
            .lines
            .push("│  B   I   S   `code`   link  H1 H2 H3 │".to_string());
        rendered
            .lines
            .push("│  • list   1. list   ☑ task   Clear   │".to_string());
        rendered
            .lines
            .push("╰──────────────────────────────────────╯".to_string());
    }
    if options.show_status {
        rendered
            .lines
            .push(editor.status_bar(file_name, options.width));
    }
    rendered
}

pub fn hit_test(x: u16, y: u16, display: &DisplayList) -> Option<Cursor> {
    display
        .items
        .iter()
        .find(|item| item.rect.contains(x, y))
        .and_then(|item| {
            item.cursor.map(|cursor| match cursor {
                Cursor::Text { block, offset } => Cursor::Text {
                    block,
                    offset: offset + usize::from(x.saturating_sub(item.rect.x)),
                },
                Cursor::ListItem {
                    block,
                    item: list_item,
                    offset,
                } => Cursor::ListItem {
                    block,
                    item: list_item,
                    offset: offset + usize::from(x.saturating_sub(item.rect.x)),
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
                    offset: offset + usize::from(x.saturating_sub(item.rect.x)),
                },
                Cursor::Checkbox { block, item } => Cursor::Checkbox { block, item },
            })
        })
}

pub fn hit_test_or_nearest(x: u16, y: u16, display: &DisplayList) -> Option<Cursor> {
    hit_test(x, y, display).or_else(|| nearest_cursor_on_row(x, y, display))
}

fn nearest_cursor_on_row(x: u16, y: u16, display: &DisplayList) -> Option<Cursor> {
    let mut best: Option<(u16, Cursor)> = None;
    for item in &display.items {
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

pub fn action_at(x: u16, y: u16, display: &DisplayList) -> Option<DisplayAction> {
    display
        .items
        .iter()
        .find(|item| item.rect.contains(x, y))
        .and_then(|item| item.action.clone())
}

pub fn position_to_cursor(pos: Cursor, display: &DisplayList) -> Option<(u16, u16)> {
    display
        .items
        .iter()
        .find(|item| item.cursor == Some(pos))
        .map(|item| (item.rect.x, item.rect.y))
}

pub fn range_to_rects(selection: Selection, display: &DisplayList) -> Vec<Rect> {
    display
        .items
        .iter()
        .filter(|item| item.cursor == Some(selection.anchor) || item.cursor == Some(selection.head))
        .map(|item| item.rect)
        .collect()
}

struct RenderContext {
    options: RenderOptions,
    lines: Vec<String>,
    display: DisplayList,
    kitty_commands: Vec<String>,
}

impl RenderContext {
    fn new(options: RenderOptions) -> Self {
        Self {
            options,
            lines: Vec::new(),
            display: DisplayList::default(),
            kitty_commands: Vec::new(),
        }
    }

    fn render_columns(&mut self, document: &Document) {
        let sep = " │ ";
        let prose: Vec<String> = document
            .blocks
            .iter()
            .map(Block::rendered_text)
            .filter(|text| !text.is_empty())
            .collect();
        let columns = usize::from(self.options.columns.max(1));
        let per_col = prose.len().div_ceil(columns).max(1);
        for row in 0..per_col {
            let mut parts = Vec::new();
            for col in 0..columns {
                let index = col * per_col + row;
                parts.push(prose.get(index).cloned().unwrap_or_default());
            }
            self.lines.push(parts.join(sep));
        }
    }

    fn render_block(&mut self, block_index: usize, block: &Block) {
        match block {
            Block::Paragraph(inlines) => self.render_text_block(block_index, inline_text(inlines)),
            Block::Heading { level, inlines } => {
                self.render_heading(block_index, *level, &inline_text(inlines));
            }
            Block::BlockQuote(blocks) => {
                for block in blocks {
                    let y = self.lines.len() as u16;
                    let text = format!("▌ {}", block.rendered_text());
                    self.push_line(
                        text,
                        DisplayKind::TextRun,
                        y,
                        Some(Cursor::Text {
                            block: block_index,
                            offset: 0,
                        }),
                    );
                }
            }
            Block::List(list) => self.render_list(block_index, list),
            Block::CodeBlock { language, text } => {
                self.render_code(block_index, language.as_deref(), text)
            }
            Block::Table(table) => self.render_table(block_index, table),
            Block::ThematicBreak => {
                self.lines
                    .push("─".repeat(usize::from(self.options.width.min(80))));
            }
            Block::ImageBlock { src, alt } => {
                self.render_image(block_index, src, alt);
            }
            Block::HtmlBlock(html) => {
                let y = self.lines.len() as u16;
                let preview = compact(html, 44);
                self.push_line(
                    format!("╭─ raw html ─╮ {preview}"),
                    DisplayKind::RawHtmlAtom,
                    y,
                    Some(Cursor::Text {
                        block: block_index,
                        offset: 0,
                    }),
                );
            }
            Block::Frontmatter(text) => self.render_text_block(block_index, text.clone()),
        }
    }

    fn render_text_block(&mut self, block_index: usize, text: String) {
        let mut offset = 0usize;
        let wrapped = wrap(&text, self.options.width.max(1));
        for (index, part) in wrapped.iter().enumerate() {
            let y = self.lines.len() as u16;
            self.push_line(
                part.clone(),
                DisplayKind::TextRun,
                y,
                Some(Cursor::Text {
                    block: block_index,
                    offset,
                }),
            );
            offset += part.chars().count();
            if index + 1 < wrapped.len() {
                offset += 1;
            }
        }
    }

    fn render_heading(&mut self, block_index: usize, level: u8, text: &str) {
        if matches!(level, 1 | 2) && self.options.kitty_graphics && text.is_ascii() {
            let width = self.options.heading_width.max(self.options.width).max(8);
            self.kitty_commands.push(format!(
                "\u{1b}_Gmdtui=headline,level={level},id={block_index}\u{1b}\\"
            ));
            let y = self.lines.len() as u16;
            let padded = " ".repeat(usize::from(width));
            self.display.items.push(DisplayItem {
                kind: DisplayKind::HeadlinePlacement,
                rect: Rect {
                    x: 0,
                    y,
                    width,
                    height: 2,
                },
                cursor: Some(Cursor::Text {
                    block: block_index,
                    offset: 0,
                }),
                action: None,
                text: text.to_string(),
            });
            self.lines.push(padded.clone());
            self.lines.push(padded);
            return;
        }
        let display = if level == 1 {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        let y = self.lines.len() as u16;
        self.push_line(
            display,
            DisplayKind::TextRun,
            y,
            Some(Cursor::Text {
                block: block_index,
                offset: 0,
            }),
        );
        let rule = if level == 1 { '═' } else { '─' };
        self.lines
            .push(rule.to_string().repeat(char_len(text).max(1)));
    }

    fn render_list(&mut self, block_index: usize, list: &List) {
        for (item_index, item) in list.items.iter().enumerate() {
            let marker = list_marker(list, item, item_index);
            let text = item.rendered_text();
            let y = self.lines.len() as u16;
            self.display.items.push(DisplayItem {
                kind: DisplayKind::Adornment,
                rect: Rect {
                    x: 0,
                    y,
                    width: marker.chars().count() as u16,
                    height: 1,
                },
                cursor: item.checked.map(|_| Cursor::Checkbox {
                    block: block_index,
                    item: item_index,
                }),
                action: None,
                text: marker.clone(),
            });
            let line = format!("{marker}{text}");
            self.display.items.push(DisplayItem {
                kind: DisplayKind::TextRun,
                rect: Rect {
                    x: marker.chars().count() as u16,
                    y,
                    width: text.chars().count() as u16,
                    height: 1,
                },
                cursor: Some(Cursor::ListItem {
                    block: block_index,
                    item: item_index,
                    offset: 0,
                }),
                action: None,
                text: text.clone(),
            });
            self.lines.push(line);
        }
    }

    fn render_code(&mut self, block_index: usize, language: Option<&str>, text: &str) {
        let width = usize::from(self.options.width.max(36));
        let button_inner = 2usize;
        let content_width = width.saturating_sub(button_inner + 6);
        let body_width = width.saturating_sub(8);
        let label = format!(
            " {}",
            compact(language.unwrap_or("code"), content_width.saturating_sub(1))
        );
        let top = format!(
            "╭{}┬{}╮",
            "─".repeat(content_width),
            "─".repeat(button_inner + 2)
        );
        let toolbar = format!("│{label:<content_width$}│ {:^button_inner$} │", "📋");
        let separator = format!(
            "├{}┼{}┤",
            "─".repeat(content_width),
            "─".repeat(button_inner + 2)
        );
        let y = self.lines.len() as u16;
        let copy_x = content_width as u16 + 2;
        self.display.items.push(DisplayItem {
            kind: DisplayKind::Adornment,
            rect: Rect {
                x: copy_x,
                y: y + 1,
                width: (button_inner + 2) as u16,
                height: 1,
            },
            cursor: None,
            action: Some(DisplayAction::CopyCodeBlock { block: block_index }),
            text: "📋".to_string(),
        });
        self.lines.push(top);
        self.lines.push(toolbar);
        self.lines.push(separator);

        let mut offset = 0usize;
        for (index, line) in text.lines().enumerate() {
            let y = self.lines.len() as u16;
            let number = format!("{:>3}", index + 1);
            let clipped = compact(line, body_width);
            let formatted = format!("│{number}│ {clipped:<body_width$} │");
            self.display.items.push(DisplayItem {
                kind: DisplayKind::TextRun,
                rect: Rect {
                    x: 6,
                    y,
                    width: clipped.chars().count() as u16,
                    height: 1,
                },
                cursor: Some(Cursor::Text {
                    block: block_index,
                    offset,
                }),
                action: None,
                text: clipped.clone(),
            });
            self.lines.push(formatted);
            offset += line.chars().count() + 1;
        }
        self.lines
            .push(format!("╰{}╯", "─".repeat(width.saturating_sub(2))));
    }

    fn render_table(&mut self, block_index: usize, table: &Table) {
        let cols = table.dimensions().1;
        let widths = table_widths(table);
        self.lines.push(table_rule('┌', '┬', '┐', &widths));
        for (row_index, row) in table.rows.iter().enumerate() {
            let y = self.lines.len() as u16;
            let mut line = String::from("│");
            for (col, width) in widths.iter().copied().enumerate().take(cols) {
                let text = row
                    .cells
                    .get(col)
                    .map_or_else(String::new, |cell| cell.rendered_text());
                let clipped = compact(&text, width);
                let x = line.chars().count() as u16;
                self.display.items.push(DisplayItem {
                    kind: DisplayKind::TableGrid,
                    rect: Rect {
                        x,
                        y,
                        width: width as u16,
                        height: 1,
                    },
                    cursor: Some(Cursor::TableCell {
                        block: block_index,
                        row: row_index,
                        col,
                        offset: 0,
                    }),
                    action: None,
                    text: clipped.clone(),
                });
                line.push(' ');
                line.push_str(&pad(&clipped, width));
                line.push(' ');
                line.push('│');
            }
            self.lines.push(line);
            if row_index + 1 == table.header_rows {
                self.lines.push(table_rule('├', '┼', '┤', &widths));
            }
        }
        self.lines.push(table_rule('└', '┴', '┘', &widths));
        if table.horizontal_scroll > 0 {
            self.lines
                .push(format!("xscroll {}", table.horizontal_scroll));
        }
    }

    fn render_image(&mut self, block_index: usize, src: &str, alt: &str) {
        if self.options.kitty_graphics {
            self.kitty_commands.push(format!(
                "\u{1b}_Gmdtui=image,id={block_index},src={src}\u{1b}\\"
            ));
        }
        let y = self.lines.len() as u16;
        self.push_line(
            format!("╭─ image ─╮ {alt} ({src})"),
            DisplayKind::ImagePlacement,
            y,
            Some(Cursor::Text {
                block: block_index,
                offset: 0,
            }),
        );
    }

    fn push_line(&mut self, line: String, kind: DisplayKind, y: u16, cursor: Option<Cursor>) {
        let width = line.chars().count() as u16;
        self.display.items.push(DisplayItem {
            kind,
            rect: Rect {
                x: 0,
                y,
                width,
                height: 1,
            },
            cursor,
            action: None,
            text: line.clone(),
        });
        self.lines.push(line);
    }
}

fn list_marker(list: &List, item: &ListItem, index: usize) -> String {
    if let Some(checked) = item.checked {
        if checked {
            "☑ ".to_string()
        } else {
            "☐ ".to_string()
        }
    } else if list.ordered {
        format!("{}. ", index + 1)
    } else {
        "• ".to_string()
    }
}

fn table_widths(table: &Table) -> Vec<usize> {
    let cols = table.dimensions().1;
    (0..cols)
        .map(|col| {
            table
                .rows
                .iter()
                .filter_map(|row| row.cells.get(col))
                .map(|cell| cell.rendered_text().chars().count().clamp(4, 32))
                .max()
                .unwrap_or(4)
        })
        .collect()
}

fn table_rule(left: char, mid: char, right: char, widths: &[usize]) -> String {
    let mut line = String::new();
    line.push(left);
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(width + 2));
        line.push(if index + 1 == widths.len() {
            right
        } else {
            mid
        });
    }
    line
}

fn compact(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else if max <= 1 {
        "…".to_string()
    } else {
        let mut out = text.chars().take(max - 1).collect::<String>();
        out.push('…');
        out
    }
}

fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

fn wrap(text: &str, width: u16) -> Vec<String> {
    let width = usize::from(width);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let needed = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if needed > width && !current.is_empty() {
            lines.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![text.to_string()]
    } else {
        lines
    }
}

pub fn rendered_inlines_without_markers(inlines: &[Inline]) -> String {
    inline_text(inlines)
}
