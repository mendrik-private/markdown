use std::{
    ops::Range,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use ropey::Rope;
use serde::{Deserialize, Serialize};
use tree_sitter::{InputEdit, Node, Point};
use tree_sitter_md::{MarkdownParser, MarkdownTree};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}

impl SourceRange {
    pub fn contains(&self, byte: usize) -> bool {
        self.start <= byte && byte <= self.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStyle {
    Inline,
    Reference,
    Autolink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagramLanguage {
    Mermaid,
    GeoJson,
    TopoJson,
    Stl,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentKind {
    Paragraph,
    Heading { level: u8 },
    ThematicBreak,
    BlockQuote,
    Alert { level: AlertLevel },
    List { ordered: bool, tight: bool },
    ListItem { checked: Option<bool> },
    CodeBlock { fenced: bool, language: String },
    Table,
    TableRow { header: bool },
    TableCell { row: usize, col: usize },
    Link { style: LinkStyle },
    Image,
    FootnoteRef,
    FootnoteDef,
    HtmlBlock,
    MathBlock,
    DiagramBlock { language: DiagramLanguage },
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub id: ComponentId,
    pub kind: ComponentKind,
    pub source: SourceRange,
    pub fields: Vec<FieldId>,
    pub children: Vec<ComponentId>,
    pub render_hash: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentArena {
    pub components: Vec<Component>,
}

impl ComponentArena {
    pub fn component_at_byte(&self, byte: usize) -> Option<&Component> {
        self.components
            .iter()
            .find(|component| component.source.contains(byte))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorAffinity {
    #[default]
    Downstream,
    Upstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentSlot {
    Before,
    Body,
    After,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorTarget {
    TextField {
        field: FieldId,
        grapheme: usize,
        affinity: CursorAffinity,
    },
    Component {
        component: ComponentId,
        slot: ComponentSlot,
    },
    Gap {
        before: Option<ComponentId>,
        after: Option<ComponentId>,
    },
    Action {
        action: ActionId,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCursor {
    pub byte: usize,
    pub desired_col: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRange {
    pub anchor: usize,
    pub focus: usize,
}

impl SelectionRange {
    pub fn ordered(self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyState {
    pub is_dirty: bool,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownParse {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub tree_sitter: TreeSitterParse,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSitterParse {
    pub parsed: bool,
    pub root_kind: String,
    pub has_error: bool,
    pub reused_previous_tree: bool,
    pub changed_range_count: usize,
    pub block_node_count: usize,
    pub inline_tree_count: usize,
    pub named_nodes: Vec<TreeSitterNode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSitterNode {
    pub kind: String,
    pub source: SourceRange,
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: SourceRange,
    pub message: String,
    pub fix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlockFields {
    pub language: SourceRange,
    pub body: SourceRange,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutCache {
    pub generation: u64,
    pub preview_generation: u64,
    pub width: u16,
    pub kitty_placeholders: bool,
    pub preview_graphics: bool,
    pub blocks: Vec<LayoutBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutBlock {
    pub component: ComponentId,
    pub start_row: usize,
    pub height: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoStack {
    undo: Vec<String>,
    redo: Vec<String>,
}

impl UndoStack {
    fn remember(&mut self, source: String) {
        if self.undo.last() != Some(&source) {
            self.undo.push(source);
        }
        self.redo.clear();
    }

    fn undo(&mut self, current: String) -> Option<String> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        Some(previous)
    }

    fn redo(&mut self, current: String) -> Option<String> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }
}

#[derive(Default)]
struct TreeSitterCache {
    tree: Option<MarkdownTree>,
}

impl Clone for TreeSitterCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for TreeSitterCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSitterCache")
            .field("has_tree", &self.tree.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorSnapshot {
    pub byte: usize,
    pub line: usize,
    pub column: usize,
    pub component: Option<ComponentKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FootnoteTarget {
    label: String,
    label_range: Range<usize>,
    full_range: Range<usize>,
    definition: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FootnoteDefinition {
    label: String,
    label_range: Range<usize>,
    full_range: Range<usize>,
    content_start: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageTarget {
    pub alt: String,
    pub source: String,
    pub title: Option<String>,
    pub full_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingTarget {
    pub level: u8,
    pub text: String,
    pub full_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    pub label: String,
    pub destination: String,
    pub title: Option<String>,
    pub reference_label: Option<String>,
    pub full_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkReferenceDefinition {
    label: String,
    destination: String,
    title: Option<String>,
    full_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkReferenceTarget {
    reference_label: String,
    full_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub id: DocumentId,
    pub path: Option<PathBuf>,
    pub text: Rope,
    pub parse: MarkdownParse,
    pub components: ComponentArena,
    pub layout_cache: LayoutCache,
    pub history: UndoStack,
    pub dirty: DirtyState,
    pub cursor: SourceCursor,
    pub selection: Option<SelectionRange>,
    tree_sitter_cache: TreeSitterCache,
}

impl Document {
    pub fn new(path: Option<PathBuf>, source: impl AsRef<str>) -> Self {
        let mut document = Self {
            id: DocumentId(NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed)),
            path,
            text: Rope::from_str(source.as_ref()),
            parse: MarkdownParse {
                ok: true,
                diagnostics: Vec::new(),
                tree_sitter: TreeSitterParse::default(),
            },
            components: ComponentArena::default(),
            layout_cache: LayoutCache::default(),
            history: UndoStack::default(),
            dirty: DirtyState::default(),
            cursor: SourceCursor::default(),
            selection: None,
            tree_sitter_cache: TreeSitterCache::default(),
        };
        document.rebuild_semantics();
        document
    }

    pub fn source(&self) -> String {
        self.text.to_string()
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    pub fn mark_saved(&mut self) {
        self.dirty.is_dirty = false;
    }

    pub fn cursor_snapshot(&self) -> CursorSnapshot {
        let (line, column) = self.line_column_for_byte(self.cursor.byte);
        CursorSnapshot {
            byte: self.cursor.byte,
            line,
            column,
            component: self
                .components
                .component_at_byte(self.cursor.byte)
                .map(|component| component.kind.clone()),
        }
    }

    pub fn set_cursor_byte(&mut self, byte: usize) {
        self.set_cursor_byte_preserving_selection(byte);
        self.selection = None;
    }

    pub fn selection_range(&self) -> Option<Range<usize>> {
        self.selection
            .map(SelectionRange::ordered)
            .filter(|range| range.start < range.end)
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        let source = self.source();
        let anchor = clamp_to_grapheme_boundary(&source, anchor.min(source.len()));
        let focus = clamp_to_grapheme_boundary(&source, focus.min(source.len()));
        self.cursor.byte = focus;
        self.cursor.desired_col = None;
        self.selection = (anchor != focus).then_some(SelectionRange { anchor, focus });
    }

    pub fn select_all(&mut self) {
        self.cursor.byte = self.len_bytes();
        self.cursor.desired_col = None;
        self.selection = Some(SelectionRange {
            anchor: 0,
            focus: self.cursor.byte,
        });
    }

    pub fn insert_char(&mut self, ch: char) {
        let mut text = String::new();
        text.push(ch);
        self.insert_text(&text);
    }

    pub fn insert_text(&mut self, inserted: &str) {
        self.edit(|document| {
            if let Some(range) = document.selection_range() {
                let start = document.byte_to_char(range.start);
                let end = document.byte_to_char(range.end);
                document.text.remove(start..end);
                document.text.insert(start, inserted);
                document.cursor.byte = range.start + inserted.len();
                document.selection = None;
                return;
            }
            let char_index = document.byte_to_char(document.cursor.byte);
            document.text.insert(char_index, inserted);
            document.cursor.byte += inserted.len();
        });
    }

    pub fn enter(&mut self) {
        self.insert_text("\n");
    }

    pub fn alt_enter(&mut self) {
        self.insert_text("  \n");
    }

    pub fn insert_paragraph_at_gap(&mut self) {
        let byte = self.cursor.byte;
        let prefix = if byte > 0 && !self.source()[..byte].ends_with('\n') {
            "\n\n"
        } else {
            ""
        };
        self.insert_text(prefix);
    }

    pub fn insert_block_at_cursor(&mut self, markdown: &str, cursor_offset: usize) {
        let source = self.source();
        let cursor = clamp_to_grapheme_boundary(&source, self.cursor.byte.min(source.len()));
        let block = markdown.trim_end_matches('\n');
        let gap_range = self
            .components
            .component_at_byte(cursor)
            .filter(|component| matches!(component.kind, ComponentKind::Gap))
            .map(|component| {
                component.source.start.min(source.len())..component.source.end.min(source.len())
            });

        let (range, replacement, inserted_start) = if let Some(range) = gap_range {
            let before_has_content = !source[..range.start].trim().is_empty();
            let after_has_content = !source[range.end..].trim().is_empty();
            let mut replacement = String::new();
            if before_has_content {
                replacement.push('\n');
            }
            let inserted_start = range.start + replacement.len();
            replacement.push_str(block);
            replacement.push('\n');
            if after_has_content {
                replacement.push('\n');
            }
            (range, replacement, inserted_start)
        } else {
            let before = &source[..cursor];
            let after = &source[cursor..];
            let prefix = block_insert_prefix(before);
            let suffix = block_insert_suffix(after);
            let inserted_start = cursor + prefix.len();
            (
                cursor..cursor,
                format!("{prefix}{block}{suffix}"),
                inserted_start,
            )
        };

        self.edit(|document| {
            let start = document.byte_to_char(range.start);
            let end = document.byte_to_char(range.end);
            document.text.remove(start..end);
            document.text.insert(start, &replacement);
            document.cursor.byte =
                (inserted_start + cursor_offset.min(block.len())).min(document.len_bytes());
            document.selection = None;
        });
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor.byte == 0 {
            return;
        }
        self.edit(|document| {
            let source = document.source();
            let previous = previous_grapheme_boundary(&source, document.cursor.byte);
            let start = document.byte_to_char(previous);
            let end = document.byte_to_char(document.cursor.byte);
            document.text.remove(start..end);
            document.cursor.byte = previous;
        });
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor.byte >= self.len_bytes() {
            return;
        }
        self.edit(|document| {
            let source = document.source();
            let next = next_grapheme_boundary(&source, document.cursor.byte);
            let start = document.byte_to_char(document.cursor.byte);
            let end = document.byte_to_char(next);
            document.text.remove(start..end);
        });
    }

    pub fn move_left(&mut self) {
        self.selection = None;
        let source = self.source();
        self.cursor.byte = previous_grapheme_boundary(&source, self.cursor.byte);
        self.cursor.desired_col = None;
    }

    pub fn move_right(&mut self) {
        self.selection = None;
        let source = self.source();
        self.cursor.byte = next_grapheme_boundary(&source, self.cursor.byte);
        self.cursor.desired_col = None;
    }

    pub fn move_word_left(&mut self) {
        self.selection = None;
        let source = self.source();
        let before = &source[..self.cursor.byte.min(source.len())];
        let mut target = 0;
        for (index, grapheme) in before.grapheme_indices(true) {
            if index >= self.cursor.byte {
                break;
            }
            if !grapheme.chars().all(char::is_whitespace) {
                target = index;
            }
        }
        self.cursor.byte = target;
        self.cursor.desired_col = None;
    }

    pub fn move_word_right(&mut self) {
        self.selection = None;
        let source = self.source();
        let after = &source[self.cursor.byte.min(source.len())..];
        let mut seen_word = false;
        for (offset, grapheme) in after.grapheme_indices(true) {
            let is_space = grapheme.chars().all(char::is_whitespace);
            if seen_word && is_space {
                self.cursor.byte += offset;
                self.cursor.desired_col = None;
                return;
            }
            seen_word |= !is_space;
        }
        self.cursor.byte = source.len();
        self.cursor.desired_col = None;
    }

    pub fn move_up(&mut self) {
        self.selection = None;
        self.move_vertical(-1);
    }

    pub fn move_down(&mut self) {
        self.selection = None;
        self.move_vertical(1);
    }

    pub fn move_left_select(&mut self) {
        self.start_selection();
        let source = self.source();
        self.cursor.byte = previous_grapheme_boundary(&source, self.cursor.byte);
        self.cursor.desired_col = None;
        self.update_selection_focus();
    }

    pub fn move_right_select(&mut self) {
        self.start_selection();
        let source = self.source();
        self.cursor.byte = next_grapheme_boundary(&source, self.cursor.byte);
        self.cursor.desired_col = None;
        self.update_selection_focus();
    }

    pub fn move_up_select(&mut self) {
        self.start_selection();
        self.move_vertical(-1);
        self.update_selection_focus();
    }

    pub fn move_down_select(&mut self) {
        self.start_selection();
        self.move_vertical(1);
        self.update_selection_focus();
    }

    pub fn structural_move(&mut self, forward: bool) {
        let current = self.cursor.byte;
        let candidate = if forward {
            self.components
                .components
                .iter()
                .find(|component| component.source.start > current)
                .map(|component| component.source.start)
        } else {
            self.components
                .components
                .iter()
                .rev()
                .find(|component| component.source.end < current)
                .map(|component| component.source.start)
        };
        if let Some(byte) = candidate {
            self.set_cursor_byte(byte);
        }
    }

    pub fn toggle_task_at_cursor(&mut self) -> bool {
        let source = self.source();
        let range = self.current_line_range();
        let line = &source[range.clone()];
        let Some(marker_start) = line
            .find("[ ]")
            .or_else(|| line.find("[x]"))
            .or_else(|| line.find("[X]"))
        else {
            return false;
        };
        let checked = &line[marker_start..marker_start + 3] != "[ ]";
        self.edit(|document| {
            let start = document.byte_to_char(range.start + marker_start + 1);
            let end = document.byte_to_char(range.start + marker_start + 2);
            document.text.remove(start..end);
            document.text.insert(start, if checked { " " } else { "x" });
        });
        true
    }

    pub fn set_code_language_at_cursor(&mut self, language: &str) -> bool {
        let Some(component) = self.component_at_cursor().cloned() else {
            return false;
        };
        if !matches!(
            component.kind,
            ComponentKind::CodeBlock { .. } | ComponentKind::DiagramBlock { .. }
        ) {
            return false;
        }
        let source = self.source();
        let Some(first_line_end) = source[component.source.start..component.source.end].find('\n')
        else {
            return false;
        };
        let line_start = component.source.start;
        let line_end = line_start + first_line_end;
        let fence_line = &source[line_start..line_end];
        let fence_len = fence_line
            .chars()
            .take_while(|ch| *ch == '`' || *ch == '~')
            .count();
        if fence_len < 3 {
            return false;
        }
        let replacement = format!("{}{}", &fence_line[..fence_len], language.trim());
        self.replace_byte_range(line_start..line_end, &replacement);
        true
    }

    pub fn heading_at_cursor(&self) -> Option<HeadingTarget> {
        let component = self.component_at_cursor()?.clone();
        let ComponentKind::Heading { level } = component.kind else {
            return None;
        };
        let source = self.source();
        let raw = &source[component.source.start..component.source.end];
        Some(HeadingTarget {
            level,
            text: heading_text(raw),
            full_range: component.source.start..component.source.end,
        })
    }

    pub fn set_heading_at_cursor(&mut self, level: u8, text: &str) -> bool {
        let Some(target) = self.heading_at_cursor() else {
            return false;
        };
        let level = level.clamp(1, 6);
        let source = self.source();
        let raw = &source[target.full_range.clone()];
        let line_ending = if raw.ends_with("\r\n") {
            "\r\n"
        } else if raw.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let replacement = format!(
            "{} {}{}",
            "#".repeat(usize::from(level)),
            text.trim(),
            line_ending
        );
        let cursor = target.full_range.start + usize::from(level) + 1;
        self.replace_byte_range(target.full_range, &replacement);
        self.set_cursor_byte(cursor);
        true
    }

    pub fn set_heading_level_at_cursor(&mut self, level: u8) -> bool {
        let Some(target) = self.heading_at_cursor() else {
            return false;
        };
        self.set_heading_at_cursor(level, &target.text)
    }

    pub fn code_block_fields_at_cursor(&self) -> Option<CodeBlockFields> {
        let component = self.component_at_cursor()?;
        if !matches!(
            component.kind,
            ComponentKind::CodeBlock { .. } | ComponentKind::DiagramBlock { .. }
        ) {
            return None;
        }
        let source = self.source();
        let raw = &source[component.source.start..component.source.end];
        let first_line_end = raw.find('\n')?;
        let first_line = &raw[..first_line_end];
        let trimmed_first = first_line.trim_start();
        if !trimmed_first.starts_with("```") && !trimmed_first.starts_with("~~~") {
            return Some(CodeBlockFields {
                language: SourceRange {
                    start: component.source.start,
                    end: component.source.start,
                },
                body: component.source.clone(),
            });
        }
        let indent_len = first_line.len().saturating_sub(trimmed_first.len());
        let fence_len = trimmed_first
            .chars()
            .take_while(|ch| *ch == '`' || *ch == '~')
            .map(char::len_utf8)
            .sum::<usize>();
        if fence_len < 3 {
            return None;
        }
        let language_start = component.source.start + indent_len + fence_len;
        let language_end = component.source.start + first_line_end;
        let body_start = component.source.start + first_line_end + 1;
        let fence = &trimmed_first[..fence_len];
        let mut body_end = component.source.end;
        let body_raw = &raw[first_line_end + 1..];
        let mut offset = 0;
        for line in body_raw.split_inclusive('\n') {
            if line.trim_start().starts_with(fence) {
                body_end = body_start + offset;
                break;
            }
            offset += line.len();
        }
        Some(CodeBlockFields {
            language: SourceRange {
                start: language_start,
                end: language_end,
            },
            body: SourceRange {
                start: body_start,
                end: body_end,
            },
        })
    }

    pub fn focus_code_language_at_cursor(&mut self) -> bool {
        let Some(fields) = self.code_block_fields_at_cursor() else {
            return false;
        };
        self.set_cursor_byte(fields.language.start);
        true
    }

    pub fn focus_code_body_at_cursor(&mut self, end: bool) -> bool {
        let Some(fields) = self.code_block_fields_at_cursor() else {
            return false;
        };
        self.set_cursor_byte(if end {
            fields.body.end
        } else {
            fields.body.start
        });
        true
    }

    pub fn code_body_at_cursor(&self) -> Option<String> {
        let component = self.component_at_cursor()?;
        if !matches!(
            component.kind,
            ComponentKind::CodeBlock { .. } | ComponentKind::DiagramBlock { .. }
        ) {
            return None;
        }
        let source = self.source();
        let raw = &source[component.source.start..component.source.end];
        let mut lines = raw.lines();
        let first = lines.next()?;
        if !first.trim_start().starts_with("```") && !first.trim_start().starts_with("~~~") {
            return Some(raw.to_string());
        }
        Some(
            lines
                .take_while(|line| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with("```") && !trimmed.starts_with("~~~")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub fn insert_link_at_cursor(&mut self, url: &str) -> bool {
        let source = self.source();
        let byte = self.cursor.byte.min(source.len());
        let start = previous_word_boundary(&source, byte);
        let end = next_word_boundary(&source, byte);
        let label = source[start..end].trim();
        if label.is_empty() || url.trim().is_empty() {
            return false;
        }
        let replacement = format!("[{label}]({})", escape_link_destination(url.trim()));
        self.replace_byte_range(start..end, &replacement);
        true
    }

    pub fn link_at_cursor(&self) -> Option<LinkTarget> {
        find_link_target_at(&self.source(), self.cursor.byte)
    }

    pub fn set_link_at_cursor(
        &mut self,
        label: &str,
        destination: &str,
        title: Option<&str>,
        reference_label: Option<&str>,
    ) -> bool {
        let label = label.trim();
        let destination = destination.trim();
        let title = title.map(str::trim).filter(|value| !value.is_empty());
        let reference_label = reference_label
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if label.is_empty() || (destination.is_empty() && reference_label.is_none()) {
            return false;
        }
        if let Some(target) = self.link_at_cursor() {
            let old_reference = target.reference_label.clone();
            let replacement = format_link(label, destination, title, reference_label, false);
            self.replace_byte_range(target.full_range, &replacement);
            if let Some(reference_label) = reference_label {
                self.set_or_create_link_reference_definition(
                    reference_label,
                    old_reference.as_deref(),
                    destination,
                    title,
                );
            }
            true
        } else {
            let source = self.source();
            let byte = self.cursor.byte.min(source.len());
            let start = previous_word_boundary(&source, byte);
            let end = next_word_boundary(&source, byte);
            if source[start..end].trim().is_empty() {
                return false;
            }
            let replacement = format_link(label, destination, title, reference_label, false);
            self.replace_byte_range(start..end, &replacement);
            if let Some(reference_label) = reference_label {
                self.set_or_create_link_reference_definition(
                    reference_label,
                    None,
                    destination,
                    title,
                );
            }
            true
        }
    }

    pub fn toggle_strong_at_cursor(&mut self) -> bool {
        self.toggle_inline_wrapper("**", "**")
    }

    pub fn toggle_emphasis_at_cursor(&mut self) -> bool {
        self.toggle_inline_wrapper("_", "_")
    }

    pub fn toggle_inline_code_at_cursor(&mut self) -> bool {
        self.toggle_inline_wrapper("`", "`")
    }

    pub fn toggle_strikethrough_at_cursor(&mut self) -> bool {
        self.toggle_inline_wrapper("~~", "~~")
    }

    pub fn footnote_label_at_cursor(&self) -> Option<String> {
        self.footnote_target_at_cursor().map(|target| target.label)
    }

    pub fn jump_to_footnote_definition_at_cursor(&mut self) -> bool {
        let Some(target) = self.footnote_target_at_cursor() else {
            return false;
        };
        let Some(definition) = find_footnote_definition(&self.source(), &target.label) else {
            return false;
        };
        self.set_cursor_byte(definition.content_start);
        true
    }

    pub fn create_footnote_definition_at_cursor(&mut self) -> bool {
        let Some(target) = self.footnote_target_at_cursor() else {
            return false;
        };
        if find_footnote_definition(&self.source(), &target.label).is_some() {
            return self.jump_to_footnote_definition_at_cursor();
        }
        let source = self.source();
        let separator = if source.ends_with("\n\n") {
            ""
        } else if source.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let insertion = format!("{separator}[^{}]: ", target.label);
        let content_start = self.len_bytes() + insertion.len();
        self.set_cursor_byte(self.len_bytes());
        self.insert_text(&insertion);
        self.set_cursor_byte(content_start);
        true
    }

    pub fn rename_footnote_at_cursor(&mut self, new_label: &str) -> bool {
        let label = new_label.trim();
        if label.is_empty() || label.contains(']') || label.contains('\n') {
            return false;
        }
        let Some(target) = self.footnote_target_at_cursor() else {
            return false;
        };
        self.replace_byte_range(target.label_range, label);
        true
    }

    pub fn image_at_cursor(&self) -> Option<ImageTarget> {
        let component = self.component_at_cursor()?;
        if !matches!(component.kind, ComponentKind::Image) {
            return None;
        }
        parse_image_syntax(&self.source()[component.source.start..component.source.end]).map(
            |mut image| {
                image.full_range.start += component.source.start;
                image.full_range.end += component.source.start;
                image
            },
        )
    }

    pub fn set_image_at_cursor(&mut self, alt: &str, source: &str, title: Option<&str>) -> bool {
        let Some(image) = self.image_at_cursor() else {
            return false;
        };
        let source = source.trim();
        if source.is_empty() || source.contains('\n') || source.contains(')') {
            return false;
        }
        let alt = alt.replace(']', "\\]");
        let replacement =
            if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
                format!("![{alt}]({source} \"{}\")", title.replace('"', "\\\""))
            } else {
                format!("![{alt}]({source})")
            };
        self.replace_byte_range(image.full_range, &replacement);
        true
    }

    pub fn table_cell_at_cursor(&self) -> Option<(usize, usize)> {
        let (component, table) = self.table_at_cursor()?;
        let local = self.cursor.byte.saturating_sub(component.source.start);
        for (row_index, row) in table.rows.iter().enumerate() {
            for (column_index, cell) in row.cells.iter().enumerate() {
                if local >= cell.source_start && local <= cell.source_end {
                    return Some((row_index, column_index));
                }
            }
        }
        Some((0, 0))
    }

    pub fn focus_table_cell(&mut self, row: usize, col: usize) -> bool {
        let Some((component, table)) = self.table_at_cursor() else {
            return false;
        };
        let Some(cell) = table.rows.get(row).and_then(|row| row.cells.get(col)) else {
            return false;
        };
        self.set_cursor_byte(component.source.start + cell.content_start);
        true
    }

    pub fn move_table_cell(&mut self, forward: bool) -> bool {
        let Some((_component, table)) = self.table_at_cursor() else {
            return false;
        };
        let Some((row, col)) = self.table_cell_at_cursor() else {
            return false;
        };
        let columns = table.column_count().max(1);
        let index = row.saturating_mul(columns).saturating_add(col);
        let cell_count = table.rows.len().saturating_mul(columns);
        if cell_count == 0 {
            return false;
        }
        let target = if forward {
            (index + 1).min(cell_count.saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        self.focus_table_cell(target / columns, target % columns)
    }

    pub fn enter_table_cell(&mut self) -> bool {
        let Some((_component, table)) = self.table_at_cursor() else {
            return false;
        };
        let Some((row, col)) = self.table_cell_at_cursor() else {
            return false;
        };
        if row + 1 >= table.rows.len() && !self.insert_table_row_at_cursor(false) {
            return false;
        }
        self.focus_table_cell(row + 1, col)
    }

    pub fn remove_empty_table_row_at_cursor(&mut self) -> bool {
        let Some((_component, table)) = self.table_at_cursor() else {
            return false;
        };
        let Some((row, _col)) = self.table_cell_at_cursor() else {
            return false;
        };
        let Some(current_row) = table.rows.get(row) else {
            return false;
        };
        if current_row
            .cells
            .iter()
            .any(|cell| !cell.text.trim().is_empty())
        {
            return false;
        }
        self.remove_table_row_at_cursor()
    }

    pub fn insert_table_row_at_cursor(&mut self, before: bool) -> bool {
        self.edit_table_at_cursor(|table, row, _col| {
            let insert_at = if before { row } else { row.saturating_add(1) };
            let columns = table.column_count().max(1);
            table.rows.insert(
                insert_at.min(table.rows.len()),
                PipeTableRow {
                    cells: (0..columns).map(|_| PipeTableCell::new("")).collect(),
                },
            );
        })
    }

    pub fn remove_table_row_at_cursor(&mut self) -> bool {
        self.edit_table_at_cursor(|table, row, _col| {
            if table.rows.len() > 1 {
                table
                    .rows
                    .remove(row.min(table.rows.len().saturating_sub(1)));
            }
        })
    }

    pub fn remove_table_row(&mut self, selected_row: usize) -> bool {
        self.edit_table_at_cursor(|table, _row, _col| {
            if table.rows.len() > 1 {
                table
                    .rows
                    .remove(selected_row.min(table.rows.len().saturating_sub(1)));
            }
        })
    }

    pub fn insert_table_column_at_cursor(&mut self, before: bool) -> bool {
        self.edit_table_at_cursor(|table, _row, col| {
            let insert_at = if before { col } else { col.saturating_add(1) };
            for row in &mut table.rows {
                row.cells
                    .insert(insert_at.min(row.cells.len()), PipeTableCell::new(""));
            }
            table.alignments.insert(
                insert_at.min(table.alignments.len()),
                TableAlignment::Default,
            );
        })
    }

    pub fn remove_table_column_at_cursor(&mut self) -> bool {
        self.edit_table_at_cursor(|table, _row, col| {
            if table.column_count() <= 1 {
                return;
            }
            for row in &mut table.rows {
                if col < row.cells.len() {
                    row.cells.remove(col);
                }
            }
            if col < table.alignments.len() {
                table.alignments.remove(col);
            }
        })
    }

    pub fn remove_table_column(&mut self, selected_col: usize) -> bool {
        self.edit_table_at_cursor(|table, _row, _col| {
            if table.column_count() <= 1 {
                return;
            }
            let selected_col = selected_col.min(table.column_count().saturating_sub(1));
            for row in &mut table.rows {
                if selected_col < row.cells.len() {
                    row.cells.remove(selected_col);
                }
            }
            if selected_col < table.alignments.len() {
                table.alignments.remove(selected_col);
            }
        })
    }

    pub fn change_table_column_alignment_at_cursor(&mut self, forward: bool) -> bool {
        self.edit_table_at_cursor(|table, _row, col| {
            table.ensure_alignments();
            if let Some(alignment) = table.alignments.get_mut(col) {
                *alignment = alignment.cycle(forward);
            }
        })
    }

    pub fn normalize_table_at_cursor(&mut self) -> bool {
        self.edit_table_at_cursor(|_table, _row, _col| {})
    }

    pub fn undo(&mut self) -> bool {
        let current = self.source();
        let Some(previous) = self.history.undo(current) else {
            return false;
        };
        let old_source = self.source();
        self.text = Rope::from_str(&previous);
        self.cursor.byte = self.cursor.byte.min(self.len_bytes());
        self.selection = None;
        self.rebuild_after_change(&old_source);
        true
    }

    pub fn redo(&mut self) -> bool {
        let current = self.source();
        let Some(next) = self.history.redo(current) else {
            return false;
        };
        let old_source = self.source();
        self.text = Rope::from_str(&next);
        self.cursor.byte = self.cursor.byte.min(self.len_bytes());
        self.selection = None;
        self.rebuild_after_change(&old_source);
        true
    }

    pub fn line_column_for_byte(&self, byte: usize) -> (usize, usize) {
        let byte = byte.min(self.len_bytes());
        let line = self.text.byte_to_line(byte);
        let line_start = self.text.line_to_byte(line);
        let line_text = self.line_slice(line);
        let local = byte.saturating_sub(line_start).min(line_text.len());
        let column = UnicodeWidthStr::width(&line_text[..local]);
        (line + 1, column + 1)
    }

    pub fn current_line_range(&self) -> Range<usize> {
        let line = self
            .text
            .byte_to_line(self.cursor.byte.min(self.len_bytes()));
        let start = self.text.line_to_byte(line);
        let end = if line + 1 < self.text.len_lines() {
            self.text.line_to_byte(line + 1)
        } else {
            self.len_bytes()
        };
        start..end
    }

    fn edit(&mut self, change: impl FnOnce(&mut Self)) {
        let old_source = self.source();
        self.history.remember(old_source.clone());
        change(self);
        self.cursor.byte = self.cursor.byte.min(self.len_bytes());
        self.selection = None;
        self.rebuild_after_change(&old_source);
    }

    fn replace_byte_range(&mut self, range: Range<usize>, replacement: &str) {
        self.edit(|document| {
            let start = document.byte_to_char(range.start);
            let end = document.byte_to_char(range.end);
            document.text.remove(start..end);
            document.text.insert(start, replacement);
            document.cursor.byte = range.start + replacement.len();
            document.selection = None;
        });
    }

    fn set_cursor_byte_preserving_selection(&mut self, byte: usize) {
        self.cursor.byte = clamp_to_grapheme_boundary(&self.source(), byte.min(self.len_bytes()));
        self.cursor.desired_col = None;
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        self.edit(|document| {
            let start = document.byte_to_char(range.start);
            let end = document.byte_to_char(range.end);
            document.text.remove(start..end);
            document.cursor.byte = range.start;
            document.selection = None;
        });
        true
    }

    fn start_selection(&mut self) {
        if self.selection.is_none() {
            self.selection = Some(SelectionRange {
                anchor: self.cursor.byte,
                focus: self.cursor.byte,
            });
        }
    }

    fn update_selection_focus(&mut self) {
        if let Some(selection) = &mut self.selection {
            selection.focus = self.cursor.byte;
            if selection.anchor == selection.focus {
                self.selection = None;
            }
        }
    }

    fn component_at_cursor(&self) -> Option<&Component> {
        self.components.component_at_byte(self.cursor.byte)
    }

    fn table_at_cursor(&self) -> Option<(Component, PipeTable)> {
        let component = self.component_at_cursor()?.clone();
        if !matches!(component.kind, ComponentKind::Table) {
            return None;
        }
        let source = self.source();
        let table = parse_pipe_table(&source[component.source.start..component.source.end]);
        Some((component, table))
    }

    fn edit_table_at_cursor(&mut self, edit: impl FnOnce(&mut PipeTable, usize, usize)) -> bool {
        let Some(component) = self.component_at_cursor().cloned() else {
            return false;
        };
        if !matches!(component.kind, ComponentKind::Table) {
            return false;
        }
        let source = self.source();
        let mut table = parse_pipe_table(&source[component.source.start..component.source.end]);
        if table.rows.is_empty() {
            return false;
        }
        let (row, col) = self.table_cell_at_cursor().unwrap_or((0, 0));
        edit(&mut table, row, col);
        let replacement = table.to_markdown();
        let table_start = component.source.start;
        self.replace_byte_range(component.source.start..component.source.end, &replacement);
        self.set_cursor_byte(table_start);
        let _focused = self.focus_table_cell(
            row.min(table.rows.len().saturating_sub(1)),
            col.min(table.column_count().saturating_sub(1)),
        );
        true
    }

    fn set_or_create_link_reference_definition(
        &mut self,
        label: &str,
        old_label: Option<&str>,
        destination: &str,
        title: Option<&str>,
    ) {
        let source = self.source();
        let definition = find_link_reference_definition(&source, label).or_else(|| {
            old_label
                .filter(|old_label| *old_label != label)
                .and_then(|old_label| find_link_reference_definition(&source, old_label))
        });
        let replacement = format_link_reference_definition(label, destination, title);
        if let Some(definition) = definition {
            self.replace_byte_range(definition.full_range, &replacement);
            return;
        }
        let separator = if source.ends_with("\n\n") {
            ""
        } else if source.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        self.set_cursor_byte(self.len_bytes());
        self.insert_text(&format!("{separator}{replacement}\n"));
    }

    fn toggle_inline_wrapper(&mut self, open: &str, close: &str) -> bool {
        let source = self.source();
        let byte = self.cursor.byte.min(source.len());
        let (start, end) = self
            .selection_range()
            .map(|range| (range.start, range.end))
            .unwrap_or_else(|| {
                (
                    previous_word_boundary(&source, byte),
                    next_word_boundary(&source, byte),
                )
            });
        if start == end {
            return false;
        }
        let wrapped_in_range = source[start..end].starts_with(open)
            && source[start..end].ends_with(close)
            && end.saturating_sub(start) >= open.len() + close.len();
        if wrapped_in_range {
            let inner_start = start + open.len();
            let inner_end = end - close.len();
            let replacement = source[inner_start..inner_end].to_string();
            self.replace_byte_range(start..end, &replacement);
            return true;
        }
        let already_wrapped = start >= open.len()
            && end + close.len() <= source.len()
            && &source[start - open.len()..start] == open
            && &source[end..end + close.len()] == close;
        if already_wrapped {
            let open_start = start - open.len();
            let close_end = end + close.len();
            let replacement = source[start..end].to_string();
            self.replace_byte_range(open_start..close_end, &replacement);
        } else {
            let replacement = format!("{open}{}{close}", &source[start..end]);
            self.replace_byte_range(start..end, &replacement);
        }
        true
    }

    fn footnote_target_at_cursor(&self) -> Option<FootnoteTarget> {
        find_footnote_target_at(&self.source(), self.cursor.byte)
    }

    fn rebuild_after_change(&mut self, old_source: &str) {
        self.dirty.is_dirty = true;
        self.dirty.generation = self.dirty.generation.saturating_add(1);
        self.layout_cache = LayoutCache::default();
        self.rebuild_semantics_incremental(old_source);
    }

    fn rebuild_semantics(&mut self) {
        let source = self.source();
        let (tree_sitter, tree) = parse_tree_sitter(&source, None, None);
        self.tree_sitter_cache.tree = tree;
        self.parse = MarkdownParse {
            ok: true,
            diagnostics: diagnose(&source),
            tree_sitter,
        };
        self.parse.ok = self.parse.tree_sitter.parsed && !self.parse.tree_sitter.has_error;
        if self.parse.tree_sitter.has_error {
            self.parse.diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: 0,
                    end: source.len(),
                },
                message: "tree-sitter-md reported a parse error".to_string(),
                fix: Some("Inspect Markdown around the highlighted source range".to_string()),
            });
        }
        self.components = parse_components(&source);
    }

    fn rebuild_semantics_incremental(&mut self, old_source: &str) {
        let source = self.source();
        let edit = source_edit(old_source, &source);
        let previous = self.tree_sitter_cache.tree.take();
        let (tree_sitter, tree) = parse_tree_sitter(&source, previous, edit.as_ref());
        self.tree_sitter_cache.tree = tree;
        self.parse = MarkdownParse {
            ok: true,
            diagnostics: diagnose(&source),
            tree_sitter,
        };
        self.parse.ok = self.parse.tree_sitter.parsed && !self.parse.tree_sitter.has_error;
        if self.parse.tree_sitter.has_error {
            self.parse.diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: 0,
                    end: source.len(),
                },
                message: "tree-sitter-md reported a parse error".to_string(),
                fix: Some("Inspect Markdown around the highlighted source range".to_string()),
            });
        }
        self.components = parse_components(&source);
    }

    fn byte_to_char(&self, byte: usize) -> usize {
        self.text.byte_to_char(byte.min(self.len_bytes()))
    }

    fn line_slice(&self, line: usize) -> String {
        self.text
            .line(line.min(self.text.len_lines().saturating_sub(1)))
            .to_string()
    }

    fn move_vertical(&mut self, delta: isize) {
        let source = self.source();
        let (line, visual_col) = self.line_column_for_byte(self.cursor.byte);
        let desired = self
            .cursor
            .desired_col
            .unwrap_or(visual_col.saturating_sub(1));
        let zero_line = line.saturating_sub(1);
        let target_line = if delta < 0 {
            zero_line.saturating_sub(delta.unsigned_abs())
        } else {
            (zero_line + delta as usize).min(self.text.len_lines().saturating_sub(1))
        };
        let target_start = self.text.line_to_byte(target_line);
        let target_text = self.line_slice(target_line);
        let mut width = 0;
        let mut target_local = 0;
        for (index, grapheme) in target_text.grapheme_indices(true) {
            if grapheme == "\n" || grapheme == "\r\n" {
                break;
            }
            let grapheme_width = grapheme
                .chars()
                .map(|ch| ch.width().unwrap_or(0))
                .sum::<usize>();
            if width + grapheme_width > desired {
                break;
            }
            width += grapheme_width;
            target_local = index + grapheme.len();
        }
        self.cursor.byte = clamp_to_grapheme_boundary(&source, target_start + target_local);
        self.cursor.desired_col = Some(desired);
    }
}

pub fn parse_components(source: &str) -> ComponentArena {
    let mut components = Vec::new();
    let mut next_id = 1_u64;
    let lines = split_lines_with_offsets(source);
    let mut index = 0_usize;

    while index < lines.len() {
        let (line_start, line) = lines[index];
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let trim_start = trimmed.trim_start();

        if trimmed.is_empty() {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::Gap,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if let Some((fence, language)) = fence_start(trim_start) {
            let start = line_start;
            let mut end = line_start + line.len();
            index += 1;
            while index < lines.len() {
                let (next_start, next_line) = lines[index];
                end = next_start + next_line.len();
                if next_line.trim_start().starts_with(fence) {
                    index += 1;
                    break;
                }
                index += 1;
            }
            let kind = if matches_diagram_language(&language) {
                ComponentKind::DiagramBlock {
                    language: diagram_language(&language),
                }
            } else {
                ComponentKind::CodeBlock {
                    fenced: true,
                    language,
                }
            };
            push_component(&mut components, &mut next_id, kind, start..end, source);
            continue;
        }

        if let Some(level) = heading_level(trim_start) {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::Heading { level },
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if is_thematic_break(trim_start) {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::ThematicBreak,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if trim_start.starts_with('|') && trim_start.contains('|') {
            let start = line_start;
            let mut end = line_start + line.len();
            index += 1;
            while index < lines.len() {
                let (next_start, next_line) = lines[index];
                if !next_line.trim_start().starts_with('|') {
                    break;
                }
                end = next_start + next_line.len();
                index += 1;
            }
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::Table,
                start..end,
                source,
            );
            continue;
        }

        if let Some((ordered, checked)) = list_marker(trim_start) {
            let start = line_start;
            let mut end = line_start + line.len();
            let mut children = Vec::new();
            while index < lines.len() {
                let (next_start, next_line) = lines[index];
                let Some((_, item_checked)) = list_marker(next_line.trim_start()) else {
                    break;
                };
                let item_id = ComponentId(next_id);
                next_id += 1;
                children.push(item_id);
                components.push(Component {
                    id: item_id,
                    kind: ComponentKind::ListItem {
                        checked: item_checked.or(checked),
                    },
                    source: SourceRange {
                        start: next_start,
                        end: next_start + next_line.len(),
                    },
                    fields: vec![FieldId(item_id.0)],
                    children: Vec::new(),
                    render_hash: stable_hash(&source[next_start..next_start + next_line.len()]),
                });
                end = next_start + next_line.len();
                index += 1;
            }
            let id = ComponentId(next_id);
            next_id += 1;
            components.push(Component {
                id,
                kind: ComponentKind::List {
                    ordered,
                    tight: true,
                },
                source: SourceRange { start, end },
                fields: Vec::new(),
                children,
                render_hash: stable_hash(&source[start..end]),
            });
            continue;
        }

        if trim_start.starts_with('>') {
            let start = line_start;
            let mut end = line_start + line.len();
            let alert = alert_level(trim_start);
            index += 1;
            while index < lines.len() {
                let (next_start, next_line) = lines[index];
                if !next_line.trim_start().starts_with('>') {
                    break;
                }
                end = next_start + next_line.len();
                index += 1;
            }
            push_component(
                &mut components,
                &mut next_id,
                alert.map_or(ComponentKind::BlockQuote, |level| ComponentKind::Alert {
                    level,
                }),
                start..end,
                source,
            );
            continue;
        }

        if trim_start.starts_with("[^") && trim_start.contains("]:") {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::FootnoteDef,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if parse_image_syntax(trim_start).is_some() {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::Image,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if find_footnote_ref_in_line(line, line_start).is_some() {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::FootnoteRef,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if trim_start.starts_with('<') {
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::HtmlBlock,
                line_start..line_start + line.len(),
                source,
            );
            index += 1;
            continue;
        }

        if trim_start == "$$" {
            let start = line_start;
            let mut end = line_start + line.len();
            index += 1;
            while index < lines.len() {
                let (next_start, next_line) = lines[index];
                end = next_start + next_line.len();
                index += 1;
                if next_line.trim() == "$$" {
                    break;
                }
            }
            push_component(
                &mut components,
                &mut next_id,
                ComponentKind::MathBlock,
                start..end,
                source,
            );
            continue;
        }

        let start = line_start;
        let mut end = line_start + line.len();
        index += 1;
        while index < lines.len() {
            let (next_start, next_line) = lines[index];
            let next_trim = next_line.trim_start();
            if next_line.trim().is_empty()
                || heading_level(next_trim).is_some()
                || fence_start(next_trim).is_some()
                || list_marker(next_trim).is_some()
                || next_trim.starts_with('>')
                || next_trim.starts_with('|')
            {
                break;
            }
            end = next_start + next_line.len();
            index += 1;
        }
        push_component(
            &mut components,
            &mut next_id,
            ComponentKind::Paragraph,
            start..end,
            source,
        );
    }

    if components.is_empty() && source.is_empty() {
        push_component(
            &mut components,
            &mut next_id,
            ComponentKind::Gap,
            0..0,
            source,
        );
    }

    ComponentArena { components }
}

fn push_component(
    components: &mut Vec<Component>,
    next_id: &mut u64,
    kind: ComponentKind,
    range: Range<usize>,
    source: &str,
) {
    let id = ComponentId(*next_id);
    *next_id += 1;
    components.push(Component {
        id,
        kind,
        source: SourceRange {
            start: range.start,
            end: range.end,
        },
        fields: vec![FieldId(id.0)],
        children: Vec::new(),
        render_hash: stable_hash(&source[range]),
    });
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PipeTable {
    rows: Vec<PipeTableRow>,
    alignments: Vec<TableAlignment>,
}

impl PipeTable {
    fn column_count(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or_default()
    }

    fn to_markdown(&self) -> String {
        let columns = self.column_count().max(1);
        let mut widths = vec![3_usize; columns];
        for row in &self.rows {
            for (index, cell) in row.cells.iter().enumerate() {
                widths[index] = widths[index].max(cell.text.chars().count());
            }
        }

        let mut lines = Vec::new();
        for (row_index, row) in self.rows.iter().enumerate() {
            lines.push(format_table_row(row, &widths));
            if row_index == 0 && self.rows.len() > 1 {
                lines.push(format_alignment_row(&widths, &self.alignments));
            }
        }

        if lines.len() == 1 {
            lines.push(format_alignment_row(&widths, &self.alignments));
        }

        let mut markdown = lines.join("\n");
        markdown.push('\n');
        markdown
    }

    fn ensure_alignments(&mut self) {
        let columns = self.column_count().max(1);
        self.alignments.resize(columns, TableAlignment::Default);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TableAlignment {
    #[default]
    Default,
    Left,
    Center,
    Right,
}

impl TableAlignment {
    fn cycle(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::Default, true) => Self::Left,
            (Self::Left, true) => Self::Center,
            (Self::Center, true) => Self::Right,
            (Self::Right, true) => Self::Default,
            (Self::Default, false) => Self::Right,
            (Self::Right, false) => Self::Center,
            (Self::Center, false) => Self::Left,
            (Self::Left, false) => Self::Default,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PipeTableRow {
    cells: Vec<PipeTableCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PipeTableCell {
    text: String,
    source_start: usize,
    content_start: usize,
    source_end: usize,
}

impl PipeTableCell {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            source_start: 0,
            content_start: 0,
            source_end: 0,
        }
    }
}

fn parse_pipe_table(source: &str) -> PipeTable {
    let mut rows = Vec::new();
    let mut alignments = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with('|') && is_alignment_row(line) {
            alignments = parse_alignment_row(line);
        } else if line.trim_start().starts_with('|') {
            rows.push(parse_pipe_table_row(line, offset));
        }
        offset += line.len();
    }
    let mut table = PipeTable { rows, alignments };
    table.ensure_alignments();
    table
}

fn parse_pipe_table_row(line: &str, line_offset: usize) -> PipeTableRow {
    let mut cells = Vec::new();
    let trimmed = line.trim_end();
    let bytes = trimmed.as_bytes();
    let mut pipe_positions = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'|').then_some(index))
        .collect::<Vec<_>>();
    if pipe_positions.first() != Some(&0) {
        pipe_positions.insert(0, 0);
    }
    if pipe_positions.last().copied() != Some(trimmed.len().saturating_sub(1)) {
        pipe_positions.push(trimmed.len());
    }
    for window in pipe_positions.windows(2) {
        let left = window[0];
        let right = window[1];
        let raw_start = (left + 1).min(trimmed.len());
        let raw_end = right.min(trimmed.len());
        let raw = &trimmed[raw_start..raw_end];
        let leading = raw.len().saturating_sub(raw.trim_start().len());
        let content_start = if raw.trim().is_empty() {
            raw_start
        } else {
            raw_start + leading
        };
        cells.push(PipeTableCell {
            text: raw.trim().replace("\\|", "|"),
            source_start: line_offset + raw_start,
            content_start: line_offset + content_start,
            source_end: line_offset + raw_end,
        });
    }
    PipeTableRow { cells }
}

fn is_alignment_row(line: &str) -> bool {
    line.trim().trim_matches('|').split('|').all(|cell| {
        let trimmed = cell.trim();
        trimmed.contains('-')
            && trimmed
                .chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
    })
}

fn format_table_row(row: &PipeTableRow, widths: &[usize]) -> String {
    let mut out = String::from("|");
    for (index, width) in widths.iter().enumerate() {
        let value = row
            .cells
            .get(index)
            .map(|cell| escape_table_cell(&cell.text))
            .unwrap_or_default();
        out.push(' ');
        out.push_str(&value);
        out.push_str(&" ".repeat(width.saturating_sub(value.chars().count()) + 1));
        out.push('|');
    }
    out
}

fn parse_alignment_row(line: &str) -> Vec<TableAlignment> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| {
            let trimmed = cell.trim();
            let left = trimmed.starts_with(':');
            let right = trimmed.ends_with(':');
            match (left, right) {
                (true, true) => TableAlignment::Center,
                (true, false) => TableAlignment::Left,
                (false, true) => TableAlignment::Right,
                (false, false) => TableAlignment::Default,
            }
        })
        .collect()
}

fn format_alignment_row(widths: &[usize], alignments: &[TableAlignment]) -> String {
    let mut out = String::from("|");
    for (index, width) in widths.iter().enumerate() {
        let marker_width = (*width).max(3);
        let marker = match alignments.get(index).copied().unwrap_or_default() {
            TableAlignment::Default => "-".repeat(marker_width),
            TableAlignment::Left => format!(":{}", "-".repeat(marker_width.saturating_sub(1))),
            TableAlignment::Center => {
                if marker_width <= 2 {
                    ":--:".to_string()
                } else {
                    format!(":{}:", "-".repeat(marker_width.saturating_sub(2)))
                }
            }
            TableAlignment::Right => format!("{}:", "-".repeat(marker_width.saturating_sub(1))),
        };
        out.push(' ');
        out.push_str(&marker);
        out.push(' ');
        out.push('|');
    }
    out
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

fn parse_tree_sitter(
    source: &str,
    previous: Option<MarkdownTree>,
    edit: Option<&InputEdit>,
) -> (TreeSitterParse, Option<MarkdownTree>) {
    let mut parser = MarkdownParser::default();
    let mut previous = previous;
    let reused_previous_tree = if let (Some(tree), Some(edit)) = (&mut previous, edit) {
        tree.edit(edit);
        true
    } else {
        false
    };
    let Some(tree) = parser.parse(source.as_bytes(), previous.as_ref()) else {
        return (
            TreeSitterParse {
                parsed: false,
                root_kind: String::new(),
                has_error: true,
                reused_previous_tree,
                changed_range_count: 0,
                block_node_count: 0,
                inline_tree_count: 0,
                named_nodes: Vec::new(),
            },
            previous,
        );
    };
    let root = tree.block_tree().root_node();
    let mut named_nodes = Vec::new();
    collect_tree_sitter_nodes(root, &mut named_nodes);
    let block_node_count = count_tree_sitter_nodes(root);
    let changed_range_count = previous
        .as_ref()
        .map(|old_tree| {
            tree.block_tree()
                .changed_ranges(old_tree.block_tree())
                .count()
        })
        .unwrap_or_default();
    let summary = TreeSitterParse {
        parsed: true,
        root_kind: root.kind().to_string(),
        has_error: root.has_error(),
        reused_previous_tree,
        changed_range_count,
        block_node_count,
        inline_tree_count: tree.inline_trees().len(),
        named_nodes,
    };
    (summary, Some(tree))
}

fn source_edit(old_source: &str, new_source: &str) -> Option<InputEdit> {
    if old_source == new_source {
        return None;
    }
    let start = common_prefix_boundary(old_source, new_source);
    let (old_end, new_end) = common_suffix_boundaries(old_source, new_source, start);
    Some(InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_for_byte(old_source, start),
        old_end_position: point_for_byte(old_source, old_end),
        new_end_position: point_for_byte(new_source, new_end),
    })
}

fn common_prefix_boundary(old_source: &str, new_source: &str) -> usize {
    let mut start = 0;
    for (old, new) in old_source.bytes().zip(new_source.bytes()) {
        if old != new {
            break;
        }
        start += 1;
    }
    while start > 0 && (!old_source.is_char_boundary(start) || !new_source.is_char_boundary(start))
    {
        start -= 1;
    }
    start
}

fn common_suffix_boundaries(old_source: &str, new_source: &str, start: usize) -> (usize, usize) {
    let mut old_end = old_source.len();
    let mut new_end = new_source.len();
    while old_end > start
        && new_end > start
        && old_source.as_bytes()[old_end - 1] == new_source.as_bytes()[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }
    while old_end < old_source.len() && !old_source.is_char_boundary(old_end) {
        old_end += 1;
    }
    while new_end < new_source.len() && !new_source.is_char_boundary(new_end) {
        new_end += 1;
    }
    (old_end, new_end)
}

fn point_for_byte(source: &str, byte: usize) -> Point {
    let byte = byte.min(source.len());
    let prefix = &source[..byte];
    let row = prefix.bytes().filter(|value| *value == b'\n').count();
    let column = prefix
        .rfind('\n')
        .map_or(prefix.len(), |line_break| prefix.len() - line_break - 1);
    Point::new(row, column)
}

fn collect_tree_sitter_nodes(node: Node<'_>, out: &mut Vec<TreeSitterNode>) {
    if node.is_named() {
        let point = node.start_position();
        out.push(TreeSitterNode {
            kind: node.kind().to_string(),
            source: SourceRange {
                start: node.start_byte(),
                end: node.end_byte(),
            },
            row: point.row,
            column: point.column,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_tree_sitter_nodes(child, out);
    }
}

fn count_tree_sitter_nodes(node: Node<'_>) -> usize {
    let mut cursor = node.walk();
    1 + node
        .children(&mut cursor)
        .map(count_tree_sitter_nodes)
        .sum::<usize>()
}

fn split_lines_with_offsets(source: &str) -> Vec<(usize, &str)> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        lines.push((offset, line));
        offset += line.len();
    }
    if !source.ends_with('\n') {
        return lines;
    }
    lines
}

fn block_insert_prefix(before: &str) -> &'static str {
    if before.trim().is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

fn block_insert_suffix(after: &str) -> &'static str {
    if after.trim().is_empty() {
        "\n"
    } else if after.starts_with("\n\n") {
        ""
    } else if after.starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        Some(hashes as u8)
    } else {
        None
    }
}

fn heading_text(raw: &str) -> String {
    raw.trim_end_matches(['\n', '\r'])
        .trim_start()
        .trim_start_matches('#')
        .trim_start()
        .to_string()
}

fn fence_start(line: &str) -> Option<(&str, String)> {
    for fence in ["```", "~~~"] {
        if let Some(rest) = line.strip_prefix(fence) {
            return Some((fence, rest.trim().to_string()));
        }
    }
    None
}

fn list_marker(line: &str) -> Option<(bool, Option<bool>)> {
    let unordered = ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker));
    let (ordered, rest) = if let Some(rest) = unordered {
        (false, rest)
    } else {
        let dot = line.find(". ")?;
        if !line[..dot].chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        (true, &line[dot + 2..])
    };
    let checked = if rest.starts_with("[ ] ") {
        Some(false)
    } else if rest.starts_with("[x] ") || rest.starts_with("[X] ") {
        Some(true)
    } else {
        None
    };
    Some((ordered, checked))
}

fn alert_level(line: &str) -> Option<AlertLevel> {
    let marker = line.trim_start_matches('>').trim_start();
    match marker {
        value if value.starts_with("[!NOTE]") => Some(AlertLevel::Note),
        value if value.starts_with("[!TIP]") => Some(AlertLevel::Tip),
        value if value.starts_with("[!IMPORTANT]") => Some(AlertLevel::Important),
        value if value.starts_with("[!WARNING]") => Some(AlertLevel::Warning),
        value if value.starts_with("[!CAUTION]") => Some(AlertLevel::Caution),
        _ => None,
    }
}

fn is_thematic_break(line: &str) -> bool {
    let compact: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|ch| ch == '-')
            || compact.chars().all(|ch| ch == '*')
            || compact.chars().all(|ch| ch == '_'))
}

fn matches_diagram_language(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "mermaid" | "geojson" | "topojson" | "stl"
    )
}

fn diagram_language(language: &str) -> DiagramLanguage {
    match language.trim().to_ascii_lowercase().as_str() {
        "mermaid" => DiagramLanguage::Mermaid,
        "geojson" => DiagramLanguage::GeoJson,
        "topojson" => DiagramLanguage::TopoJson,
        "stl" => DiagramLanguage::Stl,
        other => DiagramLanguage::Other(other.to_string()),
    }
}

fn diagnose(source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (line_start, line) in split_lines_with_offsets(source) {
        if let Some(image) = parse_image_syntax(line.trim_start())
            && is_local_image_path(&image.source)
            && !std::path::Path::new(&image.source).exists()
        {
            diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: line_start,
                    end: line_start + line.len(),
                },
                message: format!("broken local image path: {}", image.source),
                fix: Some("Edit image source".to_string()),
            });
        }
    }
    let refs = find_footnote_refs(source);
    let defs = find_footnote_definitions(source);
    for reference in &refs {
        if !defs
            .iter()
            .any(|definition| definition.label == reference.label)
        {
            diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: reference.full_range.start,
                    end: reference.full_range.end,
                },
                message: format!("missing footnote definition: {}", reference.label),
                fix: Some("Create footnote definition".to_string()),
            });
        }
    }
    for definition in &defs {
        let duplicates = defs
            .iter()
            .filter(|candidate| candidate.label == definition.label)
            .count();
        if duplicates > 1 {
            diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: definition.full_range.start,
                    end: definition.full_range.end,
                },
                message: format!("duplicate footnote label: {}", definition.label),
                fix: Some("Rename footnote".to_string()),
            });
        }
    }
    for reference in find_link_reference_targets(source) {
        if find_link_reference_definition(source, &reference.reference_label).is_none() {
            diagnostics.push(Diagnostic {
                range: SourceRange {
                    start: reference.full_range.start,
                    end: reference.full_range.end,
                },
                message: format!(
                    "missing link reference definition: {}",
                    reference.reference_label
                ),
                fix: Some("Create reference definition".to_string()),
            });
        }
    }
    for (start, line) in split_lines_with_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('|') && !trimmed.trim_end().ends_with('|') {
            diagnostics.push(Diagnostic {
                range: SourceRange {
                    start,
                    end: start + line.len(),
                },
                message: "malformed table row".to_string(),
                fix: Some("Normalize table".to_string()),
            });
        }
        for tag in ["<script", "<iframe", "<style"] {
            if trimmed.to_ascii_lowercase().contains(tag) {
                diagnostics.push(Diagnostic {
                    range: SourceRange {
                        start,
                        end: start + line.len(),
                    },
                    message: "raw HTML may be unsafe".to_string(),
                    fix: Some("Escape tag".to_string()),
                });
            }
        }
    }
    diagnostics
}

fn stable_hash(text: &str) -> u64 {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn previous_word_boundary(source: &str, byte: usize) -> usize {
    let mut start = byte.min(source.len());
    for (index, grapheme) in source[..start].grapheme_indices(true).rev() {
        if grapheme.chars().all(|ch| ch.is_whitespace()) {
            break;
        }
        start = index;
    }
    start
}

fn next_word_boundary(source: &str, byte: usize) -> usize {
    let mut end = byte.min(source.len());
    for (offset, grapheme) in source[end..].grapheme_indices(true) {
        if grapheme.chars().all(|ch| ch.is_whitespace()) {
            break;
        }
        end = byte + offset + grapheme.len();
    }
    end
}

fn escape_link_destination(url: &str) -> String {
    url.replace(')', "%29")
}

fn escape_link_label(label: &str) -> String {
    label.replace(']', "\\]")
}

fn format_link(
    label: &str,
    destination: &str,
    title: Option<&str>,
    reference_label: Option<&str>,
    image: bool,
) -> String {
    let prefix = if image { "!" } else { "" };
    let label = escape_link_label(label);
    if let Some(reference_label) = reference_label {
        return format!("{prefix}[{label}][{}]", escape_link_label(reference_label));
    }
    let destination = escape_link_destination(destination.trim());
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        format!(
            "{prefix}[{label}]({destination} \"{}\")",
            title.replace('"', "\\\"")
        )
    } else {
        format!("{prefix}[{label}]({destination})")
    }
}

fn format_link_reference_definition(label: &str, destination: &str, title: Option<&str>) -> String {
    let label = escape_link_label(label.trim());
    let destination = escape_link_destination(destination.trim());
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        format!(
            "[{label}]: {destination} \"{}\"",
            title.replace('"', "\\\"")
        )
    } else {
        format!("[{label}]: {destination}")
    }
}

fn find_link_target_at(source: &str, byte: usize) -> Option<LinkTarget> {
    let line_start = source[..byte.min(source.len())]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[byte.min(source.len())..]
        .find('\n')
        .map_or(source.len(), |offset| byte.min(source.len()) + offset);
    let line = &source[line_start..line_end];
    let local_byte = byte.saturating_sub(line_start);
    let mut offset = 0;
    while let Some(open_relative) = line[offset..].find('[') {
        let open = offset + open_relative;
        if open > 0 && line.as_bytes().get(open - 1) == Some(&b'!') {
            offset = open + 1;
            continue;
        }
        let Some(close_relative) = line[open + 1..].find(']') else {
            break;
        };
        let close = open + 1 + close_relative;
        let after = close + 1;
        if let Some(inner_start) = line[after..].strip_prefix('(') {
            let inner_offset = after + 1;
            let Some(close_paren_relative) = inner_start.find(')') else {
                offset = after;
                continue;
            };
            let close_paren = inner_offset + close_paren_relative;
            let full = open..close_paren + 1;
            if local_byte >= full.start && local_byte <= full.end {
                let (destination, title) =
                    split_image_destination(&line[inner_offset..close_paren]);
                return Some(LinkTarget {
                    label: line[open + 1..close].replace("\\]", "]"),
                    destination,
                    title,
                    reference_label: None,
                    full_range: line_start + full.start..line_start + full.end,
                });
            }
            offset = full.end;
        } else if let Some(reference_start) = line[after..].strip_prefix('[') {
            let reference_offset = after + 1;
            let Some(reference_close_relative) = reference_start.find(']') else {
                offset = after;
                continue;
            };
            let reference_close = reference_offset + reference_close_relative;
            let full = open..reference_close + 1;
            if local_byte >= full.start && local_byte <= full.end {
                let label = line[open + 1..close].replace("\\]", "]");
                let reference_label = if reference_close == reference_offset {
                    label.clone()
                } else {
                    line[reference_offset..reference_close].replace("\\]", "]")
                };
                let definition = find_link_reference_definition(source, &reference_label);
                return Some(LinkTarget {
                    label,
                    destination: definition
                        .as_ref()
                        .map(|definition| definition.destination.clone())
                        .unwrap_or_default(),
                    title: definition.and_then(|definition| definition.title),
                    reference_label: Some(reference_label),
                    full_range: line_start + full.start..line_start + full.end,
                });
            }
            offset = full.end;
        } else {
            offset = after;
        }
    }
    None
}

fn find_link_reference_definition(source: &str, label: &str) -> Option<LinkReferenceDefinition> {
    for (line_start, line) in split_lines_with_offsets(source) {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let Some(rest) = trimmed.strip_prefix('[') else {
            continue;
        };
        let Some(label_end) = rest.find("]:") else {
            continue;
        };
        let current_label = rest[..label_end].replace("\\]", "]");
        if current_label != label {
            continue;
        }
        let inner = rest[label_end + 2..].trim();
        if inner.is_empty() {
            continue;
        }
        let (destination, title) = split_image_destination(inner);
        return Some(LinkReferenceDefinition {
            label: current_label,
            destination,
            title,
            full_range: line_start..line_start + trimmed.len(),
        });
    }
    None
}

fn find_link_reference_targets(source: &str) -> Vec<LinkReferenceTarget> {
    let mut targets = Vec::new();
    for (line_start, line) in split_lines_with_offsets(source) {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.starts_with('[') && trimmed.contains("]:") {
            continue;
        }
        let mut offset = 0;
        while let Some(open_relative) = trimmed[offset..].find('[') {
            let open = offset + open_relative;
            if open > 0 && trimmed.as_bytes().get(open - 1) == Some(&b'!') {
                offset = open + 1;
                continue;
            }
            let Some(label_close_relative) = trimmed[open + 1..].find(']') else {
                break;
            };
            let label_close = open + 1 + label_close_relative;
            let after_label = label_close + 1;
            let Some(reference_start) = trimmed[after_label..].strip_prefix('[') else {
                offset = after_label;
                continue;
            };
            let Some(reference_close_relative) = reference_start.find(']') else {
                offset = after_label;
                continue;
            };
            let reference_start = after_label + 1;
            let reference_close = reference_start + reference_close_relative;
            let label = trimmed[open + 1..label_close].replace("\\]", "]");
            let reference_label = if reference_close == reference_start {
                label
            } else {
                trimmed[reference_start..reference_close].replace("\\]", "]")
            };
            targets.push(LinkReferenceTarget {
                reference_label,
                full_range: line_start + open..line_start + reference_close + 1,
            });
            offset = reference_close + 1;
        }
    }
    targets
}

fn parse_image_syntax(text: &str) -> Option<ImageTarget> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix("![")?;
    let alt_end = rest.find("](")?;
    let alt = rest[..alt_end].replace("\\]", "]");
    let after_alt = &rest[alt_end + 2..];
    let close = after_alt.find(')')?;
    let inner = after_alt[..close].trim();
    if inner.is_empty() {
        return None;
    }
    let (source, title) = split_image_destination(inner);
    Some(ImageTarget {
        alt,
        source,
        title,
        full_range: 0..trimmed.len().min(2 + alt_end + 2 + close + 1),
    })
}

fn split_image_destination(inner: &str) -> (String, Option<String>) {
    let mut parts = inner.splitn(2, char::is_whitespace);
    let source = parts.next().unwrap_or_default().to_string();
    let title = parts.next().and_then(|rest| {
        let trimmed = rest.trim();
        trimmed
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .map(|value| value.replace("\\\"", "\""))
    });
    (source, title)
}

fn is_local_image_path(source: &str) -> bool {
    !(source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:")
        || source.starts_with('#'))
}

fn find_footnote_target_at(source: &str, byte: usize) -> Option<FootnoteTarget> {
    find_footnote_refs(source)
        .into_iter()
        .chain(
            find_footnote_definitions(source)
                .into_iter()
                .map(|definition| FootnoteTarget {
                    label: definition.label,
                    label_range: definition.label_range,
                    full_range: definition.full_range,
                    definition: true,
                }),
        )
        .find(|target| {
            byte >= target.full_range.start.saturating_sub(1) && byte <= target.full_range.end + 1
        })
}

fn find_footnote_refs(source: &str) -> Vec<FootnoteTarget> {
    let mut refs = Vec::new();
    for (line_start, line) in split_lines_with_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[^") && trimmed.contains("]:") {
            continue;
        }
        let mut search_offset = 0;
        while let Some(target) =
            find_footnote_ref_in_line(&line[search_offset..], line_start + search_offset)
        {
            search_offset = target.full_range.end.saturating_sub(line_start);
            refs.push(target);
        }
    }
    refs
}

fn find_footnote_ref_in_line(line: &str, line_start: usize) -> Option<FootnoteTarget> {
    let start = line.find("[^")?;
    let label_start = start + 2;
    let end_relative = line[label_start..].find(']')?;
    let label_end = label_start + end_relative;
    if label_end == label_start {
        return None;
    }
    Some(FootnoteTarget {
        label: line[label_start..label_end].to_string(),
        label_range: line_start + label_start..line_start + label_end,
        full_range: line_start + start..line_start + label_end + 1,
        definition: false,
    })
}

fn find_footnote_definitions(source: &str) -> Vec<FootnoteDefinition> {
    split_lines_with_offsets(source)
        .into_iter()
        .filter_map(|(line_start, line)| {
            let trim_offset = line.len().saturating_sub(line.trim_start().len());
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("[^")?;
            let label_end = rest.find("]:")?;
            if label_end == 0 {
                return None;
            }
            let label_start = line_start + trim_offset + 2;
            let marker_end = line_start + trim_offset + 2 + label_end + 2;
            Some(FootnoteDefinition {
                label: rest[..label_end].to_string(),
                label_range: label_start..label_start + label_end,
                full_range: line_start..line_start + line.len(),
                content_start: marker_end
                    + line[marker_end.saturating_sub(line_start)..]
                        .chars()
                        .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                        .map(char::len_utf8)
                        .sum::<usize>(),
            })
        })
        .collect()
}

fn find_footnote_definition(source: &str, label: &str) -> Option<FootnoteDefinition> {
    find_footnote_definitions(source)
        .into_iter()
        .find(|definition| definition.label == label)
}

fn previous_grapheme_boundary(source: &str, byte: usize) -> usize {
    let mut previous = 0;
    for (index, _) in source.grapheme_indices(true) {
        if index >= byte {
            break;
        }
        previous = index;
    }
    previous
}

fn next_grapheme_boundary(source: &str, byte: usize) -> usize {
    for (index, grapheme) in source[byte.min(source.len())..].grapheme_indices(true) {
        if index > 0 {
            return byte + index;
        }
        if byte + index == source.len() {
            return source.len();
        }
        if byte + grapheme.len() > byte {
            return byte + grapheme.len();
        }
    }
    source.len()
}

fn clamp_to_grapheme_boundary(source: &str, byte: usize) -> usize {
    if source.is_char_boundary(byte) {
        byte
    } else {
        previous_grapheme_boundary(source, byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semantic_components() {
        let arena = parse_components("# Title\n\n- [x] task\n\n```rust\nfn main() {}\n```\n");
        assert!(
            arena
                .components
                .iter()
                .any(|component| { matches!(component.kind, ComponentKind::Heading { level: 1 }) })
        );
        assert!(arena.components.iter().any(|component| {
            matches!(component.kind, ComponentKind::List { ordered: false, .. })
        }));
        assert!(arena.components.iter().any(|component| {
            matches!(component.kind, ComponentKind::CodeBlock { ref language, .. } if language == "rust")
        }));
    }

    #[test]
    fn tree_sitter_summary_tracks_block_and_inline_parse() {
        let document = Document::new(None, "# Title\n\nInline [content](https://example.com).\n");
        let parse = &document.parse.tree_sitter;

        assert!(parse.parsed);
        assert!(!parse.reused_previous_tree);
        assert_eq!(parse.root_kind, "document");
        assert!(parse.block_node_count > 1);
        assert!(parse.inline_tree_count > 0);
        assert!(
            parse
                .named_nodes
                .iter()
                .any(|node| node.kind == "atx_heading")
        );
        assert!(
            parse
                .named_nodes
                .iter()
                .any(|node| node.kind == "paragraph")
        );
    }

    #[test]
    fn tree_sitter_parse_reuses_edited_previous_tree() {
        let mut document = Document::new(None, "# Title\n\nBody\n");
        document.set_cursor_byte(document.source().len());
        document.insert_text("more");

        assert!(document.parse.tree_sitter.parsed);
        assert!(document.parse.tree_sitter.reused_previous_tree);
    }

    #[test]
    fn source_edit_tracks_multiline_points() {
        let old_source = "one\ntwo\nthree";
        let new_source = "one\ntwo!\nthree";
        let edit = source_edit(old_source, new_source).unwrap_or_else(|| panic!("missing edit"));

        assert_eq!(edit.start_byte, "one\ntwo".len());
        assert_eq!(edit.old_end_byte, "one\ntwo".len());
        assert_eq!(edit.new_end_byte, "one\ntwo!".len());
        assert_eq!(edit.start_position, Point::new(1, 3));
        assert_eq!(edit.old_end_position, Point::new(1, 3));
        assert_eq!(edit.new_end_position, Point::new(1, 4));
    }

    #[test]
    fn edits_source_and_undoes() {
        let mut document = Document::new(None, "hello");
        document.set_cursor_byte(5);
        document.insert_char('!');
        assert_eq!(document.source(), "hello!");
        assert!(document.dirty.is_dirty);
        assert!(document.undo());
        assert_eq!(document.source(), "hello");
        assert!(document.redo());
        assert_eq!(document.source(), "hello!");
    }

    #[test]
    fn cursor_moves_by_grapheme() {
        let mut document = Document::new(None, "a🙂b");
        document.move_right();
        assert_eq!(document.cursor.byte, 1);
        document.move_right();
        assert_eq!(document.cursor.byte, "a🙂".len());
        document.backspace();
        assert_eq!(document.source(), "ab");
    }

    #[test]
    fn toggles_task_marker_at_cursor() {
        let mut document = Document::new(None, "- [ ] task\n");
        document.set_cursor_byte(4);

        assert!(document.toggle_task_at_cursor());
        assert_eq!(document.source(), "- [x] task\n");
        assert!(document.toggle_task_at_cursor());
        assert_eq!(document.source(), "- [ ] task\n");
    }

    #[test]
    fn edits_code_language_fence() {
        let mut document = Document::new(None, "```rust\nfn main() {}\n```\n");
        document.set_cursor_byte(5);

        assert!(document.set_code_language_at_cursor("mermaid"));
        assert!(document.source().starts_with("```mermaid\n"));
        assert_eq!(
            document.code_body_at_cursor().as_deref(),
            Some("fn main() {}")
        );
    }

    #[test]
    fn exposes_and_focuses_code_block_fields() {
        let mut document = Document::new(None, "```rust\nfn main() {}\n```\n");
        document.set_cursor_byte(5);

        let fields = document
            .code_block_fields_at_cursor()
            .unwrap_or_else(|| panic!("missing code fields"));
        assert_eq!(
            &document.source()[fields.language.start..fields.language.end],
            "rust"
        );
        assert_eq!(
            &document.source()[fields.body.start..fields.body.end],
            "fn main() {}\n"
        );

        assert!(document.focus_code_body_at_cursor(false));
        assert_eq!(document.cursor.byte, fields.body.start);
        assert!(document.focus_code_body_at_cursor(true));
        assert_eq!(document.cursor.byte, fields.body.end);
        assert!(document.focus_code_language_at_cursor());
        assert_eq!(document.cursor.byte, fields.language.start);
    }

    #[test]
    fn inserts_link_around_current_word() {
        let mut document = Document::new(None, "visit site now");
        document.set_cursor_byte(7);

        assert!(document.insert_link_at_cursor("https://example.com/a)b"));
        assert_eq!(
            document.source(),
            "visit [site](https://example.com/a%29b) now"
        );
    }

    #[test]
    fn edits_inline_link_fields_at_cursor() {
        let mut document = Document::new(None, "visit [site](https://old.example \"Old\") now");
        document.set_cursor_byte(8);

        let link = document
            .link_at_cursor()
            .unwrap_or_else(|| panic!("missing link"));
        assert_eq!(link.label, "site");
        assert_eq!(link.destination, "https://old.example");
        assert_eq!(link.title.as_deref(), Some("Old"));

        assert!(document.set_link_at_cursor("docs", "https://new.example/a)b", Some("New"), None));
        assert_eq!(
            document.source(),
            "visit [docs](https://new.example/a%29b \"New\") now"
        );
    }

    #[test]
    fn edits_reference_link_and_definition_at_cursor() {
        let mut document = Document::new(None, "visit [site][old]\n\n[old]: https://old \"Old\"\n");
        document.set_cursor_byte(8);

        assert!(document.set_link_at_cursor(
            "docs",
            "https://new.example",
            Some("New"),
            Some("ref")
        ));
        assert_eq!(
            document.source(),
            "visit [docs][ref]\n\n[ref]: https://new.example \"New\"\n"
        );
    }

    #[test]
    fn diagnoses_missing_link_reference_definitions() {
        let document = Document::new(None, "visit [docs][missing]\n\n[ok]: https://example.com\n");

        assert!(document.parse.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("missing link reference definition: missing")
                && diagnostic.fix.as_deref() == Some("Create reference definition")
        }));
    }

    #[test]
    fn toggles_inline_marks_around_current_word() {
        let mut document = Document::new(None, "alpha beta gamma");
        document.set_cursor_byte(7);

        assert!(document.toggle_strong_at_cursor());
        assert_eq!(document.source(), "alpha **beta** gamma");
        assert!(document.toggle_strong_at_cursor());
        assert_eq!(document.source(), "alpha beta gamma");
        assert!(document.toggle_emphasis_at_cursor());
        assert_eq!(document.source(), "alpha _beta_ gamma");
        assert!(document.toggle_emphasis_at_cursor());
        assert!(document.toggle_inline_code_at_cursor());
        assert_eq!(document.source(), "alpha `beta` gamma");
        assert!(document.toggle_inline_code_at_cursor());
        assert!(document.toggle_strikethrough_at_cursor());
        assert_eq!(document.source(), "alpha ~~beta~~ gamma");
    }

    #[test]
    fn selection_replaces_text_and_accepts_inline_marks() {
        let mut document = Document::new(None, "alpha beta");
        document.set_cursor_byte(0);
        document.move_right_select();
        assert_eq!(document.selection_range(), Some(0..1));
        document.insert_text("A");
        assert_eq!(document.source(), "Alpha beta");

        document.set_cursor_byte(6);
        for _ in 0..4 {
            document.move_right_select();
        }
        assert_eq!(document.selection_range(), Some(6..10));
        assert!(document.toggle_strong_at_cursor());
        assert_eq!(document.source(), "Alpha **beta**");

        document.select_all();
        assert_eq!(document.selection_range(), Some(0..document.len_bytes()));
        document.backspace();
        assert_eq!(document.source(), "");
    }

    #[test]
    fn inserts_block_template_at_gap_with_separators() {
        let mut document = Document::new(None, "before\n\nafter");
        document.set_cursor_byte("before\n".len());

        document.insert_block_at_cursor("## Heading", "## ".len());

        assert_eq!(document.source(), "before\n\n## Heading\n\nafter");
        assert_eq!(document.cursor.byte, "before\n\n## ".len());
        assert!(
            document
                .components
                .components
                .iter()
                .any(|component| matches!(component.kind, ComponentKind::Heading { level: 2 }))
        );
    }

    #[test]
    fn inserts_block_template_into_empty_document() {
        let mut document = Document::new(None, "");

        document.insert_block_at_cursor("```text\n\n```", "```text\n".len());

        assert_eq!(document.source(), "```text\n\n```\n");
        assert_eq!(document.cursor.byte, "```text\n".len());
        assert!(
            document
                .components
                .components
                .iter()
                .any(|component| matches!(component.kind, ComponentKind::CodeBlock { .. }))
        );
    }

    #[test]
    fn edits_heading_text_and_level_at_cursor() {
        let mut document = Document::new(None, "# Old title\n\nbody");
        document.set_cursor_byte(3);

        let heading = document
            .heading_at_cursor()
            .unwrap_or_else(|| panic!("missing heading"));
        assert_eq!(heading.level, 1);
        assert_eq!(heading.text, "Old title");

        assert!(document.set_heading_at_cursor(3, "New title"));
        assert_eq!(document.source(), "### New title\n\nbody");
        assert_eq!(document.cursor.byte, "### ".len());

        assert!(document.set_heading_level_at_cursor(2));
        assert_eq!(document.source(), "## New title\n\nbody");
    }

    #[test]
    fn deterministic_inline_edit_fuzz_regression() {
        let cases = [
            "plain",
            "stars*inside",
            "under_score",
            "paren)inside",
            "pipe|inside",
            "emoji🙂inside",
            "back`tick",
        ];

        for case in cases {
            let mut document = Document::new(None, format!("before {case} after"));
            let cursor = document.source().find(case).unwrap_or_default() + case.len();
            document.set_cursor_byte(cursor);

            assert!(document.toggle_strong_at_cursor());
            assert!(document.undo());
            assert!(document.redo());
            assert!(document.parse.tree_sitter.parsed);

            document.set_cursor_byte(cursor.min(document.len_bytes()));
            let _changed = document.toggle_emphasis_at_cursor();
            assert!(document.parse.tree_sitter.parsed);

            document.set_cursor_byte(cursor.min(document.len_bytes()));
            let _changed = document.toggle_inline_code_at_cursor();
            assert!(document.parse.tree_sitter.parsed);

            document.set_cursor_byte(cursor.min(document.len_bytes()));
            let _changed = document.insert_link_at_cursor("https://example.com/a)b");
            assert!(document.parse.tree_sitter.parsed);
            assert!(!document.source().contains("https://example.com/a)b"));
        }
    }

    #[test]
    fn footnote_refs_jump_create_and_rename_definitions() {
        let mut document = Document::new(None, "body[^one]\n");
        document.set_cursor_byte(5);

        assert_eq!(document.footnote_label_at_cursor().as_deref(), Some("one"));
        assert!(
            document
                .parse
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("missing footnote"))
        );
        assert!(document.create_footnote_definition_at_cursor());
        assert!(document.source().contains("[^one]: "));
        assert!(document.jump_to_footnote_definition_at_cursor());
        assert!(document.rename_footnote_at_cursor("two"));
        assert!(document.source().contains("[^two]: "));
    }

    #[test]
    fn parses_edits_and_diagnoses_images() {
        let mut document = Document::new(None, "![old](missing.png \"caption\")\n");
        document.set_cursor_byte(2);

        assert!(
            document
                .components
                .components
                .iter()
                .any(|component| matches!(component.kind, ComponentKind::Image))
        );
        assert!(
            document
                .parse
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("broken local image"))
        );
        assert_eq!(
            document.image_at_cursor().map(|image| image.alt),
            Some("old".to_string())
        );
        assert!(document.set_image_at_cursor("new", "https://example.com/a.png", Some("title")));
        assert_eq!(
            document.source(),
            "![new](https://example.com/a.png \"title\")\n"
        );
    }

    #[test]
    fn inserts_and_removes_table_rows_and_columns() {
        let mut document = Document::new(None, "| A | B |\n| - | - |\n| 1 | 2 |\n");
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());

        assert_eq!(document.table_cell_at_cursor(), Some((1, 0)));
        assert!(document.insert_table_row_at_cursor(false));
        assert!(document.source().contains("|     |     |"));
        assert!(document.insert_table_column_at_cursor(true));
        assert!(
            document
                .source()
                .lines()
                .next()
                .unwrap_or_default()
                .matches('|')
                .count()
                >= 4
        );
        assert!(document.remove_table_column_at_cursor());
        assert!(document.remove_table_row_at_cursor());
    }

    #[test]
    fn navigates_table_cells_and_enter_adds_row_at_end() {
        let mut document = Document::new(None, "| A | B |\n| - | - |\n| 1 | 2 |\n");
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());

        assert_eq!(document.table_cell_at_cursor(), Some((1, 0)));
        assert!(document.move_table_cell(true));
        assert_eq!(document.table_cell_at_cursor(), Some((1, 1)));
        assert!(document.enter_table_cell());
        assert_eq!(document.table_cell_at_cursor(), Some((2, 1)));
        assert!(document.source().lines().count() > 3);
        assert!(document.move_table_cell(false));
        assert_eq!(document.table_cell_at_cursor(), Some((2, 0)));
    }

    #[test]
    fn removes_empty_table_row_at_cursor_only_when_empty() {
        let mut document = Document::new(None, "| A | B |\n| - | - |\n| 1 | 2 |\n|   |   |\n");
        let empty_cell = document.source().rfind("|   |").unwrap_or_default() + 2;
        document.set_cursor_byte(empty_cell);

        assert!(document.remove_empty_table_row_at_cursor());
        assert_eq!(document.source().lines().count(), 3);
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());
        assert!(!document.remove_empty_table_row_at_cursor());
    }

    #[test]
    fn changes_and_preserves_table_column_alignment() {
        let mut document = Document::new(None, "| A | B |\n| :-- | --: |\n| 1 | 2 |\n");
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());

        assert!(document.change_table_column_alignment_at_cursor(true));
        assert!(document.source().contains("| :-: | --: |"));
        assert!(document.change_table_column_alignment_at_cursor(true));
        assert!(document.source().contains("--:"));
        assert!(document.insert_table_column_at_cursor(false));
        assert!(document.source().contains("---"));
        assert!(document.remove_table_column_at_cursor());
        assert!(document.source().contains("--:"));
    }

    #[test]
    fn removes_selected_table_row_and_column_by_index() {
        let mut document = Document::new(
            None,
            "| A | B | C |\n| - | - | - |\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n",
        );
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());

        assert!(document.remove_table_column(1));
        assert!(!document.source().contains(" B "));
        assert!(!document.source().contains(" 2 "));
        assert!(document.remove_table_row(1));
        assert!(!document.source().contains(" 1 "));
    }

    #[test]
    fn normalizes_touched_table_only() {
        let mut document = Document::new(None, "before\n\n| A|B |\n|-|-|\n| 1|2|\n\nafter");
        document.set_cursor_byte(document.source().find('1').unwrap_or_default());

        assert!(document.normalize_table_at_cursor());
        assert!(document.source().contains("| A   | B   |"));
        assert!(document.source().contains("| --- | --- |"));
        assert!(document.source().starts_with("before"));
        assert!(document.source().ends_with("after"));
    }
}
