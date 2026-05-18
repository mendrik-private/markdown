use comrak::{Options, markdown_to_html};
use mdtui_core::Document;

#[derive(Clone, Debug)]
pub struct MarkdownImport {
    pub document: Document,
    pub html_oracle: String,
}

pub fn import_gfm(source: &str) -> MarkdownImport {
    MarkdownImport {
        document: Document::new(None, source),
        html_oracle: gfm_html(source),
    }
}

pub fn export_gfm(document: &Document) -> String {
    document.source()
}

pub fn gfm_html(source: &str) -> String {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.footnotes = true;
    markdown_to_html(source, &options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comrak_oracle_enables_gfm_tables_and_tasks() {
        let html = gfm_html("| A |\n| - |\n| B |\n\n- [x] done\n");
        assert!(html.contains("<table>"));
        assert!(html.contains("checkbox"));
    }
}
