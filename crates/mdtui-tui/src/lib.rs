use mdtui_core::{Block, Cursor, Direction, Editor, InlineMark};
use mdtui_markdown::{export_gfm, import_gfm};
use mdtui_render::{RenderOptions, Rendered, hit_test_or_nearest, render_editor};

#[derive(Clone, Debug)]
pub struct App {
    pub editor: Editor,
    pub file_name: String,
    pub render_options: RenderOptions,
}

impl Default for App {
    fn default() -> Self {
        Self::from_markdown("untitled.md", "")
    }
}

impl App {
    pub fn from_markdown(file_name: impl Into<String>, source: &str) -> Self {
        let mut editor = Editor::new(import_gfm(source));
        if let Some(cursor) = first_non_headline_cursor(&editor.document.blocks) {
            editor.set_cursor(cursor);
        }
        Self {
            editor,
            file_name: file_name.into(),
            render_options: RenderOptions::default(),
        }
    }

    pub fn render(&self) -> Rendered {
        render_editor(&self.editor, &self.file_name, self.render_options.clone())
    }

    pub fn render_text(&self) -> String {
        self.render().text()
    }

    pub fn save_to_gfm(&self) -> String {
        export_gfm(&self.editor.document)
    }

    pub fn type_char(&mut self, ch: char) {
        self.editor.press_char(ch);
    }

    pub fn enter(&mut self) {
        self.editor.enter();
    }

    pub fn backspace(&mut self) {
        self.editor.backspace();
    }

    pub fn delete(&mut self) {
        self.editor.delete();
    }

    pub fn shift_right(&mut self) {
        self.editor.move_right(true);
    }

    pub fn shift_down(&mut self) {
        self.editor.move_down(true);
    }

    pub fn ctrl_arrow(&mut self, direction: Direction) {
        self.editor.ctrl_arrow(direction);
    }

    pub fn tab(&mut self) {
        self.editor.tab(false);
    }

    pub fn shift_tab(&mut self) {
        self.editor.tab(true);
    }

    pub fn apply_bold(&mut self) {
        self.editor.apply_mark(InlineMark::Strong);
    }

    pub fn apply_italic(&mut self) {
        self.editor.apply_mark(InlineMark::Emphasis);
    }

    pub fn apply_strike(&mut self) {
        self.editor.apply_mark(InlineMark::Strike);
    }

    pub fn apply_code(&mut self) {
        self.editor.apply_mark(InlineMark::Code);
    }

    pub fn apply_code_block(&mut self) {
        self.editor.toggle_code_block();
    }

    pub fn apply_superscript(&mut self) {
        self.editor.apply_mark(InlineMark::Superscript);
    }

    pub fn apply_subscript(&mut self) {
        self.editor.apply_mark(InlineMark::Subscript);
    }

    pub fn apply_block_quote(&mut self) {
        self.editor.toggle_block_quote();
    }

    pub fn clear_styles(&mut self) {
        self.editor.clear_styles();
    }

    pub fn click(&mut self, x: u16, y: u16) {
        let rendered = self.render();
        if let Some(cursor) = hit_test_or_nearest(x, y, &rendered.display) {
            if let Cursor::Checkbox { block, item } = cursor {
                self.editor.toggle_checkbox(block, item);
            } else {
                self.editor.set_cursor(cursor);
            }
        }
    }

    pub fn drag_select(&mut self, from: (u16, u16), to: (u16, u16)) {
        let rendered = self.render();
        if let (Some(anchor), Some(head)) = (
            hit_test_or_nearest(from.0, from.1, &rendered.display),
            hit_test_or_nearest(to.0, to.1, &rendered.display),
        ) {
            self.editor.select_range(anchor, head);
        }
    }

    pub fn help_popup() -> String {
        [
            "╭─ Help ─────────────────────────────╮",
            "│ Navigation: arrows, shift+arrows   │",
            "│ Editing: type, enter, backspace    │",
            "│ Tables: tab, shift+tab, ctrl+arrow │",
            "│ Lists: enter, backspace, space     │",
            "│ Misc: ctrl-s save, ctrl-q quit, ?  │",
            "╰────────────────────────────────────╯",
        ]
        .join("\n")
    }
}

fn first_non_headline_cursor(blocks: &[Block]) -> Option<Cursor> {
    blocks
        .iter()
        .enumerate()
        .find_map(|(block, item)| match item {
            Block::Heading { .. } | Block::ThematicBreak => None,
            Block::List(list) if list.items.is_empty() => None,
            Block::List(_) => Some(Cursor::ListItem {
                block,
                item: 0,
                offset: 0,
            }),
            Block::Table(table) if table.rows.is_empty() || table.rows[0].cells.is_empty() => None,
            Block::Table(_) => Some(Cursor::TableCell {
                block,
                row: 0,
                col: 0,
                offset: 0,
            }),
            Block::Paragraph(_)
            | Block::BlockQuote(_)
            | Block::CodeBlock { .. }
            | Block::ImageBlock { .. }
            | Block::HtmlBlock(_)
            | Block::Frontmatter(_) => Some(Cursor::Text { block, offset: 0 }),
        })
}
