use mdtui_core::{
    Alignment, Block, Document, Inline, List, ListItem, Table, TableCell, TableRow, inline_text,
};

pub fn import_gfm(source: &str) -> Document {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        if let Some(language) = line.trim().strip_prefix("```") {
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                body.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            blocks.push(Block::CodeBlock {
                language: (!language.trim().is_empty()).then(|| language.trim().to_string()),
                text: body.join("\n"),
            });
            continue;
        }

        if let Some((level, text)) = parse_heading(line) {
            blocks.push(Block::Heading {
                level,
                inlines: parse_inlines(text),
            });
            i += 1;
            continue;
        }

        if is_table_start(&lines, i) {
            let mut table_lines = Vec::new();
            while i < lines.len() && looks_like_table_row(lines[i]) {
                table_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Table(parse_table(&table_lines)));
            continue;
        }

        if let Some((list, consumed)) = parse_list(&lines[i..]) {
            blocks.push(Block::List(list));
            i += consumed;
            continue;
        }

        if line.trim_start().starts_with('<') && line.trim_end().ends_with('>') {
            blocks.push(Block::HtmlBlock(line.to_string()));
            i += 1;
            continue;
        }

        if line.trim_start().starts_with("> ") {
            let mut quoted = Vec::new();
            while i < lines.len() {
                let Some(rest) = lines[i].trim_start().strip_prefix("> ") else {
                    break;
                };
                quoted.push(Block::Paragraph(parse_inlines(rest)));
                i += 1;
            }
            blocks.push(Block::BlockQuote(quoted));
            continue;
        }

        if line.trim() == "---" || line.trim() == "***" {
            blocks.push(Block::ThematicBreak);
            i += 1;
            continue;
        }

        if let Some((alt, src)) = parse_image_line(line.trim()) {
            blocks.push(Block::ImageBlock { src, alt });
            i += 1;
            continue;
        }

        let mut paragraph = vec![line.trim()];
        i += 1;
        while i < lines.len()
            && !lines[i].trim().is_empty()
            && parse_heading(lines[i]).is_none()
            && !looks_like_table_row(lines[i])
            && parse_list_item(lines[i]).is_none()
            && !lines[i].trim().starts_with("```")
        {
            paragraph.push(lines[i].trim());
            i += 1;
        }
        blocks.push(Block::Paragraph(parse_inlines(&paragraph.join(" "))));
    }

    if blocks.is_empty() {
        Document::default()
    } else {
        Document::new(blocks)
    }
}

pub fn export_gfm(document: &Document) -> String {
    let mut out = Vec::new();
    for block in &document.blocks {
        out.push(export_block(block));
    }
    out.join("\n\n")
}

pub fn semantic_equivalent_after_roundtrip(source: &str) -> bool {
    let first = import_gfm(source);
    let exported = export_gfm(&first);
    let second = import_gfm(&exported);
    first.rendered_text() == second.rendered_text()
}

fn export_block(block: &Block) -> String {
    match block {
        Block::Paragraph(inlines) => export_inlines(inlines),
        Block::Heading { level, inlines } => {
            format!(
                "{} {}",
                "#".repeat((*level).into()),
                export_inlines(inlines)
            )
        }
        Block::BlockQuote(blocks) => blocks
            .iter()
            .map(export_block)
            .flat_map(|text| {
                text.lines()
                    .map(|line| format!("> {line}"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::List(list) => export_list(list),
        Block::CodeBlock { language, text } => {
            format!("```{}\n{}\n```", language.as_deref().unwrap_or(""), text)
        }
        Block::Table(table) => export_table(table),
        Block::ThematicBreak => "---".to_string(),
        Block::ImageBlock { src, alt } => format!("![{alt}]({src})"),
        Block::HtmlBlock(html) | Block::Frontmatter(html) => html.clone(),
    }
}

fn export_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(text),
            Inline::Emphasis(children) => {
                out.push('*');
                out.push_str(&export_inlines(children));
                out.push('*');
            }
            Inline::Strong(children) => {
                out.push_str("**");
                out.push_str(&export_inlines(children));
                out.push_str("**");
            }
            Inline::Strike(children) => {
                out.push_str("~~");
                out.push_str(&export_inlines(children));
                out.push_str("~~");
            }
            Inline::InlineCode(text) => {
                out.push('`');
                out.push_str(text);
                out.push('`');
            }
            Inline::Link {
                target, children, ..
            } => {
                out.push('[');
                out.push_str(&export_inlines(children));
                out.push_str("](");
                out.push_str(target);
                out.push(')');
            }
            Inline::Image { src, alt, .. } => {
                out.push_str("![");
                out.push_str(alt);
                out.push_str("](");
                out.push_str(src);
                out.push(')');
            }
            Inline::HtmlInline(html) => out.push_str(html),
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("  \n"),
        }
    }
    out
}

fn export_list(list: &List) -> String {
    list.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let marker = if let Some(checked) = item.checked {
                if checked { "- [x] " } else { "- [ ] " }.to_string()
            } else if list.ordered {
                format!("{}. ", index + 1)
            } else {
                "- ".to_string()
            };
            let body = item
                .blocks
                .iter()
                .map(export_block)
                .collect::<Vec<_>>()
                .join("\n");
            format!("{marker}{body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn export_table(table: &Table) -> String {
    let (_, cols) = table.dimensions();
    let mut lines = Vec::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut cells = Vec::new();
        for col in 0..cols {
            cells.push(
                row.cells
                    .get(col)
                    .map_or_else(String::new, TableCell::rendered_text),
            );
        }
        lines.push(format!("| {} |", cells.join(" | ")));
        if row_index + 1 == table.header_rows {
            lines.push(format!("| {} |", vec!["---"; cols].join(" | ")));
        }
    }
    lines.join("\n")
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
        Some((level as u8, trimmed[level + 1..].trim()))
    } else {
        None
    }
}

fn is_table_start(lines: &[&str], index: usize) -> bool {
    index + 1 < lines.len()
        && looks_like_table_row(lines[index])
        && lines[index + 1]
            .chars()
            .all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
        && lines[index + 1].contains('-')
}

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn parse_table(lines: &[&str]) -> Table {
    let mut rows = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index == 1 && line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')) {
            continue;
        }
        rows.push(split_table_cells(line));
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    Table {
        alignments: vec![Alignment::Left; width],
        rows: rows
            .into_iter()
            .map(|cells| TableRow {
                cells: cells
                    .into_iter()
                    .map(|cell| TableCell {
                        blocks: vec![Block::Paragraph(parse_inlines(&cell))],
                    })
                    .collect(),
            })
            .collect(),
        header_rows: 1,
        horizontal_scroll: 0,
    }
}

fn split_table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn parse_list(lines: &[&str]) -> Option<(List, usize)> {
    let mut items = Vec::new();
    let mut ordered = false;
    let mut consumed = 0;
    for line in lines {
        let Some(item) = parse_list_item(line) else {
            break;
        };
        ordered |= item.0;
        items.push(item.1);
        consumed += 1;
    }
    (!items.is_empty()).then_some((
        List {
            ordered,
            tight: true,
            items,
        },
        consumed,
    ))
}

fn parse_list_item(line: &str) -> Option<(bool, ListItem)> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- [x] ") {
        return Some((false, parsed_list_item(rest, Some(true))));
    }
    if let Some(rest) = trimmed.strip_prefix("- [X] ") {
        return Some((false, parsed_list_item(rest, Some(true))));
    }
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return Some((false, parsed_list_item(rest, Some(false))));
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some((false, parsed_list_item(rest, None)));
    }
    if let Some(dot) = trimmed.find(". ") {
        let number = &trimmed[..dot];
        if !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()) {
            return Some((true, parsed_list_item(&trimmed[dot + 2..], None)));
        }
    }
    None
}

fn parsed_list_item(markdown: &str, checked: Option<bool>) -> ListItem {
    ListItem {
        checked,
        blocks: vec![Block::Paragraph(parse_inlines(markdown))],
    }
}

fn parse_image_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("![")?;
    let close_alt = rest.find("](")?;
    let alt = rest[..close_alt].to_string();
    let after = &rest[close_alt + 2..];
    let close_src = after.find(')')?;
    Some((alt, after[..close_src].to_string()))
}

fn parse_inlines(source: &str) -> Vec<Inline> {
    let mut inlines = Vec::new();
    let mut rest = source;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix("**")
            && let Some(end) = stripped.find("**")
        {
            inlines.push(Inline::Strong(parse_inlines(&stripped[..end])));
            rest = &stripped[end + 2..];
            continue;
        }
        if let Some(stripped) = rest.strip_prefix("~~")
            && let Some(end) = stripped.find("~~")
        {
            inlines.push(Inline::Strike(parse_inlines(&stripped[..end])));
            rest = &stripped[end + 2..];
            continue;
        }
        if let Some(stripped) = rest.strip_prefix('`')
            && let Some(end) = stripped.find('`')
        {
            inlines.push(Inline::InlineCode(stripped[..end].to_string()));
            rest = &stripped[end + 1..];
            continue;
        }
        if let Some((inline, consumed)) = parse_link_or_image(rest) {
            inlines.push(inline);
            rest = &rest[consumed..];
            continue;
        }
        if let Some(stripped) = rest.strip_prefix('*')
            && let Some(end) = stripped.find('*')
        {
            inlines.push(Inline::Emphasis(parse_inlines(&stripped[..end])));
            rest = &stripped[end + 1..];
            continue;
        }
        if rest.starts_with('<')
            && let Some(end) = rest.find('>')
        {
            inlines.push(Inline::HtmlInline(rest[..=end].to_string()));
            rest = &rest[end + 1..];
            continue;
        }
        let next = next_markup(rest).unwrap_or(rest.len());
        inlines.push(Inline::Text(rest[..next].to_string()));
        rest = &rest[next..];
    }
    merge_text(inlines)
}

fn parse_link_or_image(rest: &str) -> Option<(Inline, usize)> {
    let image = rest.starts_with("![");
    let body = if image {
        &rest[2..]
    } else {
        rest.strip_prefix('[')?
    };
    let close = body.find("](")?;
    let label = &body[..close];
    let after = &body[close + 2..];
    let end = after.find(')')?;
    let target = after[..end].to_string();
    let consumed = if image { 2 } else { 1 } + close + 2 + end + 1;
    let inline = if image {
        Inline::Image {
            src: target,
            alt: label.to_string(),
            title: None,
        }
    } else {
        Inline::Link {
            target,
            title: None,
            children: parse_inlines(label),
        }
    };
    Some((inline, consumed))
}

fn next_markup(rest: &str) -> Option<usize> {
    ["**", "~~", "`", "![", "[", "*", "<"]
        .iter()
        .filter_map(|needle| rest.find(needle))
        .filter(|index| *index > 0)
        .min()
}

fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut merged: Vec<Inline> = Vec::new();
    for inline in inlines {
        match (merged.last_mut(), inline) {
            (Some(Inline::Text(left)), Inline::Text(right)) => left.push_str(&right),
            (_, other) => merged.push(other),
        }
    }
    if merged.is_empty() || inline_text(&merged).is_empty() {
        vec![Inline::Text(String::new())]
    } else {
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_superscript_and_subscript_html() {
        let source = "H<sub>2</sub>O and x<sup>2</sup>";

        let document = import_gfm(source);
        let exported = export_gfm(&document);

        assert_eq!(exported, source);
        assert!(semantic_equivalent_after_roundtrip(source));
    }

    #[test]
    fn consecutive_blockquote_lines_import_as_single_blockquote() {
        let document = import_gfm("> alpha\n> beta");

        assert_eq!(
            document.blocks,
            vec![Block::BlockQuote(vec![
                Block::Paragraph(vec![Inline::Text("alpha".to_string())]),
                Block::Paragraph(vec![Inline::Text("beta".to_string())]),
            ])]
        );
    }
}
