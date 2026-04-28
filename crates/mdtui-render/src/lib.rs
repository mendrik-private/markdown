use std::sync::OnceLock;

use hyphenation::{Hyphenator, Language, Load, Standard};
use mdtui_core::{
    Block, Cursor, Document, Editor, Inline, List, ListItem, Selection, Table, UiFocus, char_len,
    inline_text, split_chars,
};
use whatlang::{Lang, detect};

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
    CopyCodeBlock {
        block: usize,
    },
    FollowLink {
        block: usize,
    },
    ScrollCodeBlock {
        block: usize,
        track_start: u16,
        track_width: u16,
        thumb_width: u16,
        content_width: u16,
        visible_width: u16,
    },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub width: u16,
    pub heading_width: u16,
    pub kitty_graphics: bool,
    pub columns: u8,
    pub hyphenate: bool,
    pub show_status: bool,
    pub code_horizontal_scrolls: Vec<(usize, usize)>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 80,
            heading_width: 0,
            kitty_graphics: false,
            columns: 1,
            hyphenate: true,
            show_status: true,
            code_horizontal_scrolls: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub app_bg: &'static str,
    pub panel_bg: &'static str,
    pub panel_raised: &'static str,
    pub code_bg: &'static str,
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
            panel_bg: "#18120d",
            panel_raised: "#241a12",
            code_bg: "#18120d",
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
    let mut ctx = RenderContext::new(options.clone(), heading_targets(document));
    for (block_index, block) in document.blocks.iter().enumerate() {
        ctx.render_block(block_index, block);
        if block_index + 1 < document.blocks.len() {
            ctx.lines.push(String::new());
        }
    }
    Rendered {
        lines: ctx.lines,
        display: ctx.display,
        kitty_commands: ctx.kitty_commands,
    }
}

pub fn render_editor(editor: &Editor, file_name: &str, options: RenderOptions) -> Rendered {
    let mut rendered = render_document(&editor.document, options.clone());
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
    heading_targets: Vec<HeadingTarget>,
    lines: Vec<String>,
    display: DisplayList,
    kitty_commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HeadingTarget {
    slug: String,
    block: usize,
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TocEntry {
    item: usize,
    block: usize,
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WrappedTextLine {
    text: String,
    offset: usize,
    hyphenated: bool,
}

impl RenderContext {
    fn new(options: RenderOptions, heading_targets: Vec<HeadingTarget>) -> Self {
        Self {
            options,
            heading_targets,
            lines: Vec::new(),
            display: DisplayList::default(),
            kitty_commands: Vec::new(),
        }
    }

    fn render_block(&mut self, block_index: usize, block: &Block) {
        match block {
            Block::Paragraph(inlines) => self.render_text_block(block_index, inline_text(inlines)),
            Block::Heading { level, inlines } => {
                self.render_heading(block_index, *level, &inline_text(inlines));
            }
            Block::BlockQuote(blocks) => self.render_block_quote(block_index, blocks),
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
        let columns = usize::from(self.options.columns.max(1));
        if columns > 1
            && let Some((wrapped, column_width, column_height)) = text_columns(
                &text,
                self.options.width.max(1),
                columns,
                self.options.hyphenate,
            )
        {
            self.render_text_columns(block_index, &wrapped, column_width, column_height);
            return;
        }
        let wrapped = wrap_text_block(&text, self.options.width.max(1), self.options.hyphenate);
        self.render_wrapped_text_block(block_index, &wrapped);
    }

    fn render_wrapped_text_block(&mut self, block_index: usize, wrapped: &[WrappedTextLine]) {
        for line in wrapped {
            let y = self.lines.len() as u16;
            self.push_line(
                line.text.clone(),
                DisplayKind::TextRun,
                y,
                Some(Cursor::Text {
                    block: block_index,
                    offset: line.offset,
                }),
            );
            if line.hyphenated {
                let hyphen_x = line.text.chars().count() as u16;
                self.display.items.push(DisplayItem {
                    kind: DisplayKind::Adornment,
                    rect: Rect {
                        x: hyphen_x,
                        y,
                        width: 1,
                        height: 1,
                    },
                    cursor: None,
                    action: None,
                    text: "-".to_string(),
                });
                if let Some(rendered) = self.lines.last_mut() {
                    rendered.push('-');
                }
            }
        }
    }

    fn render_text_columns(
        &mut self,
        block_index: usize,
        wrapped: &[WrappedTextLine],
        column_width: usize,
        column_height: usize,
    ) {
        let columns = usize::from(self.options.columns.max(1));
        let sep = " │ ";
        let sep_width = char_len(sep);
        for row in 0..column_height {
            let y = self.lines.len() as u16;
            let mut line = String::new();
            for col in 0..columns {
                if col > 0 {
                    line.push_str(sep);
                }
                let x = (col * (column_width + sep_width)) as u16;
                let index = col * column_height + row;
                if let Some(part) = wrapped.get(index) {
                    self.display.items.push(DisplayItem {
                        kind: DisplayKind::TextRun,
                        rect: Rect {
                            x,
                            y,
                            width: part.text.chars().count() as u16,
                            height: 1,
                        },
                        cursor: Some(Cursor::Text {
                            block: block_index,
                            offset: part.offset,
                        }),
                        action: None,
                        text: part.text.clone(),
                    });
                    line.push_str(&part.text);
                    if part.hyphenated {
                        self.display.items.push(DisplayItem {
                            kind: DisplayKind::Adornment,
                            rect: Rect {
                                x: x + part.text.chars().count() as u16,
                                y,
                                width: 1,
                                height: 1,
                            },
                            cursor: None,
                            action: None,
                            text: "-".to_string(),
                        });
                        line.push('-');
                    }
                    let fill = column_width
                        .saturating_sub(char_len(&part.text) + if part.hyphenated { 1 } else { 0 });
                    if fill > 0 {
                        line.push_str(&" ".repeat(fill));
                    }
                } else {
                    line.push_str(&" ".repeat(column_width));
                }
            }
            self.lines.push(line);
        }
    }

    fn render_block_quote(&mut self, block_index: usize, blocks: &[Block]) {
        let prefix = "▌ ";
        let prefix_width = prefix.chars().count() as u16;
        let wrap_width = self.options.width.saturating_sub(prefix_width).max(1);
        let mut offset = 0usize;

        for (quote_index, block) in blocks.iter().enumerate() {
            let wrapped = wrap(&block.rendered_text(), wrap_width);
            for (line_index, part) in wrapped.iter().enumerate() {
                let y = self.lines.len() as u16;
                self.display.items.push(DisplayItem {
                    kind: DisplayKind::Adornment,
                    rect: Rect {
                        x: 0,
                        y,
                        width: prefix_width,
                        height: 1,
                    },
                    cursor: None,
                    action: None,
                    text: prefix.to_string(),
                });
                self.display.items.push(DisplayItem {
                    kind: DisplayKind::TextRun,
                    rect: Rect {
                        x: prefix_width,
                        y,
                        width: part.chars().count() as u16,
                        height: 1,
                    },
                    cursor: Some(Cursor::Text {
                        block: block_index,
                        offset,
                    }),
                    action: None,
                    text: part.clone(),
                });
                let line = format!("{prefix}{part}");
                self.lines.push(line);
                offset += part.chars().count();
                if line_index + 1 < wrapped.len() || quote_index + 1 < blocks.len() {
                    offset += 1;
                }
            }
        }
    }

    fn render_heading(&mut self, block_index: usize, level: u8, text: &str) {
        if matches!(level, 1 | 2) && self.options.kitty_graphics {
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
        self.lines.push(heading_rule(level, text));
    }

    fn render_list(&mut self, block_index: usize, list: &List) {
        if let Some(entries) = toc_entries_for_list(list, &self.heading_targets) {
            self.render_toc_list(block_index, &entries);
            return;
        }
        for (item_index, item) in list.items.iter().enumerate() {
            let marker = list_marker(list, item, item_index);
            let text = item.rendered_text();
            let marker_width = marker.chars().count() as u16;
            let wrap_width = self.options.width.saturating_sub(marker_width).max(1);
            let wrapped = wrap(&text, wrap_width);
            let indent = " ".repeat(usize::from(marker_width));
            let mut offset = 0usize;
            for (line_index, part) in wrapped.iter().enumerate() {
                let y = self.lines.len() as u16;
                if line_index == 0 {
                    self.display.items.push(DisplayItem {
                        kind: DisplayKind::Adornment,
                        rect: Rect {
                            x: 0,
                            y,
                            width: marker_width,
                            height: 1,
                        },
                        cursor: item.checked.map(|_| Cursor::Checkbox {
                            block: block_index,
                            item: item_index,
                        }),
                        action: None,
                        text: marker.clone(),
                    });
                }
                self.display.items.push(DisplayItem {
                    kind: DisplayKind::TextRun,
                    rect: Rect {
                        x: marker_width,
                        y,
                        width: part.chars().count() as u16,
                        height: 1,
                    },
                    cursor: Some(Cursor::ListItem {
                        block: block_index,
                        item: item_index,
                        offset,
                    }),
                    action: None,
                    text: part.clone(),
                });
                let line = if line_index == 0 {
                    format!("{marker}{part}")
                } else {
                    format!("{indent}{part}")
                };
                self.lines.push(line);
                offset += part.chars().count();
                if line_index + 1 < wrapped.len() {
                    offset += 1;
                }
            }
        }
    }

    fn render_toc_list(&mut self, block_index: usize, entries: &[TocEntry]) {
        let width = usize::from(self.options.width.max(12));
        for (index, entry) in entries.iter().enumerate() {
            let number = (index + 1).to_string();
            let max_title = width.saturating_sub(number.chars().count() + 3).max(1);
            let title = compact(&entry.title, max_title);
            let dots = "."
                .repeat(width.saturating_sub(title.chars().count() + number.chars().count() + 2));
            let line = format!("{title} {dots} {number}");
            let y = self.lines.len() as u16;
            self.display.items.push(DisplayItem {
                kind: DisplayKind::TextRun,
                rect: Rect {
                    x: 0,
                    y,
                    width: line.chars().count() as u16,
                    height: 1,
                },
                cursor: Some(Cursor::ListItem {
                    block: block_index,
                    item: entry.item,
                    offset: 0,
                }),
                action: Some(DisplayAction::FollowLink { block: entry.block }),
                text: line.clone(),
            });
            self.lines.push(line);
        }
    }

    fn render_code(&mut self, block_index: usize, language: Option<&str>, text: &str) {
        let width = usize::from(self.options.width.max(36));
        let copy_label = "copy";
        let button_inner = copy_label.chars().count();
        let content_width = width.saturating_sub(button_inner + 5);
        let body_width = width.saturating_sub(8);
        let scroll = code_horizontal_scroll_for(block_index, &self.options.code_horizontal_scrolls);
        let content_max_width = text
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let has_horizontal_overflow = content_max_width > body_width;
        let label = format!(
            " {}",
            compact(language.unwrap_or("code"), content_width.saturating_sub(1))
        );
        let top = format!(
            "╭{}┬{}╮",
            "─".repeat(content_width),
            "─".repeat(button_inner + 2)
        );
        let toolbar = format!("│{label:<content_width$}│ {copy_label:^button_inner$} │");
        let separator = format!(
            "├{}┴{}┤",
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
            text: copy_label.to_string(),
        });
        self.lines.push(top);
        self.lines.push(toolbar);
        self.lines.push(separator);

        let mut offset = 0usize;
        for (index, line) in text.lines().enumerate() {
            let y = self.lines.len() as u16;
            let number = format!("{:>3}", index + 1);
            let clipped = if line.chars().count() > scroll {
                clip_exact(
                    line.chars().skip(scroll).collect::<String>().as_str(),
                    body_width,
                )
            } else {
                String::new()
            };
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
        let footer_y = self.lines.len() as u16;
        self.lines.push(if has_horizontal_overflow {
            let (footer, thumb_start, thumb_width) =
                code_footer_with_thumb(width, body_width, content_max_width, scroll);
            self.display.items.push(DisplayItem {
                kind: DisplayKind::Adornment,
                rect: Rect {
                    x: thumb_start,
                    y: footer_y,
                    width: thumb_width,
                    height: 1,
                },
                cursor: None,
                action: Some(DisplayAction::ScrollCodeBlock {
                    block: block_index,
                    track_start: 1 + CODE_SCROLLBAR_GUTTER_WIDTH as u16,
                    track_width: code_scrollbar_track_width(width) as u16,
                    thumb_width,
                    content_width: content_max_width as u16,
                    visible_width: body_width as u16,
                }),
                text: "━".repeat(usize::from(thumb_width)),
            });
            footer
        } else {
            format!("╰{}╯", "─".repeat(width.saturating_sub(2)))
        });
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
            "[✗] ".to_string()
        } else {
            "[_] ".to_string()
        }
    } else if list.ordered {
        format!("{}. ", index + 1)
    } else {
        "• ".to_string()
    }
}

fn heading_targets(document: &Document) -> Vec<HeadingTarget> {
    document
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(block, item)| match item {
            Block::Heading { inlines, .. } => {
                let title = inline_text(inlines);
                Some(HeadingTarget {
                    slug: slugify_heading(&title),
                    block,
                    title,
                })
            }
            _ => None,
        })
        .collect()
}

fn toc_entries_for_list(list: &List, headings: &[HeadingTarget]) -> Option<Vec<TocEntry>> {
    list.items
        .iter()
        .enumerate()
        .map(|(item_index, item)| {
            let Block::Paragraph(inlines) = item.blocks.first()? else {
                return None;
            };
            let [
                Inline::Link {
                    target, children, ..
                },
            ] = inlines.as_slice()
            else {
                return None;
            };
            let slug = target.strip_prefix('#')?;
            let heading = headings.iter().find(|heading| heading.slug == slug)?;
            Some(TocEntry {
                item: item_index,
                block: heading.block,
                title: if heading.title.is_empty() {
                    strip_toc_numbering(&inline_text(children))
                } else {
                    strip_toc_numbering(&heading.title)
                },
            })
        })
        .collect()
}

fn strip_toc_numbering(text: &str) -> String {
    let trimmed = text.trim();
    let Some((prefix, rest)) = trimmed.split_once(' ') else {
        return trimmed.to_string();
    };
    if prefix.ends_with('.')
        && prefix[..prefix.len().saturating_sub(1)]
            .chars()
            .all(|ch| ch.is_ascii_digit())
    {
        rest.trim_start().to_string()
    } else {
        trimmed.to_string()
    }
}

fn slugify_heading(text: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in text.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch);
        } else if ch.is_whitespace() || ch == '-' {
            pending_dash = true;
        }
    }
    slug
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

fn clip_exact(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

const CODE_SCROLLBAR_GUTTER_WIDTH: usize = 5;

fn code_footer_with_thumb(
    width: usize,
    body_width: usize,
    content_width: usize,
    scroll: usize,
) -> (String, u16, u16) {
    let gutter_width = CODE_SCROLLBAR_GUTTER_WIDTH;
    let inner_width = width.saturating_sub(2);
    let track_width = code_scrollbar_track_width(width);
    let thumb_width = code_scrollbar_thumb_width(track_width, body_width, content_width);
    let max_thumb_start = track_width.saturating_sub(thumb_width);
    let max_scroll = content_width.saturating_sub(body_width);
    let thumb_offset = if max_scroll == 0 || max_thumb_start == 0 {
        0
    } else {
        scroll
            .min(max_scroll)
            .saturating_mul(max_thumb_start)
            .div_ceil(max_scroll)
    };
    (
        format!(
            "╰{}{}{}{}╯",
            "─".repeat(gutter_width.min(inner_width)),
            "─".repeat(thumb_offset),
            "━".repeat(thumb_width),
            "─".repeat(track_width.saturating_sub(thumb_offset + thumb_width))
        ),
        (1 + gutter_width + thumb_offset) as u16,
        thumb_width as u16,
    )
}

fn code_scrollbar_track_width(width: usize) -> usize {
    width.saturating_sub(2 + CODE_SCROLLBAR_GUTTER_WIDTH)
}

fn code_scrollbar_thumb_width(
    track_width: usize,
    body_width: usize,
    content_width: usize,
) -> usize {
    if content_width == 0 {
        return 0;
    }
    ((track_width.saturating_mul(body_width)).div_ceil(content_width))
        .clamp(7, 14)
        .min(track_width.max(1))
}

fn code_horizontal_scroll_for(block_index: usize, scrolls: &[(usize, usize)]) -> usize {
    scrolls
        .iter()
        .find(|(block, _)| *block == block_index)
        .map(|(_, scroll)| *scroll)
        .unwrap_or(0)
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
            char_len(word)
        } else {
            char_len(&current) + 1 + char_len(word)
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

fn wrap_text_block(text: &str, width: u16, hyphenate: bool) -> Vec<WrappedTextLine> {
    let width = usize::from(width.max(1));
    if text.is_empty() {
        return vec![WrappedTextLine {
            text: String::new(),
            offset: 0,
            hyphenated: false,
        }];
    }
    let tokens = normalized_word_tokens(text);
    if tokens.is_empty() {
        return vec![WrappedTextLine {
            text: text.to_string(),
            offset: 0,
            hyphenated: false,
        }];
    }
    let hyphenator = if hyphenate {
        detected_hyphenator(text)
    } else {
        None
    };
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_offset = 0usize;

    for token in tokens {
        let mut remaining = token.text.clone();
        let mut remaining_offset = token.start;
        loop {
            let prefix_space = if current.is_empty() { 0 } else { 1 };
            let available = width.saturating_sub(char_len(&current) + prefix_space);
            let remaining_len = char_len(&remaining);
            if remaining_len <= available {
                if current.is_empty() {
                    current_offset = remaining_offset;
                    current.push_str(&remaining);
                } else {
                    current.push(' ');
                    current.push_str(&remaining);
                }
                break;
            }

            if available > 1
                && let Some((prefix, consumed_chars)) =
                    hyphenated_prefix(&remaining, available - 1, hyphenator)
            {
                if current.is_empty() {
                    current_offset = remaining_offset;
                    current.push_str(&prefix);
                } else {
                    current.push(' ');
                    current.push_str(&prefix);
                }
                lines.push(WrappedTextLine {
                    text: current,
                    offset: current_offset,
                    hyphenated: true,
                });
                current = String::new();
                remaining = split_chars(&remaining, consumed_chars).1;
                remaining_offset += consumed_chars;
                continue;
            }

            if !current.is_empty() {
                lines.push(WrappedTextLine {
                    text: current,
                    offset: current_offset,
                    hyphenated: false,
                });
                current = String::new();
                continue;
            }

            let take = if hyphenate {
                width.saturating_sub(1).max(1)
            } else {
                width.max(1)
            }
            .min(remaining_len);
            let (prefix, suffix) = split_chars(&remaining, take);
            current_offset = remaining_offset;
            if suffix.is_empty() {
                current.push_str(&prefix);
                break;
            }
            lines.push(WrappedTextLine {
                text: prefix,
                offset: current_offset,
                hyphenated: hyphenate,
            });
            remaining = suffix;
            remaining_offset += take;
        }
    }

    if !current.is_empty() {
        lines.push(WrappedTextLine {
            text: current,
            offset: current_offset,
            hyphenated: false,
        });
    }
    if lines.is_empty() {
        vec![WrappedTextLine {
            text: text.to_string(),
            offset: 0,
            hyphenated: false,
        }]
    } else {
        lines
    }
}

fn text_columns(
    text: &str,
    width: u16,
    columns: usize,
    hyphenate: bool,
) -> Option<(Vec<WrappedTextLine>, usize, usize)> {
    if columns <= 1 {
        return None;
    }
    let total_width = usize::from(width);
    let sep_width = char_len(" │ ");
    let available = total_width.checked_sub(sep_width * columns.saturating_sub(1))?;
    let column_width = available / columns;
    if column_width < 16 {
        return None;
    }
    let wrapped = wrap_text_block(text, column_width as u16, hyphenate);
    if !column_layout_is_balanced(wrapped.len(), columns) {
        return None;
    }
    let column_height = wrapped.len().div_ceil(columns);
    Some((wrapped, column_width, column_height))
}

fn column_layout_is_balanced(total_lines: usize, columns: usize) -> bool {
    if columns <= 1 || total_lines == 0 {
        return false;
    }
    let column_height = total_lines.div_ceil(columns);
    if column_height < 4 {
        return false;
    }
    let trailing = total_lines % column_height;
    trailing == 0 || trailing + 2 >= column_height
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WordToken {
    text: String,
    start: usize,
}

fn normalized_word_tokens(text: &str) -> Vec<WordToken> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for word in text.split_whitespace() {
        out.push(WordToken {
            text: word.to_string(),
            start: offset,
        });
        offset += char_len(word) + 1;
    }
    out
}

fn heading_rule(level: u8, text: &str) -> String {
    let rule = match level {
        1 => '═',
        2 => '─',
        3 => '🬂',
        4 => '🭶',
        _ => '‾',
    };
    rule.to_string().repeat(char_len(text).max(1))
}

fn hyphenated_prefix(
    word: &str,
    max_chars: usize,
    hyphenator: Option<&'static Standard>,
) -> Option<(String, usize)> {
    let hyphenator = hyphenator?;
    let mut prefix = String::new();
    let mut consumed_chars = 0usize;
    let total_chars = char_len(word);
    let mut best = None;
    for segment in hyphenator.hyphenate(word).into_iter().segments() {
        let segment_chars = char_len(segment);
        if consumed_chars + segment_chars > max_chars {
            break;
        }
        prefix.push_str(segment);
        consumed_chars += segment_chars;
        if consumed_chars < total_chars {
            best = Some((prefix.clone(), consumed_chars));
        }
    }
    best
}

fn detected_hyphenator(text: &str) -> Option<&'static Standard> {
    detect(text)
        .filter(|info| info.is_reliable())
        .and_then(|info| hyphenator_for_lang(info.lang()))
}

fn hyphenator_for_lang(lang: Lang) -> Option<&'static Standard> {
    match lang {
        Lang::Eng => english_us_hyphenator(),
        Lang::Fra => french_hyphenator(),
        Lang::Deu => german_hyphenator(),
        Lang::Spa => spanish_hyphenator(),
        Lang::Por => portuguese_hyphenator(),
        Lang::Ita => italian_hyphenator(),
        Lang::Nld => dutch_hyphenator(),
        Lang::Pol => polish_hyphenator(),
        Lang::Rus => russian_hyphenator(),
        _ => None,
    }
}

fn english_us_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::EnglishUS).ok())
        .as_ref()
}

fn french_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::French).ok())
        .as_ref()
}

fn german_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::German1996).ok())
        .as_ref()
}

fn spanish_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Spanish).ok())
        .as_ref()
}

fn portuguese_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Portuguese).ok())
        .as_ref()
}

fn italian_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Italian).ok())
        .as_ref()
}

fn dutch_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Dutch).ok())
        .as_ref()
}

fn polish_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Polish).ok())
        .as_ref()
}

fn russian_hyphenator() -> Option<&'static Standard> {
    static DICT: OnceLock<Option<Standard>> = OnceLock::new();
    DICT.get_or_init(|| Standard::from_embedded(Language::Russian).ok())
        .as_ref()
}

pub fn rendered_inlines_without_markers(inlines: &[Inline]) -> String {
    inline_text(inlines)
}
