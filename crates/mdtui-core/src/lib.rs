use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub version: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            blocks: vec![Block::Paragraph(vec![Inline::Text(String::new())])],
            version: 0,
        }
    }
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks, version: 0 }
    }

    pub fn rendered_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::rendered_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn word_count(&self) -> usize {
        self.rendered_text()
            .split_whitespace()
            .filter(|word| !word.is_empty())
            .count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    BlockQuote(Vec<Block>),
    List(List),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    Table(Table),
    ThematicBreak,
    ImageBlock {
        src: String,
        alt: String,
    },
    HtmlBlock(String),
    Frontmatter(String),
}

impl Block {
    pub fn rendered_text(&self) -> String {
        match self {
            Self::Paragraph(inlines) | Self::Heading { inlines, .. } => inline_text(inlines),
            Self::BlockQuote(blocks) => blocks
                .iter()
                .map(Block::rendered_text)
                .collect::<Vec<_>>()
                .join("\n"),
            Self::List(list) => list
                .items
                .iter()
                .map(|item| item.rendered_text())
                .collect::<Vec<_>>()
                .join("\n"),
            Self::CodeBlock { text, .. } => text.clone(),
            Self::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .map(TableCell::rendered_text)
                .collect::<Vec<_>>()
                .join("\t"),
            Self::ThematicBreak => String::new(),
            Self::ImageBlock { alt, .. } => alt.clone(),
            Self::HtmlBlock(html) | Self::Frontmatter(html) => html.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditableBlockFallback {
    pub placeholder: &'static str,
}

pub fn editable_block_fallback(block: &Block) -> Option<EditableBlockFallback> {
    match block {
        Block::ThematicBreak => Some(EditableBlockFallback { placeholder: "-" }),
        _ => None,
    }
}

pub fn default_cursor_for_block(block: usize, doc_block: &Block) -> Option<Cursor> {
    match doc_block {
        Block::Paragraph(_)
        | Block::Heading { .. }
        | Block::BlockQuote(_)
        | Block::CodeBlock { .. }
        | Block::ThematicBreak
        | Block::ImageBlock { .. }
        | Block::HtmlBlock(_)
        | Block::Frontmatter(_) => Some(Cursor::Text { block, offset: 0 }),
        Block::List(list) => list.items.first().map(|_| Cursor::ListItem {
            block,
            item: 0,
            offset: 0,
        }),
        Block::Table(table) => {
            let (rows, cols) = table.dimensions();
            (rows > 0 && cols > 0).then_some(Cursor::TableCell {
                block,
                row: 0,
                col: 0,
                offset: 0,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    InlineCode(String),
    Link {
        target: String,
        title: Option<String>,
        children: Vec<Inline>,
    },
    Image {
        src: String,
        alt: String,
        title: Option<String>,
    },
    HtmlInline(String),
    SoftBreak,
    HardBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineMark {
    Emphasis,
    Strong,
    Strike,
    Code,
    Superscript,
    Subscript,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List {
    pub ordered: bool,
    pub tight: bool,
    pub items: Vec<ListItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

impl ListItem {
    pub fn paragraph(text: impl Into<String>, checked: Option<bool>) -> Self {
        Self {
            checked,
            blocks: vec![Block::Paragraph(vec![Inline::Text(text.into())])],
        }
    }

    pub fn rendered_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::rendered_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub alignments: Vec<Alignment>,
    pub rows: Vec<TableRow>,
    pub header_rows: usize,
    pub horizontal_scroll: usize,
}

impl Table {
    pub fn new(rows: Vec<Vec<String>>) -> Self {
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        Self {
            alignments: vec![Alignment::Left; width],
            rows: rows
                .into_iter()
                .map(|cells| TableRow {
                    cells: cells
                        .into_iter()
                        .map(|text| TableCell {
                            blocks: vec![Block::Paragraph(vec![Inline::Text(text)])],
                        })
                        .collect(),
                })
                .collect(),
            header_rows: 1,
            horizontal_scroll: 0,
        }
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (
            self.rows.len(),
            self.rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub blocks: Vec<Block>,
}

impl TableCell {
    pub fn rendered_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::rendered_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Text {
        block: usize,
        offset: usize,
    },
    ListItem {
        block: usize,
        item: usize,
        offset: usize,
    },
    TableCell {
        block: usize,
        row: usize,
        col: usize,
        offset: usize,
    },
    Checkbox {
        block: usize,
        item: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cursor,
    pub head: Cursor,
}

impl Selection {
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiFocus {
    Document,
    Outline,
    Workspace,
    CommandPalette,
    StylePopover,
    LinkPopover,
    TablePopover,
    AiPopover,
    WidthSlider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    InsertText {
        target: Cursor,
        text: String,
    },
    DeleteRange {
        selection: Selection,
    },
    WrapInline {
        selection: Selection,
        mark: InlineMark,
    },
    WrapLink {
        selection: Selection,
        target: String,
    },
    ToggleTask {
        block: usize,
        item: usize,
    },
    TableInsertRow {
        block: usize,
        index: usize,
    },
    TableInsertColumn {
        block: usize,
        index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub id: u64,
    pub ops: Vec<Op>,
    pub before_selection: Option<Selection>,
    pub after_selection: Option<Selection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    document: Document,
    cursor: Cursor,
    selection: Option<Selection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    pub document: Document,
    pub cursor: Cursor,
    pub selection: Option<Selection>,
    pub focus: UiFocus,
    pub show_style_popover: bool,
    pub transactions: Vec<Transaction>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    next_tx: u64,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Document::default())
    }
}

impl Editor {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            cursor: Cursor::Text {
                block: 0,
                offset: 0,
            },
            selection: None,
            focus: UiFocus::Document,
            show_style_popover: false,
            transactions: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_tx: 1,
        }
    }

    pub fn status_bar(&self, file_name: &str, width: u16) -> String {
        let (row, col) = self.logical_position();
        let selection = self
            .selection
            .filter(|selection| !selection.is_collapsed())
            .map_or(String::new(), |_| " │ selection".to_string());
        format!(
            "mdtui │ {file_name} │ row {row} col {col} │ words {}{selection} │ width {width} │ [ctrl-s] save [ctrl-q] quit [?] help",
            self.document.word_count()
        )
    }

    pub fn logical_position(&self) -> (usize, usize) {
        match self.cursor {
            Cursor::Text { block, offset } => (block + 1, offset + 1),
            Cursor::ListItem {
                block,
                item,
                offset,
            } => (block + item + 1, offset + 1),
            Cursor::TableCell {
                row, col, offset, ..
            } => (row + 1, col + offset + 1),
            Cursor::Checkbox { block, item } => (block + item + 1, 1),
        }
    }

    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
        self.selection = None;
        self.show_style_popover = false;
    }

    pub fn select_all(&mut self) {
        let len = self.active_text_len();
        self.selection = Some(Selection {
            anchor: self.with_offset(0),
            head: self.with_offset(len),
        });
        self.show_style_popover = len > 0;
    }

    pub fn select_range(&mut self, anchor: Cursor, head: Cursor) {
        self.selection = Some(Selection { anchor, head });
        self.cursor = head;
        self.show_style_popover = anchor != head;
    }

    pub fn press_char(&mut self, ch: char) {
        if self.focus != UiFocus::Document {
            return;
        }
        self.normalize_checkbox_cursor();
        let text = ch.to_string();
        if self.delete_selection_if_any() {
            self.insert_text_at_cursor(&text);
        } else {
            self.record_undo();
            self.insert_text_at_cursor(&text);
        }
        self.push_tx(vec![Op::InsertText {
            target: self.cursor,
            text,
        }]);
    }

    pub fn paste_plain_text(&mut self, text: &str) {
        self.record_undo();
        self.delete_selection_without_snapshot();
        self.insert_text_at_cursor(text);
        self.push_tx(vec![Op::InsertText {
            target: self.cursor,
            text: text.to_string(),
        }]);
    }

    pub fn enter(&mut self) {
        match self.cursor {
            Cursor::ListItem {
                block,
                item,
                offset,
            } => self.enter_list_item(block, item, offset),
            Cursor::TableCell { .. } => {
                self.record_undo();
                self.insert_text_at_cursor("\n");
            }
            Cursor::Text { block, offset } => self.split_text_block(block, offset),
            Cursor::Checkbox { .. } => {}
        }
    }

    pub fn backspace(&mut self) {
        self.normalize_checkbox_cursor();
        if self.delete_selection_if_any() {
            return;
        }
        if self.active_block_fallback().is_some() {
            self.record_undo();
            let change = self.set_active_text(String::new());
            self.apply_active_text_change(change, 0);
            self.document.version += 1;
            return;
        }

        match self.cursor {
            Cursor::ListItem {
                block,
                item,
                offset: 0,
            } => self.backspace_list_boundary(block, item),
            Cursor::TableCell { offset: 0, .. } => self.move_table_prev_cell(),
            _ if self.clamped_cursor_offset() > 0 => {
                self.record_undo();
                let offset = self.clamped_cursor_offset();
                let change = self.delete_active_text_range(offset.saturating_sub(1), offset);
                self.apply_active_text_change(change, offset.saturating_sub(1));
                self.document.version += 1;
            }
            _ => {}
        }
    }

    pub fn delete(&mut self) {
        self.normalize_checkbox_cursor();
        if self.delete_selection_if_any() {
            return;
        }
        if self.active_block_fallback().is_some() {
            self.record_undo();
            let change = self.set_active_text(String::new());
            self.apply_active_text_change(change, 0);
            self.document.version += 1;
            return;
        }
        let offset = self.clamped_cursor_offset();
        if offset < self.active_text_len() {
            self.record_undo();
            let change = self.delete_active_text_range(offset, offset + 1);
            self.apply_active_text_change(change, offset);
            self.document.version += 1;
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        self.move_horizontal(1, extend);
    }

    pub fn move_left(&mut self, extend: bool) {
        self.move_horizontal(-1, extend);
    }

    pub fn move_down(&mut self, extend: bool) {
        let next = self.next_text_cursor();
        self.apply_movement(next, extend);
    }

    pub fn move_up(&mut self, extend: bool) {
        let prev = self.previous_text_cursor();
        self.apply_movement(prev, extend);
    }

    pub fn apply_mark(&mut self, mark: InlineMark) {
        let Some(selection) = self.selection else {
            return;
        };
        self.record_undo();
        self.wrap_selection(selection, mark);
        self.show_style_popover = true;
        self.push_tx(vec![Op::WrapInline { selection, mark }]);
    }

    pub fn clear_styles(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some((start, end)) = same_owner_offsets(selection) else {
            return;
        };
        if start == end {
            return;
        }
        let (start, end) = self.expanded_style_clear_range(start, end);
        self.record_undo();
        match selection.anchor {
            Cursor::Text { block, .. } => {
                if let Some(block) = self.document.blocks.get_mut(block) {
                    clear_block_styles(block, start, end);
                }
            }
            Cursor::ListItem { block, item, .. } => {
                if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
                    && let Some(list_item) = list.items.get_mut(item)
                    && let Some(block) = list_item.blocks.get_mut(0)
                {
                    clear_block_styles(block, start, end);
                }
            }
            Cursor::TableCell {
                block, row, col, ..
            } => {
                if let Some(Block::Table(table)) = self.document.blocks.get_mut(block)
                    && let Some(block) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(col))
                        .and_then(|cell| cell.blocks.get_mut(0))
                {
                    clear_block_styles(block, start, end);
                }
            }
            Cursor::Checkbox { .. } => {}
        }
        let selection = expanded_selection(selection, start, end);
        self.selection = Some(selection);
        self.cursor = selection.head;
        self.document.version += 1;
    }

    pub fn apply_link(&mut self, target: impl Into<String>) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some((start, end)) = same_owner_offsets(selection) else {
            return;
        };
        if start == end {
            return;
        }
        let target = target.into();
        self.record_undo();
        match selection.anchor {
            Cursor::Text { block, .. } => {
                if let Some(block) = self.document.blocks.get_mut(block) {
                    wrap_block_link(block, start, end, &target);
                }
            }
            Cursor::ListItem { block, item, .. } => {
                if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
                    && let Some(list_item) = list.items.get_mut(item)
                    && let Some(block) = list_item.blocks.get_mut(0)
                {
                    wrap_block_link(block, start, end, &target);
                }
            }
            Cursor::TableCell {
                block, row, col, ..
            } => {
                if let Some(Block::Table(table)) = self.document.blocks.get_mut(block)
                    && let Some(block) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(col))
                        .and_then(|cell| cell.blocks.get_mut(0))
                {
                    wrap_block_link(block, start, end, &target);
                }
            }
            Cursor::Checkbox { .. } => {}
        }
        self.show_style_popover = true;
        self.document.version += 1;
        self.push_tx(vec![Op::WrapLink { selection, target }]);
    }

    pub fn toggle_block_quote(&mut self) {
        let Cursor::Text { block, offset } = self.cursor else {
            return;
        };
        let Some(current) = self.document.blocks.get(block).cloned() else {
            return;
        };
        self.record_undo();
        self.document.blocks[block] = match current {
            Block::BlockQuote(blocks) => blocks
                .into_iter()
                .next()
                .unwrap_or_else(|| Block::Paragraph(vec![Inline::Text(String::new())])),
            other => Block::BlockQuote(vec![Block::Paragraph(vec![Inline::Text(
                other.rendered_text(),
            )])]),
        };
        self.cursor = Cursor::Text { block, offset };
        self.document.version += 1;
    }

    pub fn toggle_code_block(&mut self) {
        let Cursor::Text { block, offset } = self.cursor else {
            return;
        };
        let Some(current) = self.document.blocks.get(block).cloned() else {
            return;
        };
        self.record_undo();
        self.document.blocks[block] = match current {
            Block::CodeBlock { text, .. } => Block::Paragraph(vec![Inline::Text(text)]),
            other => Block::CodeBlock {
                language: None,
                text: other.rendered_text(),
            },
        };
        self.cursor = Cursor::Text { block, offset };
        self.document.version += 1;
    }

    pub fn selection_covers_active_text(&self) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        let Some((start, end)) = same_owner_offsets(selection) else {
            return false;
        };
        start == 0 && end == self.active_text_len()
    }

    pub fn toggle_checkbox(&mut self, block: usize, item: usize) {
        let can_toggle = matches!(
            self.document.blocks.get(block),
            Some(Block::List(list)) if list.items.get(item).and_then(|item| item.checked).is_some()
        );
        if !can_toggle {
            return;
        }
        self.record_undo();
        if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
            && let Some(checked) = list
                .items
                .get_mut(item)
                .and_then(|item| item.checked.as_mut())
        {
            *checked = !*checked;
        }
        self.document.version += 1;
        self.cursor = Cursor::Checkbox { block, item };
        self.push_tx(vec![Op::ToggleTask { block, item }]);
    }

    pub fn space(&mut self) {
        if let Cursor::Checkbox { block, item } = self.cursor {
            self.toggle_checkbox(block, item);
        } else {
            self.press_char(' ');
        }
    }

    pub fn tab(&mut self, backwards: bool) {
        if !matches!(self.cursor, Cursor::TableCell { .. }) {
            return;
        }
        if backwards {
            self.move_table_prev_cell();
        } else {
            self.move_table_next_cell();
        }
    }

    pub fn ctrl_arrow(&mut self, direction: Direction) {
        let Cursor::TableCell {
            block, row, col, ..
        } = self.cursor
        else {
            return;
        };
        match direction {
            Direction::Right => self.insert_table_col(block, col + 1),
            Direction::Left => self.insert_table_col(block, col),
            Direction::Down => self.insert_table_row(block, row + 1),
            Direction::Up => self.insert_table_row(block, row),
        }
    }

    pub fn insert_block_at(&mut self, index: usize, block: Block, cursor: Cursor) {
        self.record_undo();
        let insert_at = index.min(self.document.blocks.len());
        self.document.blocks.insert(insert_at, block);
        self.cursor = cursor;
        self.selection = None;
        self.show_style_popover = false;
        self.document.version += 1;
    }

    pub fn remove_current_table_row(&mut self) -> bool {
        let Cursor::TableCell {
            block, row, col, ..
        } = self.cursor
        else {
            return false;
        };
        let Some(Block::Table(table)) = self.document.blocks.get(block) else {
            return false;
        };
        if table.rows.len() <= 1 {
            return false;
        }
        self.record_undo();
        let Some(Block::Table(table)) = self.document.blocks.get_mut(block) else {
            return false;
        };
        table.rows.remove(row);
        let next_row = row.min(table.rows.len().saturating_sub(1));
        let next_col = col.min(table.rows[next_row].cells.len().saturating_sub(1));
        self.cursor = Cursor::TableCell {
            block,
            row: next_row,
            col: next_col,
            offset: 0,
        };
        self.selection = None;
        self.show_style_popover = false;
        self.document.version += 1;
        true
    }

    pub fn remove_current_table_column(&mut self) -> bool {
        let Cursor::TableCell {
            block, row, col, ..
        } = self.cursor
        else {
            return false;
        };
        let Some(Block::Table(table)) = self.document.blocks.get(block) else {
            return false;
        };
        let cols = table.dimensions().1;
        if cols <= 1 {
            return false;
        }
        self.record_undo();
        let Some(Block::Table(table)) = self.document.blocks.get_mut(block) else {
            return false;
        };
        for current_row in &mut table.rows {
            if col < current_row.cells.len() {
                current_row.cells.remove(col);
            }
        }
        if col < table.alignments.len() {
            table.alignments.remove(col);
        }
        let next_col = col.min(cols.saturating_sub(2));
        self.cursor = Cursor::TableCell {
            block,
            row: row.min(table.rows.len().saturating_sub(1)),
            col: next_col,
            offset: 0,
        };
        self.selection = None;
        self.show_style_popover = false;
        self.document.version += 1;
        true
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            let current = self.snapshot();
            self.redo.push(current);
            self.restore(snapshot);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            let current = self.snapshot();
            self.undo.push(current);
            self.restore(snapshot);
        }
    }

    pub fn table_dimensions_at(&self, block: usize) -> Option<(usize, usize)> {
        match self.document.blocks.get(block) {
            Some(Block::Table(table)) => Some(table.dimensions()),
            _ => None,
        }
    }

    pub fn active_text(&self) -> String {
        match self.cursor {
            Cursor::Text { block, .. } => editable_text_of_block(self.document.blocks.get(block)),
            Cursor::ListItem { block, item, .. } => self
                .document
                .blocks
                .get(block)
                .and_then(|block| match block {
                    Block::List(list) => list.items.get(item),
                    _ => None,
                })
                .map_or_else(String::new, ListItem::rendered_text),
            Cursor::TableCell {
                block, row, col, ..
            } => self
                .document
                .blocks
                .get(block)
                .and_then(|block| match block {
                    Block::Table(table) => table.rows.get(row),
                    _ => None,
                })
                .and_then(|row| row.cells.get(col))
                .map_or_else(String::new, TableCell::rendered_text),
            Cursor::Checkbox { .. } => String::new(),
        }
    }

    fn split_text_block(&mut self, block: usize, offset: usize) {
        let Some(current) = self.document.blocks.get(block).cloned() else {
            return;
        };
        let text = current.rendered_text();
        let (left, right) = split_chars(&text, offset);
        self.record_undo();
        match current {
            Block::Heading { level, .. } => {
                self.document.blocks[block] = Block::Heading {
                    level,
                    inlines: vec![Inline::Text(left)],
                };
                self.document
                    .blocks
                    .insert(block + 1, Block::Paragraph(vec![Inline::Text(right)]));
            }
            _ => {
                self.document.blocks[block] = Block::Paragraph(vec![Inline::Text(left)]);
                self.document
                    .blocks
                    .insert(block + 1, Block::Paragraph(vec![Inline::Text(right)]));
            }
        }
        self.cursor = Cursor::Text {
            block: block + 1,
            offset: 0,
        };
        self.document.version += 1;
    }

    fn enter_list_item(&mut self, block: usize, item: usize, offset: usize) {
        let Some(Block::List(list)) = self.document.blocks.get(block) else {
            return;
        };
        let Some(current) = list.items.get(item) else {
            return;
        };
        let checked = current.checked.map(|_| false);
        let text = current.rendered_text();

        self.record_undo();
        if text.is_empty() {
            self.exit_empty_list_item(block, item);
            return;
        }

        let (left, right) = split_chars(&text, offset);
        if let Some(Block::List(list)) = self.document.blocks.get_mut(block) {
            if let Some(current) = list.items.get_mut(item) {
                current.blocks = vec![Block::Paragraph(vec![Inline::Text(left)])];
            }
            list.items
                .insert(item + 1, ListItem::paragraph(right, checked));
        }
        self.cursor = Cursor::ListItem {
            block,
            item: item + 1,
            offset: 0,
        };
        self.document.version += 1;
    }

    fn exit_empty_list_item(&mut self, block: usize, item: usize) {
        if let Some(Block::List(list)) = self.document.blocks.get_mut(block) {
            if item < list.items.len() {
                list.items.remove(item);
            }
            if list.items.is_empty() {
                self.document.blocks.remove(block);
            }
        }
        let insert_at = (block + 1).min(self.document.blocks.len());
        self.document.blocks.insert(
            insert_at,
            Block::Paragraph(vec![Inline::Text(String::new())]),
        );
        self.cursor = Cursor::Text {
            block: insert_at,
            offset: 0,
        };
        self.document.version += 1;
    }

    fn backspace_list_boundary(&mut self, block: usize, item: usize) {
        self.record_undo();
        if item == 0 {
            let Some(Block::List(list)) = self.document.blocks.get(block).cloned() else {
                return;
            };
            let first_text = list
                .items
                .first()
                .map_or_else(String::new, ListItem::rendered_text);
            self.document.blocks[block] = Block::Paragraph(vec![Inline::Text(first_text)]);
            if list.items.len() > 1 {
                let remaining = List {
                    items: list.items.into_iter().skip(1).collect(),
                    ..list
                };
                self.document
                    .blocks
                    .insert(block + 1, Block::List(remaining));
            }
            self.cursor = Cursor::Text { block, offset: 0 };
        } else if let Some(Block::List(list)) = self.document.blocks.get_mut(block) {
            let current = list
                .items
                .get(item)
                .map_or_else(String::new, ListItem::rendered_text);
            if let Some(previous) = list.items.get_mut(item - 1) {
                let old_len = char_len(&previous.rendered_text());
                previous.blocks = vec![Block::Paragraph(vec![Inline::Text(format!(
                    "{}{}",
                    previous.rendered_text(),
                    current
                ))])];
                list.items.remove(item);
                self.cursor = Cursor::ListItem {
                    block,
                    item: item - 1,
                    offset: old_len,
                };
            }
        }
        self.document.version += 1;
    }

    fn move_horizontal(&mut self, delta: isize, extend: bool) {
        let len = self.active_text_len();
        let offset = self.cursor_offset();
        let next = if delta.is_negative() {
            offset.saturating_sub(delta.unsigned_abs())
        } else {
            offset.saturating_add(delta as usize).min(len)
        };
        self.apply_movement(self.with_offset(next), extend);
    }

    fn apply_movement(&mut self, next: Cursor, extend: bool) {
        if extend {
            let anchor = self
                .selection
                .map_or(self.cursor, |selection| selection.anchor);
            self.selection = Some(Selection { anchor, head: next });
            self.show_style_popover = anchor != next;
        } else {
            self.selection = None;
            self.show_style_popover = false;
        }
        self.cursor = next;
    }

    fn active_block_fallback(&self) -> Option<EditableBlockFallback> {
        let Cursor::Text { block, .. } = self.cursor else {
            return None;
        };
        self.document
            .blocks
            .get(block)
            .and_then(editable_block_fallback)
    }

    fn apply_active_text_change(&mut self, change: ActiveTextChange, offset: usize) {
        match change {
            ActiveTextChange::Updated => {
                self.cursor = self.with_offset(offset.min(self.active_text_len()));
            }
            ActiveTextChange::Deleted(cursor) => {
                self.cursor = cursor;
                self.selection = None;
                self.show_style_popover = false;
            }
        }
    }

    fn next_text_cursor(&self) -> Cursor {
        let Cursor::Text { block, .. } = self.cursor else {
            return self.cursor;
        };
        let next_block = (block + 1).min(self.document.blocks.len().saturating_sub(1));
        Cursor::Text {
            block: next_block,
            offset: 0,
        }
    }

    fn previous_text_cursor(&self) -> Cursor {
        let Cursor::Text { block, .. } = self.cursor else {
            return self.cursor;
        };
        Cursor::Text {
            block: block.saturating_sub(1),
            offset: 0,
        }
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        let (change, new_offset) = if self.active_block_fallback().is_some() {
            (self.set_active_text(text.to_string()), char_len(text))
        } else {
            let old = self.active_text();
            let offset = self.clamped_cursor_offset();
            let new = insert_chars(&old, offset, text);
            (
                self.set_active_text(new),
                offset.saturating_add(char_len(text)),
            )
        };
        self.apply_active_text_change(change, new_offset);
        self.selection = None;
        self.show_style_popover = false;
        self.document.version += 1;
    }

    fn set_active_text(&mut self, text: String) -> ActiveTextChange {
        match self.cursor {
            Cursor::Text { block, .. } => set_block_text(&mut self.document.blocks, block, text),
            Cursor::ListItem { block, item, .. } => {
                if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
                    && let Some(list_item) = list.items.get_mut(item)
                {
                    set_list_item_text(list_item, text);
                }
                ActiveTextChange::Updated
            }
            Cursor::TableCell {
                block, row, col, ..
            } => {
                if let Some(Block::Table(table)) = self.document.blocks.get_mut(block)
                    && let Some(cell) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(col))
                {
                    cell.blocks = vec![Block::Paragraph(vec![Inline::Text(text)])];
                }
                ActiveTextChange::Updated
            }
            Cursor::Checkbox { .. } => ActiveTextChange::Updated,
        }
    }

    fn active_text_len(&self) -> usize {
        char_len(&self.active_text())
    }

    fn clamped_cursor_offset(&self) -> usize {
        self.cursor_offset().min(self.active_text_len())
    }

    fn normalize_checkbox_cursor(&mut self) {
        if let Cursor::Checkbox { block, item } = self.cursor {
            self.cursor = Cursor::ListItem {
                block,
                item,
                offset: 0,
            };
        }
    }

    fn cursor_offset(&self) -> usize {
        match self.cursor {
            Cursor::Text { offset, .. }
            | Cursor::ListItem { offset, .. }
            | Cursor::TableCell { offset, .. } => offset,
            Cursor::Checkbox { .. } => 0,
        }
    }

    fn with_offset(&self, offset: usize) -> Cursor {
        match self.cursor {
            Cursor::Text { block, .. } => Cursor::Text { block, offset },
            Cursor::ListItem { block, item, .. } => Cursor::ListItem {
                block,
                item,
                offset,
            },
            Cursor::TableCell {
                block, row, col, ..
            } => Cursor::TableCell {
                block,
                row,
                col,
                offset,
            },
            Cursor::Checkbox { block, item } => Cursor::Checkbox { block, item },
        }
    }

    fn delete_selection_if_any(&mut self) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        if selection.is_collapsed() {
            self.selection = None;
            return false;
        }
        self.record_undo();
        self.delete_selection_without_snapshot();
        self.push_tx(vec![Op::DeleteRange { selection }]);
        true
    }

    fn delete_selection_without_snapshot(&mut self) {
        let Some(selection) = self.selection.take() else {
            return;
        };
        let Some((start, end)) = same_owner_offsets(selection) else {
            return;
        };
        self.cursor = selection.anchor;
        let change = self.delete_active_text_range(start, end);
        self.apply_active_text_change(change, start);
        self.show_style_popover = false;
        self.document.version += 1;
    }

    fn wrap_selection(&mut self, selection: Selection, mark: InlineMark) {
        let Some((start, end)) = same_owner_offsets(selection) else {
            return;
        };
        if start == end {
            return;
        }
        match selection.anchor {
            Cursor::Text { block, .. } => {
                if let Some(block) = self.document.blocks.get_mut(block) {
                    wrap_block_range(block, start, end, mark);
                }
            }
            Cursor::ListItem { block, item, .. } => {
                if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
                    && let Some(list_item) = list.items.get_mut(item)
                    && let Some(block) = list_item.blocks.get_mut(0)
                {
                    wrap_block_range(block, start, end, mark);
                }
            }
            Cursor::TableCell {
                block, row, col, ..
            } => {
                if let Some(Block::Table(table)) = self.document.blocks.get_mut(block)
                    && let Some(block) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(col))
                        .and_then(|cell| cell.blocks.get_mut(0))
                {
                    wrap_block_range(block, start, end, mark);
                }
            }
            Cursor::Checkbox { .. } => {}
        }
        self.document.version += 1;
    }

    fn expanded_style_clear_range(&self, start: usize, end: usize) -> (usize, usize) {
        let chunks = match self.cursor {
            Cursor::Text { block, .. } => {
                self.document.blocks.get(block).and_then(block_style_chunks)
            }
            Cursor::ListItem { block, item, .. } => self
                .document
                .blocks
                .get(block)
                .and_then(|block| list_item_style_chunks(block, item)),
            Cursor::TableCell {
                block, row, col, ..
            } => self
                .document
                .blocks
                .get(block)
                .and_then(|block| table_cell_style_chunks(block, row, col)),
            Cursor::Checkbox { .. } => None,
        };
        chunks.as_deref().map_or((start, end), |chunks| {
            expand_style_clear_range(chunks, start, end)
        })
    }

    fn delete_active_text_range(&mut self, start: usize, end: usize) -> ActiveTextChange {
        match self.cursor {
            Cursor::Text { block, .. } => {
                delete_range_in_block(&mut self.document.blocks, block, start, end)
            }
            Cursor::ListItem { block, item, .. } => {
                if let Some(Block::List(list)) = self.document.blocks.get_mut(block)
                    && let Some(list_item) = list.items.get_mut(item)
                {
                    delete_range_in_list_item(list_item, start, end);
                }
                ActiveTextChange::Updated
            }
            Cursor::TableCell {
                block, row, col, ..
            } => {
                if let Some(Block::Table(table)) = self.document.blocks.get_mut(block)
                    && let Some(cell) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(col))
                {
                    delete_range_in_table_cell(cell, start, end);
                }
                ActiveTextChange::Updated
            }
            Cursor::Checkbox { .. } => ActiveTextChange::Updated,
        }
    }

    fn insert_table_row(&mut self, block: usize, index: usize) {
        if !matches!(self.document.blocks.get(block), Some(Block::Table(_))) {
            return;
        }
        self.record_undo();
        let Some(Block::Table(table)) = self.document.blocks.get_mut(block) else {
            return;
        };
        let (_, cols) = table.dimensions();
        let row = TableRow {
            cells: (0..cols)
                .map(|_| TableCell {
                    blocks: vec![Block::Paragraph(vec![Inline::Text(String::new())])],
                })
                .collect(),
        };
        let insert_at = index.min(table.rows.len());
        table.rows.insert(insert_at, row);
        self.cursor = Cursor::TableCell {
            block,
            row: insert_at,
            col: 0,
            offset: 0,
        };
        self.document.version += 1;
        self.push_tx(vec![Op::TableInsertRow {
            block,
            index: insert_at,
        }]);
    }

    fn insert_table_col(&mut self, block: usize, index: usize) {
        if !matches!(self.document.blocks.get(block), Some(Block::Table(_))) {
            return;
        }
        self.record_undo();
        let Some(Block::Table(table)) = self.document.blocks.get_mut(block) else {
            return;
        };
        let (_, cols) = table.dimensions();
        let insert_at = index.min(cols);
        for row in &mut table.rows {
            row.cells.insert(
                insert_at,
                TableCell {
                    blocks: vec![Block::Paragraph(vec![Inline::Text(String::new())])],
                },
            );
        }
        table.alignments.insert(insert_at, Alignment::Left);
        self.cursor = Cursor::TableCell {
            block,
            row: 0,
            col: insert_at,
            offset: 0,
        };
        self.document.version += 1;
        self.push_tx(vec![Op::TableInsertColumn {
            block,
            index: insert_at,
        }]);
    }

    fn move_table_next_cell(&mut self) {
        let Cursor::TableCell {
            block, row, col, ..
        } = self.cursor
        else {
            return;
        };
        let Some((rows, cols)) = self.table_dimensions_at(block) else {
            return;
        };
        if col + 1 < cols {
            self.cursor = Cursor::TableCell {
                block,
                row,
                col: col + 1,
                offset: 0,
            };
        } else if row + 1 < rows {
            self.cursor = Cursor::TableCell {
                block,
                row: row + 1,
                col: 0,
                offset: 0,
            };
        } else {
            self.insert_table_row(block, rows);
        }
    }

    fn move_table_prev_cell(&mut self) {
        let Cursor::TableCell {
            block, row, col, ..
        } = self.cursor
        else {
            return;
        };
        if col > 0 {
            self.cursor = Cursor::TableCell {
                block,
                row,
                col: col - 1,
                offset: 0,
            };
        } else if row > 0 {
            let last_col = self
                .table_dimensions_at(block)
                .map_or(0, |(_, cols)| cols.saturating_sub(1));
            self.cursor = Cursor::TableCell {
                block,
                row: row - 1,
                col: last_col,
                offset: 0,
            };
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            document: self.document.clone(),
            cursor: self.cursor,
            selection: self.selection,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.document = snapshot.document;
        self.cursor = snapshot.cursor;
        self.selection = snapshot.selection;
        self.show_style_popover = self
            .selection
            .is_some_and(|selection| !selection.is_collapsed());
    }

    fn record_undo(&mut self) {
        self.undo.push(self.snapshot());
        self.redo.clear();
    }

    fn push_tx(&mut self, ops: Vec<Op>) {
        let transaction = Transaction {
            id: self.next_tx,
            ops,
            before_selection: None,
            after_selection: self.selection,
        };
        self.next_tx += 1;
        self.transactions.push(transaction);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    pub enabled: bool,
    pub model: String,
    pub api_key_present: bool,
}

impl AiConfig {
    pub fn available(&self) -> bool {
        self.enabled && !self.model.is_empty() && self.api_key_present
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedResult<T> {
    pub document_version: u64,
    pub payload: T,
}

pub fn accept_worker_result<T>(document: &Document, result: &VersionedResult<T>) -> bool {
    document.version == result.document_version
}

pub fn inline_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::InlineCode(text) => {
                out.push_str(text);
            }
            Inline::HtmlInline(text)
                if matches!(
                    text.trim().to_ascii_lowercase().as_str(),
                    "<sup>" | "</sup>" | "<sub>" | "</sub>"
                ) => {}
            Inline::HtmlInline(text) => out.push_str(text),
            Inline::Emphasis(children)
            | Inline::Strong(children)
            | Inline::Strike(children)
            | Inline::Link { children, .. } => out.push_str(&inline_text(children)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push('\n'),
        }
    }
    out
}

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub fn split_chars(text: &str, at: usize) -> (String, String) {
    let split = byte_index(text, at);
    (text[..split].to_string(), text[split..].to_string())
}

pub fn insert_chars(text: &str, at: usize, insert: &str) -> String {
    let (left, right) = split_chars(text, at);
    format!("{left}{insert}{right}")
}

pub fn delete_range_chars(text: &str, start: usize, end: usize) -> String {
    let start_byte = byte_index(text, start);
    let end_byte = byte_index(text, end);
    format!("{}{}", &text[..start_byte], &text[end_byte..])
}

fn byte_index(text: &str, at: usize) -> usize {
    text.char_indices()
        .nth(at)
        .map_or(text.len(), |(index, _)| index)
}

enum ActiveTextChange {
    Updated,
    Deleted(Cursor),
}

fn editable_text_of_block(block: Option<&Block>) -> String {
    block.map_or_else(String::new, |block| {
        editable_block_fallback(block).map_or_else(
            || block.rendered_text(),
            |fallback| fallback.placeholder.to_string(),
        )
    })
}

fn set_block_text(blocks: &mut Vec<Block>, block: usize, text: String) -> ActiveTextChange {
    let Some(current) = blocks.get(block) else {
        return ActiveTextChange::Updated;
    };
    match current {
        Block::Paragraph(_) => {
            if let Some(Block::Paragraph(inlines)) = blocks.get_mut(block) {
                *inlines = vec![Inline::Text(text)];
            }
            ActiveTextChange::Updated
        }
        Block::Heading { .. } => {
            if let Some(Block::Heading { inlines, .. }) = blocks.get_mut(block) {
                *inlines = vec![Inline::Text(text)];
            }
            ActiveTextChange::Updated
        }
        Block::CodeBlock { .. } => {
            if let Some(Block::CodeBlock { text: code, .. }) = blocks.get_mut(block) {
                *code = text;
            }
            ActiveTextChange::Updated
        }
        Block::ImageBlock { .. } => {
            if let Some(Block::ImageBlock { alt, .. }) = blocks.get_mut(block) {
                *alt = text;
            }
            ActiveTextChange::Updated
        }
        Block::ThematicBreak => {
            if text.is_empty() {
                remove_block(blocks, block)
            } else if editable_block_fallback(current)
                .is_some_and(|fallback| text == fallback.placeholder)
            {
                ActiveTextChange::Updated
            } else {
                blocks[block] = Block::Paragraph(vec![Inline::Text(text)]);
                ActiveTextChange::Updated
            }
        }
        _ => ActiveTextChange::Updated,
    }
}

fn set_list_item_text(list_item: &mut ListItem, text: String) {
    if let [Block::Paragraph(inlines)] = list_item.blocks.as_mut_slice()
        && let [Inline::Link { children, .. }] = inlines.as_mut_slice()
    {
        *children = vec![Inline::Text(text)];
        return;
    }
    list_item.blocks = vec![Block::Paragraph(vec![Inline::Text(text)])];
}

fn remove_block(blocks: &mut Vec<Block>, block: usize) -> ActiveTextChange {
    if block < blocks.len() {
        blocks.remove(block);
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(vec![Inline::Text(String::new())]));
        return ActiveTextChange::Deleted(Cursor::Text {
            block: 0,
            offset: 0,
        });
    }
    let forward = block.min(blocks.len().saturating_sub(1));
    if let Some(cursor) = default_cursor_for_block(forward, &blocks[forward]) {
        return ActiveTextChange::Deleted(cursor);
    }
    for index in (0..forward).rev() {
        if let Some(cursor) = default_cursor_for_block(index, &blocks[index]) {
            return ActiveTextChange::Deleted(cursor);
        }
    }
    blocks.insert(0, Block::Paragraph(vec![Inline::Text(String::new())]));
    ActiveTextChange::Deleted(Cursor::Text {
        block: 0,
        offset: 0,
    })
}

fn same_owner_offsets(selection: Selection) -> Option<(usize, usize)> {
    if owner(selection.anchor) != owner(selection.head) {
        return None;
    }
    let a = offset(selection.anchor);
    let b = offset(selection.head);
    Some((a.min(b), a.max(b)))
}

fn owner(cursor: Cursor) -> Option<(usize, usize, usize)> {
    match cursor {
        Cursor::Text { block, .. } => Some((block, usize::MAX, usize::MAX)),
        Cursor::ListItem { block, item, .. } => Some((block, item, usize::MAX)),
        Cursor::TableCell {
            block, row, col, ..
        } => Some((block, row, col)),
        Cursor::Checkbox { .. } => None,
    }
}

fn offset(cursor: Cursor) -> usize {
    match cursor {
        Cursor::Text { offset, .. }
        | Cursor::ListItem { offset, .. }
        | Cursor::TableCell { offset, .. } => offset,
        Cursor::Checkbox { .. } => 0,
    }
}

fn cursor_with_offset(cursor: Cursor, offset: usize) -> Cursor {
    match cursor {
        Cursor::Text { block, .. } => Cursor::Text { block, offset },
        Cursor::ListItem { block, item, .. } => Cursor::ListItem {
            block,
            item,
            offset,
        },
        Cursor::TableCell {
            block, row, col, ..
        } => Cursor::TableCell {
            block,
            row,
            col,
            offset,
        },
        Cursor::Checkbox { block, item } => Cursor::Checkbox { block, item },
    }
}

fn expanded_selection(selection: Selection, start: usize, end: usize) -> Selection {
    if offset(selection.anchor) <= offset(selection.head) {
        Selection {
            anchor: cursor_with_offset(selection.anchor, start),
            head: cursor_with_offset(selection.head, end),
        }
    } else {
        Selection {
            anchor: cursor_with_offset(selection.anchor, end),
            head: cursor_with_offset(selection.head, start),
        }
    }
}

fn wrap_block_range(block: &mut Block, start: usize, end: usize, mark: InlineMark) {
    if start >= end {
        return;
    }
    let inlines = match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => inlines,
        _ => return,
    };
    let chunks = inline_chunks(inlines);
    if chunks.is_empty() {
        return;
    }
    let remove = selection_fully_marked(&chunks, start, end, mark);
    let next = apply_mark_to_chunks(&chunks, start, end, mark, remove);
    *inlines = chunks_to_inlines(&next);
}

fn block_style_chunks(block: &Block) -> Option<Vec<InlineChunk>> {
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => Some(inline_chunks(inlines)),
        _ => None,
    }
}

fn list_item_style_chunks(block: &Block, item: usize) -> Option<Vec<InlineChunk>> {
    let Block::List(list) = block else {
        return None;
    };
    let list_item = list.items.get(item)?;
    let [Block::Paragraph(inlines)] = list_item.blocks.as_slice() else {
        return None;
    };
    Some(inline_chunks(inlines))
}

fn table_cell_style_chunks(block: &Block, row: usize, col: usize) -> Option<Vec<InlineChunk>> {
    let Block::Table(table) = block else {
        return None;
    };
    let cell = table.rows.get(row)?.cells.get(col)?;
    let [Block::Paragraph(inlines)] = cell.blocks.as_slice() else {
        return None;
    };
    Some(inline_chunks(inlines))
}

fn clear_block_styles(block: &mut Block, start: usize, end: usize) {
    if start >= end {
        return;
    }
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            let chunks = inline_chunks(inlines);
            *inlines = chunks_to_inlines(&clear_styles_in_chunks(&chunks, start, end));
        }
        Block::BlockQuote(blocks) => {
            *block = Block::Paragraph(vec![Inline::Text(
                blocks
                    .first()
                    .map_or_else(String::new, Block::rendered_text),
            )]);
        }
        Block::CodeBlock { text, .. } => {
            *block = Block::Paragraph(vec![Inline::Text(text.clone())]);
        }
        _ => {}
    }
}

fn delete_range_in_block(
    blocks: &mut Vec<Block>,
    block: usize,
    start: usize,
    end: usize,
) -> ActiveTextChange {
    let Some(kind) = blocks.get(block) else {
        return ActiveTextChange::Updated;
    };
    match kind {
        Block::Paragraph(_) | Block::Heading { .. } => {
            if let Some(inlines) = paragraph_like_inlines_mut(blocks.get_mut(block)) {
                let chunks = inline_chunks(inlines);
                *inlines = chunks_to_inlines(&delete_range_in_chunks(&chunks, start, end));
            }
            ActiveTextChange::Updated
        }
        Block::CodeBlock { .. } => {
            let next = blocks.get(block).and_then(|block| match block {
                Block::CodeBlock { text, .. } => Some(delete_range_chars(text, start, end)),
                _ => None,
            });
            if let Some(Block::CodeBlock { text: code, .. }) = blocks.get_mut(block) {
                *code = next.unwrap_or_default();
            }
            ActiveTextChange::Updated
        }
        Block::ImageBlock { .. } => {
            let next = blocks.get(block).and_then(|block| match block {
                Block::ImageBlock { alt, .. } => Some(delete_range_chars(alt, start, end)),
                _ => None,
            });
            if let Some(Block::ImageBlock {
                alt: current_alt, ..
            }) = blocks.get_mut(block)
            {
                *current_alt = next.unwrap_or_default();
            }
            ActiveTextChange::Updated
        }
        Block::ThematicBreak => set_block_text(blocks, block, String::new()),
        _ => ActiveTextChange::Updated,
    }
}

fn delete_range_in_list_item(list_item: &mut ListItem, start: usize, end: usize) {
    let [Block::Paragraph(inlines)] = list_item.blocks.as_mut_slice() else {
        let next = delete_range_chars(&list_item.rendered_text(), start, end);
        set_list_item_text(list_item, next);
        return;
    };
    let chunks = inline_chunks(inlines);
    let shared_link = shared_link_meta(&chunks);
    let next = delete_range_in_chunks(&chunks, start, end);
    if let Some(link) = shared_link {
        let children = chunks_to_inlines_without_link(&next);
        *inlines = vec![Inline::Link {
            target: link.target,
            title: link.title,
            children: if children.is_empty() {
                vec![Inline::Text(String::new())]
            } else {
                children
            },
        }];
    } else {
        *inlines = chunks_to_inlines(&next);
    }
}

fn delete_range_in_table_cell(cell: &mut TableCell, start: usize, end: usize) {
    let [Block::Paragraph(inlines)] = cell.blocks.as_mut_slice() else {
        return;
    };
    let chunks = inline_chunks(inlines);
    *inlines = chunks_to_inlines(&delete_range_in_chunks(&chunks, start, end));
}

fn wrap_block_link(block: &mut Block, start: usize, end: usize, target: &str) {
    if start >= end {
        return;
    }
    let inlines = match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => inlines,
        _ => return,
    };
    let chunks = inline_chunks(inlines);
    if chunks.is_empty() {
        return;
    }
    let next = apply_link_to_chunks(&chunks, start, end, target);
    *inlines = chunks_to_inlines(&next);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineChunk {
    text: String,
    marks: ActiveMarks,
    link: Option<LinkMeta>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkMeta {
    target: String,
    title: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ActiveMarks {
    emphasis: bool,
    strong: bool,
    strike: bool,
    code: bool,
    superscript: bool,
    subscript: bool,
}

fn inline_chunks(inlines: &[Inline]) -> Vec<InlineChunk> {
    let mut out = Vec::new();
    flatten_inline_chunks(inlines, ActiveMarks::default(), None, &mut out);
    out
}

fn flatten_inline_chunks(
    inlines: &[Inline],
    current: ActiveMarks,
    link: Option<&LinkMeta>,
    out: &mut Vec<InlineChunk>,
) {
    let mut current = current;
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_inline_chunk(out, text.clone(), current, link),
            Inline::Emphasis(children) => {
                let mut next = current;
                next.emphasis = true;
                flatten_inline_chunks(children, next, link, out);
            }
            Inline::Strong(children) => {
                let mut next = current;
                next.strong = true;
                flatten_inline_chunks(children, next, link, out);
            }
            Inline::Strike(children) => {
                let mut next = current;
                next.strike = true;
                flatten_inline_chunks(children, next, link, out);
            }
            Inline::InlineCode(text) => {
                let mut next = current;
                next.code = true;
                push_inline_chunk(out, text.clone(), next, link);
            }
            Inline::Link {
                target,
                title,
                children,
            } => {
                let next = LinkMeta {
                    target: target.clone(),
                    title: title.clone(),
                };
                flatten_inline_chunks(children, current, Some(&next), out);
            }
            Inline::Image { alt, .. } => push_inline_chunk(out, alt.clone(), current, link),
            Inline::HtmlInline(html) => {
                let tag = html.trim();
                if tag.eq_ignore_ascii_case("<sup>") {
                    current.superscript = true;
                } else if tag.eq_ignore_ascii_case("</sup>") {
                    current.superscript = false;
                } else if tag.eq_ignore_ascii_case("<sub>") {
                    current.subscript = true;
                } else if tag.eq_ignore_ascii_case("</sub>") {
                    current.subscript = false;
                } else {
                    push_inline_chunk(out, html.clone(), current, link);
                }
            }
            Inline::SoftBreak => push_inline_chunk(out, "\n".to_string(), current, link),
            Inline::HardBreak => push_inline_chunk(out, "  \n".to_string(), current, link),
        }
    }
}

fn push_inline_chunk(
    out: &mut Vec<InlineChunk>,
    text: String,
    marks: ActiveMarks,
    link: Option<&LinkMeta>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.marks == marks
        && last.link.as_ref() == link
    {
        last.text.push_str(&text);
    } else {
        out.push(InlineChunk {
            text,
            marks,
            link: link.cloned(),
        });
    }
}

fn selection_fully_marked(
    chunks: &[InlineChunk],
    start: usize,
    end: usize,
    mark: InlineMark,
) -> bool {
    if start >= end {
        return false;
    }
    let mut covered = 0usize;
    let mut offset = 0usize;
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end > overlap_start {
            if !chunk_mark_enabled(chunk.marks, mark) {
                return false;
            }
            covered += overlap_end - overlap_start;
        }
        offset = chunk_end;
    }
    covered == end - start
}

fn apply_mark_to_chunks(
    chunks: &[InlineChunk],
    start: usize,
    end: usize,
    mark: InlineMark,
    remove: bool,
) -> Vec<InlineChunk> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end <= overlap_start {
            push_inline_chunk(
                &mut out,
                chunk.text.clone(),
                chunk.marks,
                chunk.link.as_ref(),
            );
            offset = chunk_end;
            continue;
        }

        let leading = overlap_start.saturating_sub(offset);
        let selected = overlap_end - overlap_start;
        let (prefix, rest) = split_chars(&chunk.text, leading);
        let (middle, suffix) = split_chars(&rest, selected);

        if !prefix.is_empty() {
            push_inline_chunk(&mut out, prefix, chunk.marks, chunk.link.as_ref());
        }
        if !middle.is_empty() {
            push_inline_chunk(
                &mut out,
                middle,
                with_chunk_mark(chunk.marks, mark, remove),
                chunk.link.as_ref(),
            );
        }
        if !suffix.is_empty() {
            push_inline_chunk(&mut out, suffix, chunk.marks, chunk.link.as_ref());
        }
        offset = chunk_end;
    }
    out
}

fn clear_styles_in_chunks(chunks: &[InlineChunk], start: usize, end: usize) -> Vec<InlineChunk> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end <= overlap_start {
            push_inline_chunk(
                &mut out,
                chunk.text.clone(),
                chunk.marks,
                chunk.link.as_ref(),
            );
            offset = chunk_end;
            continue;
        }

        let leading = overlap_start.saturating_sub(offset);
        let selected = overlap_end - overlap_start;
        let (prefix, rest) = split_chars(&chunk.text, leading);
        let (middle, suffix) = split_chars(&rest, selected);

        if !prefix.is_empty() {
            push_inline_chunk(&mut out, prefix, chunk.marks, chunk.link.as_ref());
        }
        if !middle.is_empty() {
            push_inline_chunk(&mut out, middle, ActiveMarks::default(), None);
        }
        if !suffix.is_empty() {
            push_inline_chunk(&mut out, suffix, chunk.marks, chunk.link.as_ref());
        }
        offset = chunk_end;
    }
    out
}

fn delete_range_in_chunks(chunks: &[InlineChunk], start: usize, end: usize) -> Vec<InlineChunk> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end <= overlap_start {
            push_inline_chunk(
                &mut out,
                chunk.text.clone(),
                chunk.marks,
                chunk.link.as_ref(),
            );
            offset = chunk_end;
            continue;
        }

        let leading = overlap_start.saturating_sub(offset);
        let removed = overlap_end - overlap_start;
        let (prefix, rest) = split_chars(&chunk.text, leading);
        let (_, suffix) = split_chars(&rest, removed);

        if !prefix.is_empty() {
            push_inline_chunk(&mut out, prefix, chunk.marks, chunk.link.as_ref());
        }
        if !suffix.is_empty() {
            push_inline_chunk(&mut out, suffix, chunk.marks, chunk.link.as_ref());
        }
        offset = chunk_end;
    }
    out
}

fn expand_style_clear_range(chunks: &[InlineChunk], start: usize, end: usize) -> (usize, usize) {
    let mut expanded_start = start;
    let mut expanded_end = end;
    let mut offset = 0usize;
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end > overlap_start && chunk_has_styles(chunk) {
            expanded_start = expanded_start.min(offset);
            expanded_end = expanded_end.max(chunk_end);
        }
        offset = chunk_end;
    }
    (expanded_start, expanded_end)
}

fn apply_link_to_chunks(
    chunks: &[InlineChunk],
    start: usize,
    end: usize,
    target: &str,
) -> Vec<InlineChunk> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let meta = LinkMeta {
        target: target.to_string(),
        title: None,
    };
    for chunk in chunks {
        let len = char_len(&chunk.text);
        let chunk_end = offset + len;
        let overlap_start = start.max(offset);
        let overlap_end = end.min(chunk_end);
        if overlap_end <= overlap_start {
            push_inline_chunk(
                &mut out,
                chunk.text.clone(),
                chunk.marks,
                chunk.link.as_ref(),
            );
            offset = chunk_end;
            continue;
        }

        let leading = overlap_start.saturating_sub(offset);
        let selected = overlap_end - overlap_start;
        let (prefix, rest) = split_chars(&chunk.text, leading);
        let (middle, suffix) = split_chars(&rest, selected);

        if !prefix.is_empty() {
            push_inline_chunk(&mut out, prefix, chunk.marks, chunk.link.as_ref());
        }
        if !middle.is_empty() {
            push_inline_chunk(&mut out, middle, chunk.marks, Some(&meta));
        }
        if !suffix.is_empty() {
            push_inline_chunk(&mut out, suffix, chunk.marks, chunk.link.as_ref());
        }
        offset = chunk_end;
    }
    out
}

fn with_chunk_mark(mut marks: ActiveMarks, mark: InlineMark, remove: bool) -> ActiveMarks {
    let enabled = !remove;
    match mark {
        InlineMark::Emphasis => marks.emphasis = enabled,
        InlineMark::Strong => marks.strong = enabled,
        InlineMark::Strike => marks.strike = enabled,
        InlineMark::Code => marks.code = enabled,
        InlineMark::Superscript => {
            marks.superscript = enabled;
            if enabled {
                marks.subscript = false;
            }
        }
        InlineMark::Subscript => {
            marks.subscript = enabled;
            if enabled {
                marks.superscript = false;
            }
        }
    }
    marks
}

fn chunk_mark_enabled(marks: ActiveMarks, mark: InlineMark) -> bool {
    match mark {
        InlineMark::Emphasis => marks.emphasis,
        InlineMark::Strong => marks.strong,
        InlineMark::Strike => marks.strike,
        InlineMark::Code => marks.code,
        InlineMark::Superscript => marks.superscript,
        InlineMark::Subscript => marks.subscript,
    }
}

fn chunk_has_styles(chunk: &InlineChunk) -> bool {
    chunk.link.is_some()
        || chunk.marks.emphasis
        || chunk.marks.strong
        || chunk.marks.strike
        || chunk.marks.code
        || chunk.marks.superscript
        || chunk.marks.subscript
}

fn shared_link_meta(chunks: &[InlineChunk]) -> Option<LinkMeta> {
    let mut shared = None;
    for chunk in chunks {
        let Some(link) = &chunk.link else {
            return None;
        };
        if let Some(existing) = &shared {
            if existing != link {
                return None;
            }
        } else {
            shared = Some(link.clone());
        }
    }
    shared
}

fn chunks_to_inlines_without_link(chunks: &[InlineChunk]) -> Vec<Inline> {
    let stripped: Vec<_> = chunks
        .iter()
        .map(|chunk| InlineChunk {
            text: chunk.text.clone(),
            marks: chunk.marks,
            link: None,
        })
        .collect();
    chunks_to_inlines(&stripped)
}

fn paragraph_like_inlines_mut(block: Option<&mut Block>) -> Option<&mut Vec<Inline>> {
    match block? {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => Some(inlines),
        _ => None,
    }
}

fn chunks_to_inlines(chunks: &[InlineChunk]) -> Vec<Inline> {
    let mut out = Vec::new();
    for chunk in chunks {
        if chunk.text.is_empty() {
            continue;
        }
        out.extend(inlines_for_chunk(chunk));
    }
    if out.is_empty() {
        vec![Inline::Text(String::new())]
    } else {
        merge_adjacent_text(out)
    }
}

fn inlines_for_chunk(chunk: &InlineChunk) -> Vec<Inline> {
    let mut children = if chunk.marks.code {
        vec![Inline::InlineCode(chunk.text.clone())]
    } else {
        vec![Inline::Text(chunk.text.clone())]
    };

    if chunk.marks.emphasis {
        children = vec![Inline::Emphasis(children)];
    }
    if chunk.marks.strong {
        children = vec![Inline::Strong(children)];
    }
    if chunk.marks.strike {
        children = vec![Inline::Strike(children)];
    }
    if chunk.marks.superscript {
        children = wrap_html_inline(children, "sup");
    }
    if chunk.marks.subscript {
        children = wrap_html_inline(children, "sub");
    }
    if let Some(link) = &chunk.link {
        children = vec![Inline::Link {
            target: link.target.clone(),
            title: link.title.clone(),
            children,
        }];
    }
    children
}

fn wrap_html_inline(children: Vec<Inline>, tag: &str) -> Vec<Inline> {
    let mut wrapped = Vec::with_capacity(children.len() + 2);
    wrapped.push(Inline::HtmlInline(format!("<{tag}>")));
    wrapped.extend(children);
    wrapped.push(Inline::HtmlInline(format!("</{tag}>")));
    wrapped
}

fn merge_adjacent_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut merged = Vec::new();
    for inline in inlines {
        match (merged.last_mut(), inline) {
            (Some(Inline::Text(left)), Inline::Text(right)) => left.push_str(&right),
            (_, other) => merged.push(other),
        }
    }
    merged
}

pub fn recent_transactions(editor: &Editor, limit: usize) -> VecDeque<Transaction> {
    editor
        .transactions
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph_editor(inlines: Vec<Inline>) -> Editor {
        let mut editor = Editor::new(Document::new(vec![Block::Paragraph(inlines)]));
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 0,
        });
        editor
    }

    #[test]
    fn strong_toggle_removes_existing_mark() {
        let mut editor = paragraph_editor(vec![Inline::Strong(vec![Inline::Text(
            "hello".to_string(),
        )])]);
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 0,
            },
            Cursor::Text {
                block: 0,
                offset: 5,
            },
        );

        editor.apply_mark(InlineMark::Strong);

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("hello".to_string())])]
        );
    }

    #[test]
    fn strong_toggle_merges_partial_overlap() {
        let mut editor = paragraph_editor(vec![
            Inline::Text("ab".to_string()),
            Inline::Strong(vec![Inline::Text("cd".to_string())]),
            Inline::Text("ef".to_string()),
        ]);
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 1,
            },
            Cursor::Text {
                block: 0,
                offset: 5,
            },
        );

        editor.apply_mark(InlineMark::Strong);

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![
                Inline::Text("a".to_string()),
                Inline::Strong(vec![Inline::Text("bcde".to_string())]),
                Inline::Text("f".to_string()),
            ])]
        );
    }

    #[test]
    fn superscript_toggle_round_trips_html_wrappers() {
        let mut editor = paragraph_editor(vec![Inline::Text("H2O".to_string())]);
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 1,
            },
            Cursor::Text {
                block: 0,
                offset: 2,
            },
        );

        editor.apply_mark(InlineMark::Subscript);

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![
                Inline::Text("H".to_string()),
                Inline::HtmlInline("<sub>".to_string()),
                Inline::Text("2".to_string()),
                Inline::HtmlInline("</sub>".to_string()),
                Inline::Text("O".to_string()),
            ])]
        );

        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 1,
            },
            Cursor::Text {
                block: 0,
                offset: 2,
            },
        );
        editor.apply_mark(InlineMark::Subscript);

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("H2O".to_string())])]
        );
    }

    #[test]
    fn clear_styles_strips_selected_inline_markup() {
        let mut editor = paragraph_editor(vec![
            Inline::Strong(vec![Inline::Text("ab".to_string())]),
            Inline::Text("cd".to_string()),
        ]);
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 0,
            },
            Cursor::Text {
                block: 0,
                offset: 3,
            },
        );

        editor.clear_styles();

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("abcd".to_string())])]
        );
    }

    #[test]
    fn clear_styles_expands_to_styled_boundaries() {
        let mut editor = paragraph_editor(vec![
            Inline::Strong(vec![Inline::Text("abc".to_string())]),
            Inline::Text("d".to_string()),
        ]);
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 1,
            },
            Cursor::Text {
                block: 0,
                offset: 2,
            },
        );

        editor.clear_styles();

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("abcd".to_string())])]
        );
        assert_eq!(editor.selection.and_then(same_owner_offsets), Some((0, 3)));
    }

    #[test]
    fn toggle_block_quote_wraps_and_unwraps_current_block() {
        let mut editor = paragraph_editor(vec![Inline::Text("quoted".to_string())]);
        editor.select_all();

        editor.toggle_block_quote();
        assert!(matches!(editor.document.blocks[0], Block::BlockQuote(_)));

        editor.toggle_block_quote();
        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("quoted".to_string())])]
        );
    }

    #[test]
    fn toggle_code_block_wraps_and_unwraps_current_block() {
        let mut editor = paragraph_editor(vec![Inline::Text("fn main() {}".to_string())]);
        editor.select_all();

        editor.toggle_code_block();
        assert!(matches!(editor.document.blocks[0], Block::CodeBlock { .. }));

        editor.toggle_code_block();
        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text(
                "fn main() {}".to_string()
            )])]
        );
    }

    #[test]
    fn thematic_break_uses_editable_placeholder_and_deletes_cleanly() {
        let mut editor = Editor::new(Document::new(vec![
            Block::Paragraph(vec![Inline::Text("before".to_string())]),
            Block::ThematicBreak,
            Block::Paragraph(vec![Inline::Text("after".to_string())]),
        ]));
        editor.set_cursor(Cursor::Text {
            block: 1,
            offset: 0,
        });

        assert_eq!(editor.active_text(), "-");
        let version = editor.document.version;

        editor.delete();

        assert!(editor.document.version > version);
        assert_eq!(
            editor.document.blocks,
            vec![
                Block::Paragraph(vec![Inline::Text("before".to_string())]),
                Block::Paragraph(vec![Inline::Text("after".to_string())]),
            ]
        );
        assert_eq!(
            editor.cursor,
            Cursor::Text {
                block: 1,
                offset: 0
            }
        );
    }

    #[test]
    fn backspace_on_editable_fallback_bumps_document_version() {
        let mut editor = Editor::new(Document::new(vec![Block::ThematicBreak]));
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 0,
        });
        let version = editor.document.version;

        editor.backspace();

        assert!(editor.document.version > version);
        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text(String::new())])]
        );
    }

    #[test]
    fn backspace_clamps_oversized_cursor_offsets_before_deleting() {
        let mut editor = paragraph_editor(vec![Inline::Text("hello".to_string())]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 99,
        });

        editor.backspace();

        assert_eq!(editor.document.blocks[0].rendered_text(), "hell");
        assert_eq!(
            editor.cursor,
            Cursor::Text {
                block: 0,
                offset: 4
            }
        );
    }

    #[test]
    fn backspace_on_text_bumps_document_version() {
        let mut editor = paragraph_editor(vec![Inline::Text("hello".to_string())]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 5,
        });
        let version = editor.document.version;

        editor.backspace();

        assert!(editor.document.version > version);
        assert_eq!(editor.document.blocks[0].rendered_text(), "hell");
    }

    #[test]
    fn delete_on_text_bumps_document_version() {
        let mut editor = paragraph_editor(vec![Inline::Text("hello".to_string())]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 0,
        });
        let version = editor.document.version;

        editor.delete();

        assert!(editor.document.version > version);
        assert_eq!(editor.document.blocks[0].rendered_text(), "ello");
    }

    #[test]
    fn backspace_into_styled_text_preserves_remaining_styles() {
        let mut editor = paragraph_editor(vec![
            Inline::Text("a".to_string()),
            Inline::Strong(vec![Inline::Text("bc".to_string())]),
        ]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 1,
        });

        editor.backspace();

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Strong(vec![Inline::Text(
                "bc".to_string()
            )])])]
        );
    }

    #[test]
    fn delete_into_styled_text_preserves_remaining_styles() {
        let mut editor = paragraph_editor(vec![
            Inline::Text("a".to_string()),
            Inline::Strong(vec![Inline::Text("bc".to_string())]),
        ]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 1,
        });

        editor.delete();

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![
                Inline::Text("a".to_string()),
                Inline::Strong(vec![Inline::Text("c".to_string())]),
            ])]
        );
    }

    #[test]
    fn typing_clamps_oversized_cursor_offsets_after_appending() {
        let mut editor = paragraph_editor(vec![Inline::Text("hello".to_string())]);
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 99,
        });

        editor.press_char('!');

        assert_eq!(editor.document.blocks[0].rendered_text(), "hello!");
        assert_eq!(
            editor.cursor,
            Cursor::Text {
                block: 0,
                offset: 6
            }
        );
    }

    #[test]
    fn typing_on_thematic_break_turns_it_into_paragraph_text() {
        let mut editor = Editor::new(Document::new(vec![Block::ThematicBreak]));
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 0,
        });

        editor.press_char('a');

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("a".to_string())])]
        );
        assert_eq!(
            editor.cursor,
            Cursor::Text {
                block: 0,
                offset: 1
            }
        );
    }

    #[test]
    fn backspace_on_checkbox_cursor_unindents_list_item() {
        let mut editor = Editor::new(Document::new(vec![Block::List(List {
            ordered: false,
            tight: false,
            items: vec![ListItem {
                checked: Some(true),
                blocks: vec![Block::Paragraph(vec![Inline::Text("task".to_string())])],
            }],
        })]));
        editor.set_cursor(Cursor::Checkbox { block: 0, item: 0 });

        editor.backspace();

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![Inline::Text("task".to_string())])]
        );
        assert_eq!(
            editor.cursor,
            Cursor::Text {
                block: 0,
                offset: 0
            }
        );
    }

    #[test]
    fn typing_on_checkbox_cursor_edits_list_item_text() {
        let mut editor = Editor::new(Document::new(vec![Block::List(List {
            ordered: false,
            tight: false,
            items: vec![ListItem {
                checked: Some(true),
                blocks: vec![Block::Paragraph(vec![Inline::Text("task".to_string())])],
            }],
        })]));
        editor.set_cursor(Cursor::Checkbox { block: 0, item: 0 });

        editor.press_char('A');

        assert_eq!(
            editor.document.blocks,
            vec![Block::List(List {
                ordered: false,
                tight: false,
                items: vec![ListItem {
                    checked: Some(true),
                    blocks: vec![Block::Paragraph(vec![Inline::Text("Atask".to_string())])],
                }],
            })]
        );
        assert_eq!(
            editor.cursor,
            Cursor::ListItem {
                block: 0,
                item: 0,
                offset: 1
            }
        );
    }

    #[test]
    fn typing_on_internal_link_list_item_preserves_link_target() {
        let mut editor = Editor::new(Document::new(vec![Block::List(List {
            ordered: true,
            tight: false,
            items: vec![ListItem {
                checked: None,
                blocks: vec![Block::Paragraph(vec![Inline::Link {
                    target: "#section".to_string(),
                    title: None,
                    children: vec![Inline::Text("Section".to_string())],
                }])],
            }],
        })]));
        editor.set_cursor(Cursor::ListItem {
            block: 0,
            item: 0,
            offset: 0,
        });

        editor.press_char('A');

        assert_eq!(
            editor.document.blocks,
            vec![Block::List(List {
                ordered: true,
                tight: false,
                items: vec![ListItem {
                    checked: None,
                    blocks: vec![Block::Paragraph(vec![Inline::Link {
                        target: "#section".to_string(),
                        title: None,
                        children: vec![Inline::Text("ASection".to_string())],
                    }])],
                }],
            })]
        );
    }

    #[test]
    fn delete_on_internal_link_list_item_preserves_link_target() {
        let mut editor = Editor::new(Document::new(vec![Block::List(List {
            ordered: true,
            tight: false,
            items: vec![ListItem {
                checked: None,
                blocks: vec![Block::Paragraph(vec![Inline::Link {
                    target: "#section".to_string(),
                    title: None,
                    children: vec![Inline::Text("Section".to_string())],
                }])],
            }],
        })]));
        editor.set_cursor(Cursor::ListItem {
            block: 0,
            item: 0,
            offset: 0,
        });

        editor.delete();

        assert_eq!(
            editor.document.blocks,
            vec![Block::List(List {
                ordered: true,
                tight: false,
                items: vec![ListItem {
                    checked: None,
                    blocks: vec![Block::Paragraph(vec![Inline::Link {
                        target: "#section".to_string(),
                        title: None,
                        children: vec![Inline::Text("ection".to_string())],
                    }])],
                }],
            })]
        );
    }

    #[test]
    fn deleting_entire_internal_link_list_item_text_preserves_link_target() {
        let mut editor = Editor::new(Document::new(vec![Block::List(List {
            ordered: true,
            tight: false,
            items: vec![ListItem {
                checked: None,
                blocks: vec![Block::Paragraph(vec![Inline::Link {
                    target: "#section".to_string(),
                    title: None,
                    children: vec![Inline::Text("Section".to_string())],
                }])],
            }],
        })]));
        editor.set_cursor(Cursor::ListItem {
            block: 0,
            item: 0,
            offset: 0,
        });
        editor.select_all();

        editor.delete();

        assert_eq!(
            editor.document.blocks,
            vec![Block::List(List {
                ordered: true,
                tight: false,
                items: vec![ListItem {
                    checked: None,
                    blocks: vec![Block::Paragraph(vec![Inline::Link {
                        target: "#section".to_string(),
                        title: None,
                        children: vec![Inline::Text(String::new())],
                    }])],
                }],
            })]
        );
    }

    #[test]
    fn typing_on_image_block_updates_alt_text() {
        let mut editor = Editor::new(Document::new(vec![Block::ImageBlock {
            src: "cover.png".to_string(),
            alt: "Cover".to_string(),
        }]));
        editor.set_cursor(Cursor::Text {
            block: 0,
            offset: 5,
        });

        editor.press_char('!');

        assert_eq!(
            editor.document.blocks,
            vec![Block::ImageBlock {
                src: "cover.png".to_string(),
                alt: "Cover!".to_string(),
            }]
        );
    }

    #[test]
    fn applying_link_wraps_selected_text() {
        let mut editor = Editor::new(Document::new(vec![Block::Paragraph(vec![Inline::Text(
            "hello world".to_string(),
        )])]));
        editor.select_range(
            Cursor::Text {
                block: 0,
                offset: 0,
            },
            Cursor::Text {
                block: 0,
                offset: 5,
            },
        );

        editor.apply_link("https://example.com");

        assert_eq!(
            editor.document.blocks,
            vec![Block::Paragraph(vec![
                Inline::Link {
                    target: "https://example.com".to_string(),
                    title: None,
                    children: vec![Inline::Text("hello".to_string())],
                },
                Inline::Text(" world".to_string()),
            ])]
        );
    }
}
