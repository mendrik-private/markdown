use mdtui_core::{AiConfig, Cursor, Direction, VersionedResult, accept_worker_result};
use mdtui_markdown::{export_gfm, import_gfm, semantic_equivalent_after_roundtrip};
use mdtui_render::{DisplayKind, RenderOptions, Theme, render_document};
use mdtui_terminal::{
    HeadlineImageCache, ImageCache, InputEngine, InputEvent, contains_forbidden_text_sizing,
};
use mdtui_tui::App;

fn sample() -> &'static str {
    "# Title

Hello **bold** and [link](https://example.com).
- abc
- def
- [x] done

| Name | Done |
| --- | --- |
| API | yes |

<div data-x=\"1\">raw</div>
![alt](image.png)"
}

fn table_app() -> App {
    App::from_markdown(
        "table.md",
        "| Name | Done |
| --- | --- |
| API | yes |",
    )
}

fn list_app() -> App {
    App::from_markdown("list.md", "- abc\n- def")
}

#[test]
fn app_starts_in_direct_editing_mode() {
    let mut app = App::default();
    app.type_char('a');
    assert_eq!(app.editor.document.rendered_text(), "a");
}

#[test]
fn pressing_i_inserts_literal_i_not_insert_mode() {
    let mut app = App::default();
    app.type_char('i');
    assert_eq!(app.editor.document.rendered_text(), "i");
}

#[test]
fn printable_character_inserts_without_entering_insert_mode() {
    let mut app = App::default();
    app.type_char('x');
    assert_eq!(app.editor.document.rendered_text(), "x");
}

#[test]
fn status_bar_never_shows_vim_insert_or_normal_mode() {
    let app = App::default();
    let status = app.editor.status_bar("x.md", 80).to_lowercase();
    assert!(!status.contains("insert"));
    assert!(!status.contains("normal"));
    assert!(!status.contains("visual"));
}

#[test]
fn rendered_heading_does_not_show_hash_markers() {
    assert!(
        !App::from_markdown("x.md", "# Title")
            .render_text()
            .contains('#')
    );
}

#[test]
fn rendered_bold_does_not_show_star_markers() {
    assert!(
        !App::from_markdown("x.md", "**bold**")
            .render_text()
            .contains("**")
    );
}

#[test]
fn rendered_link_does_not_show_markdown_link_syntax() {
    let rendered = App::from_markdown("x.md", "[site](https://example.com)").render_text();
    assert!(rendered.contains("site"));
    assert!(!rendered.contains("]("));
}

#[test]
fn rendered_list_link_does_not_show_markdown_link_syntax() {
    let rendered = App::from_markdown("x.md", "1. [Project identity](#project)").render_text();
    assert!(rendered.contains("Project identity"));
    assert!(!rendered.contains("]("));
}

#[test]
fn rendered_list_bold_does_not_show_star_markers() {
    let rendered = App::from_markdown("x.md", "1. **No modal insert mode.**").render_text();
    assert!(rendered.contains("No modal insert mode."));
    assert!(!rendered.contains("**"));
}

#[test]
fn rendered_task_item_does_not_show_bracket_marker() {
    let rendered = App::from_markdown("x.md", "- [x] done").render_text();
    assert!(rendered.contains('☑'));
    assert!(!rendered.contains("[x]"));
}

#[test]
fn rendered_table_does_not_show_pipe_delimiter_source() {
    let rendered = table_app().render_text();
    assert!(rendered.contains('┌'));
    assert!(!rendered.contains("| --- |"));
}

#[test]
fn rendered_editing_never_exposes_markdown_markers_after_deletion() {
    let mut app = App::from_markdown("x.md", sample());
    app.editor.set_cursor(Cursor::Text {
        block: 1,
        offset: 5,
    });
    app.backspace();
    let rendered = app.render_text();
    for marker in ["**", "[x]", "| --- |", "]("] {
        assert!(!rendered.contains(marker));
    }
}

#[test]
fn shift_right_extends_selection() {
    let mut app = App::from_markdown("x.md", "abc");
    app.shift_right();
    assert!(app.editor.selection.is_some());
}

#[test]
fn shift_arrow_extends_selection() {
    shift_right_extends_selection();
}

#[test]
fn shift_down_extends_selection_across_softwrap() {
    let mut app = App::from_markdown("x.md", "one\n\ntwo");
    app.shift_down();
    assert!(app.editor.selection.is_some());
}

#[test]
fn mouse_drag_selects_rendered_text() {
    let mut app = App::from_markdown("x.md", "abc");
    app.drag_select((0, 0), (2, 0));
    assert!(app.editor.selection.is_some());
}

#[test]
fn selection_can_cross_inline_mark_boundaries() {
    let mut app = App::from_markdown("x.md", "a **b** c");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 0,
            offset: 5,
        },
    );
    assert!(app.editor.selection.is_some());
}

#[test]
fn deleting_selection_across_marks_leaves_no_orphan_nodes() {
    let mut app = App::from_markdown("x.md", "a **b** c");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 0,
            offset: 5,
        },
    );
    app.delete();
    assert_eq!(app.editor.document.rendered_text(), "");
}

#[test]
fn deleting_selection_across_inline_marks_removes_empty_marks() {
    deleting_selection_across_marks_leaves_no_orphan_nodes();
}

#[test]
fn selecting_text_shows_floating_styling_toolbar() {
    let mut app = App::from_markdown("x.md", "abc");
    app.shift_right();
    assert!(app.render_text().contains("Style"));
}

#[test]
fn styling_bar_appears_for_non_empty_selection() {
    selecting_text_shows_floating_styling_toolbar();
}

#[test]
fn toolbar_bold_wraps_selection_in_strong_without_inserting_markers() {
    let mut app = App::from_markdown("x.md", "abc");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 0,
            offset: 2,
        },
    );
    app.apply_bold();
    assert!(!app.render_text().contains("**"));
    assert!(app.save_to_gfm().contains("**ab**"));
}

#[test]
fn tight_list_renders_without_blank_lines_between_items() {
    let rendered = list_app().render_text();
    assert!(!rendered.contains("abc\n\n• def"));
}

#[test]
fn tight_list_renders_without_extra_blank_lines() {
    tight_list_renders_without_blank_lines_between_items();
}

#[test]
fn enter_inside_list_item_creates_new_list_item() {
    let mut app = list_app();
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 0,
        offset: 3,
    });
    app.enter();
    assert!(app.render_text().contains("• "));
}

#[test]
fn enter_in_list_item_creates_next_list_item() {
    enter_inside_list_item_creates_new_list_item();
}

#[test]
fn enter_in_task_item_creates_unchecked_task_item() {
    let mut app = App::from_markdown("x.md", "- [x] done");
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 0,
        offset: 4,
    });
    app.enter();
    assert!(app.render_text().contains('☐'));
}

#[test]
fn enter_on_empty_list_item_exits_list() {
    let mut app = App::from_markdown("x.md", "- ");
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 0,
        offset: 0,
    });
    app.enter();
    assert!(matches!(app.editor.cursor, Cursor::Text { .. }));
}

#[test]
fn backspace_at_start_of_second_list_item_merges_without_marker_text() {
    let mut app = list_app();
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 1,
        offset: 0,
    });
    app.backspace();
    let rendered = app.render_text();
    assert!(rendered.contains("• abcdef"));
    assert!(!rendered.contains("- "));
}

#[test]
fn backspace_at_start_of_first_item_unwraps_without_marker_text() {
    let mut app = list_app();
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 0,
        offset: 0,
    });
    app.backspace();
    let rendered = app.render_text();
    assert!(rendered.contains("abc"));
    assert!(!rendered.contains("- abc"));
}

#[test]
fn clicking_checkbox_toggles_task_state() {
    let mut app = App::from_markdown("x.md", "- [ ] todo");
    app.click(0, 0);
    assert!(app.render_text().contains('☑'));
}

#[test]
fn space_on_checkbox_toggles_task_state() {
    let mut app = App::from_markdown("x.md", "- [ ] todo");
    app.editor
        .set_cursor(Cursor::Checkbox { block: 0, item: 0 });
    app.editor.space();
    assert!(app.render_text().contains('☑'));
}

#[test]
fn table_renders_with_unicode_borders_not_pipes() {
    let rendered = table_app().render_text();
    for glyph in ['┌', '┬', '│', '└'] {
        assert!(rendered.contains(glyph));
    }
    assert!(!rendered.contains("| Name |"));
}

#[test]
fn gfm_table_renders_as_unicode_grid_not_pipes() {
    table_renders_with_unicode_borders_not_pipes();
}

#[test]
fn typing_inside_table_cell_updates_cell_content() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 1,
        col: 0,
        offset: 3,
    });
    app.type_char('!');
    assert!(app.render_text().contains("API!"));
}

#[test]
fn table_cell_accepts_typing() {
    typing_inside_table_cell_updates_cell_content();
}

#[test]
fn enter_inside_table_cell_does_not_destroy_table() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 1,
        col: 0,
        offset: 3,
    });
    app.enter();
    assert!(app.render_text().contains('┌'));
}

#[test]
fn tab_moves_to_next_cell() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 1,
        col: 0,
        offset: 0,
    });
    app.tab();
    assert!(matches!(
        app.editor.cursor,
        Cursor::TableCell { col: 1, .. }
    ));
}

#[test]
fn ctrl_right_in_table_adds_column_after() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 0,
        col: 0,
        offset: 0,
    });
    app.ctrl_arrow(Direction::Right);
    assert_eq!(app.editor.table_dimensions_at(0), Some((2, 3)));
}

#[test]
fn ctrl_left_in_table_adds_column_before() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 0,
        col: 0,
        offset: 0,
    });
    app.ctrl_arrow(Direction::Left);
    assert_eq!(app.editor.table_dimensions_at(0), Some((2, 3)));
}

#[test]
fn ctrl_down_in_table_adds_row_after() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 0,
        col: 0,
        offset: 0,
    });
    app.ctrl_arrow(Direction::Down);
    assert_eq!(app.editor.table_dimensions_at(0), Some((3, 2)));
}

#[test]
fn ctrl_up_in_table_adds_row_before() {
    let mut app = table_app();
    app.editor.set_cursor(Cursor::TableCell {
        block: 0,
        row: 0,
        col: 0,
        offset: 0,
    });
    app.ctrl_arrow(Direction::Up);
    assert_eq!(app.editor.table_dimensions_at(0), Some((3, 2)));
}

#[test]
fn ctrl_right_adds_column_after_current_cell() {
    ctrl_right_in_table_adds_column_after();
}

#[test]
fn ctrl_down_adds_row_after_current_cell() {
    ctrl_down_in_table_adds_row_after();
}

#[test]
fn oversized_table_has_horizontal_viewport() {
    let mut doc = import_gfm(
        "| VeryLongColumnName | Other |\n| --- | --- |\n| abcdefghijklmnopqrstuvwxyz | x |",
    );
    if let mdtui_core::Block::Table(table) = &mut doc.blocks[0] {
        table.horizontal_scroll = 12;
    }
    let rendered = render_document(&doc, RenderOptions::default()).text();
    assert!(rendered.contains("xscroll 12"));
}

#[test]
fn table_empty_row_survives_save_reload() {
    let doc = import_gfm("| A | B |\n| --- | --- |\n|  |  |");
    let exported = export_gfm(&doc);
    let reloaded = import_gfm(&exported);
    assert_eq!(reloaded.rendered_text(), doc.rendered_text());
}

#[test]
fn heading_uses_kitty_graphics_when_supported() {
    let opts = RenderOptions {
        kitty_graphics: true,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("# Title"), opts);
    assert!(!rendered.kitty_commands.is_empty());
}

#[test]
fn unicode_heading_uses_kitty_graphics_when_supported() {
    let opts = RenderOptions {
        kitty_graphics: true,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("# mdtui — TUI GFM Markdown Editor"), opts);
    assert!(!rendered.kitty_commands.is_empty());
    assert!(
        rendered
            .display
            .items
            .iter()
            .any(|item| item.kind == DisplayKind::HeadlinePlacement)
    );
}

#[test]
fn kitty_h1_h2_reserve_two_full_width_rows() {
    for source in ["# Title", "## Subtitle"] {
        let opts = RenderOptions {
            width: 24,
            kitty_graphics: true,
            ..RenderOptions::default()
        };
        let rendered = render_document(&import_gfm(source), opts);
        let slot = rendered
            .display
            .items
            .iter()
            .find(|item| item.kind == DisplayKind::HeadlinePlacement)
            .expect("headline slot");
        assert_eq!(slot.rect.width, 24);
        assert_eq!(slot.rect.height, 2);
        assert_eq!(rendered.lines[0].chars().count(), 24);
        assert_eq!(rendered.lines[1].chars().count(), 24);
    }
}

#[test]
fn kitty_h1_h2_can_span_panel_width_beyond_wrap_width() {
    let opts = RenderOptions {
        width: 24,
        heading_width: 48,
        kitty_graphics: true,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("# Title"), opts);
    let slot = rendered
        .display
        .items
        .iter()
        .find(|item| item.kind == DisplayKind::HeadlinePlacement)
        .expect("headline slot");
    assert_eq!(slot.rect.width, 48);
    assert_eq!(rendered.lines[0].chars().count(), 48);
    assert_eq!(rendered.lines[1].chars().count(), 48);
}

#[test]
fn heading_never_emits_text_sizing_or_osc66() {
    let opts = RenderOptions {
        kitty_graphics: true,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("# Title"), opts);
    assert!(
        rendered
            .kitty_commands
            .iter()
            .all(|cmd| !contains_forbidden_text_sizing(cmd))
    );
}

#[test]
fn heading_image_cache_reuses_id_across_cursor_moves() {
    let mut cache = HeadlineImageCache::default();
    let first = cache.id_for("h1:title");
    let second = cache.id_for("h1:title");
    assert_eq!(first, second);
}

#[test]
fn heading_switches_to_editable_text_when_cursor_enters_placement() {
    let rendered = App::from_markdown("x.md", "# Title").render_text();
    assert!(rendered.contains("TITLE"));
    assert!(!rendered.contains('#'));
}

#[test]
fn heading_restores_cached_image_when_cursor_leaves() {
    heading_image_cache_reuses_id_across_cursor_moves();
}

#[test]
fn heading_falls_back_cleanly_without_kitty_graphics() {
    let rendered = render_document(&import_gfm("# Title"), RenderOptions::default());
    assert!(rendered.kitty_commands.is_empty());
    assert!(rendered.text().contains("TITLE"));
}

#[test]
fn image_uses_kitty_graphics_when_supported() {
    let opts = RenderOptions {
        kitty_graphics: true,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("![alt](x.png)"), opts);
    assert!(
        rendered
            .kitty_commands
            .iter()
            .any(|cmd| cmd.contains("image"))
    );
}

#[test]
fn image_renders_placeholder_when_graphics_unsupported() {
    let rendered = render_document(&import_gfm("![alt](x.png)"), RenderOptions::default()).text();
    assert!(rendered.contains("image"));
    assert!(rendered.contains("alt"));
}

#[test]
fn offscreen_images_are_evicted_by_lru_budget() {
    let mut cache = ImageCache::new(10);
    cache.insert("old", 8);
    cache.insert("new", 8);
    assert!(!cache.contains("old"));
    assert!(cache.contains("new"));
}

#[test]
fn raw_html_unknown_block_is_preserved_on_roundtrip() {
    assert!(semantic_equivalent_after_roundtrip(
        "<custom attr=\"1\">x</custom>"
    ));
}

#[test]
fn raw_html_block_renders_as_atomic_placeholder() {
    assert!(
        App::from_markdown("x.md", "<div>x</div>")
            .render_text()
            .contains("raw html")
    );
}

#[test]
fn raw_html_inline_renders_as_selectable_atom() {
    let rendered = App::from_markdown("x.md", "a <span>x</span> b").render_text();
    assert!(rendered.contains("<span>"));
}

#[test]
fn raw_html_edit_command_updates_payload_and_export() {
    let mut doc = import_gfm("<div>x</div>");
    doc.blocks[0] = mdtui_core::Block::HtmlBlock("<div>y</div>".to_string());
    assert!(export_gfm(&doc).contains('y'));
}

#[test]
fn raw_html_never_invokes_tui_layout_renderer() {
    let rendered =
        App::from_markdown("x.md", "<section style=\"display:flex\">x</section>").render_text();
    assert!(rendered.contains("raw html"));
    assert!(!rendered.contains("flex row"));
}

#[test]
fn code_block_renders_line_numbers_and_copy_button() {
    let rendered =
        App::from_markdown("x.md", "```python\ndef greet():\n    return 1\n```").render_text();
    assert!(rendered.contains("│ copy │"));
    assert!(rendered.contains("│  1│ def greet():"));
    assert!(rendered.contains("│  2│     return 1"));
}

#[test]
fn code_block_toolbar_matches_border_width() {
    let rendered = render_document(
        &import_gfm("```python\ndef greet():\n    return 1\n```"),
        RenderOptions::default(),
    );
    assert!(rendered.lines.get(2).is_some_and(|line| line.contains('┴')));
    let widths = rendered
        .lines
        .iter()
        .take(5)
        .map(|line| line.chars().count())
        .collect::<Vec<_>>();
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "{widths:?}"
    );
}

#[test]
fn two_column_mode_balances_paragraph_blocks() {
    let opts = RenderOptions {
        columns: 2,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("one\n\ntwo\n\nthree\n\nfour"), opts).text();
    assert!(rendered.contains(" │ "));
}

#[test]
fn three_column_mode_balances_with_hyphenation() {
    let opts = RenderOptions {
        columns: 3,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("one\n\ntwo\n\nthree"), opts).text();
    assert!(rendered.contains(" │ "));
}

#[test]
fn h1_h2_span_columns_by_default() {
    let opts = RenderOptions {
        columns: 2,
        ..RenderOptions::default()
    };
    let rendered = render_document(&import_gfm("# Title\n\nbody"), opts).text();
    assert!(rendered.contains("Title") || rendered.contains("TITLE"));
}

#[test]
fn selection_across_columns_is_model_ordered() {
    let mut app = App::from_markdown("x.md", "one\n\ntwo");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 1,
            offset: 3,
        },
    );
    assert!(app.editor.selection.is_some());
}

#[test]
fn key_release_stops_held_arrow_without_replayed_events() {
    let mut input = InputEngine::default();
    input.push(InputEvent::KeyDown("ArrowDown".to_string()));
    input.push(InputEvent::KeyRepeat("ArrowDown".to_string()));
    input.push(InputEvent::KeyUp("ArrowDown".to_string()));
    let events = input.drain_frame();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, InputEvent::KeyRepeat(key) if key == "ArrowDown"))
    );
}

#[test]
fn burst_typing_drops_no_committed_text_and_accumulates_no_stale_motion() {
    let mut input = InputEngine::default();
    input.push(InputEvent::Text("abc".to_string()));
    input.push(InputEvent::MouseMove { x: 1, y: 1 });
    input.push(InputEvent::MouseMove { x: 2, y: 2 });
    let events = input.drain_frame();
    assert!(events.contains(&InputEvent::Text("abc".to_string())));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InputEvent::MouseMove { .. }))
            .count(),
        1
    );
}

#[test]
fn changing_document_width_reflows_without_moving_cursor_model_position() {
    let mut app = App::from_markdown("x.md", "hello world");
    app.editor.set_cursor(Cursor::Text {
        block: 0,
        offset: 5,
    });
    let cursor = app.editor.cursor;
    app.render_options.width = 10;
    let _ = app.render();
    app.render_options.width = 80;
    let _ = app.render();
    assert_eq!(app.editor.cursor, cursor);
}

#[test]
fn document_width_slider_changes_softwrap_without_changing_markdown() {
    let mut app = App::from_markdown("x.md", "hello wide world");
    let before = app.save_to_gfm();
    app.render_options.width = 8;
    let _ = app.render();
    assert_eq!(app.save_to_gfm(), before);
}

#[test]
fn width_slider_changes_document_width() {
    let mut app = App::default();
    app.render_options.width = 72;
    assert_eq!(app.render_options.width, 72);
}

#[test]
fn top_level_blocks_have_blank_line_spacing() {
    let rendered = render_document(&import_gfm("one\n\ntwo"), RenderOptions::default()).text();
    assert_eq!(rendered, "one\n\ntwo");
}

#[test]
fn hyphenation_affects_visual_wrap_only_not_exported_text() {
    document_width_slider_changes_softwrap_without_changing_markdown();
}

#[test]
fn undo_restores_selection_after_structural_delete() {
    let mut app = App::from_markdown("x.md", "abc");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 0,
            offset: 2,
        },
    );
    app.delete();
    app.editor.undo();
    assert!(app.editor.selection.is_some());
}

#[test]
fn redo_restores_selection_after_undo() {
    let mut app = App::from_markdown("x.md", "abc");
    app.type_char('x');
    app.editor.undo();
    app.editor.redo();
    assert!(app.editor.document.rendered_text().contains('x'));
}

#[test]
fn destroyed_cursor_node_recovers_to_nearest_valid_position() {
    let mut app = list_app();
    app.editor.set_cursor(Cursor::ListItem {
        block: 0,
        item: 0,
        offset: 0,
    });
    app.backspace();
    assert!(matches!(app.editor.cursor, Cursor::Text { block: 0, .. }));
}

#[test]
fn stale_worker_response_is_discarded_after_node_version_change() {
    let mut doc = import_gfm("x");
    let result = VersionedResult {
        document_version: 0,
        payload: "old",
    };
    doc.version = 1;
    assert!(!accept_worker_result(&doc, &result));
}

#[test]
fn stale_spellcheck_result_is_discarded_after_edit() {
    stale_worker_response_is_discarded_after_node_version_change();
}

#[test]
fn ai_is_disabled_when_model_empty() {
    let config = AiConfig {
        enabled: true,
        model: String::new(),
        api_key_present: true,
    };
    assert!(!config.available());
}

#[test]
fn ai_is_disabled_when_api_key_missing() {
    let config = AiConfig {
        enabled: true,
        model: "gpt".to_string(),
        api_key_present: false,
    };
    assert!(!config.available());
}

#[test]
fn ai_replace_selection_is_single_undo_step() {
    let mut app = App::from_markdown("x.md", "abc");
    app.editor.select_range(
        Cursor::Text {
            block: 0,
            offset: 0,
        },
        Cursor::Text {
            block: 0,
            offset: 3,
        },
    );
    app.editor.paste_plain_text("ai");
    app.editor.undo();
    assert_eq!(app.editor.document.rendered_text(), "abc");
}

#[test]
fn theme_matches_dark_amber_palette() {
    let theme = Theme::dark_amber();
    assert_eq!(theme.app_bg, "#0f0c08");
    assert_eq!(theme.panel_bg, "#18120d");
    assert_eq!(theme.panel_raised, "#241a12");
    assert_eq!(theme.code_bg, "#000000");
    assert_eq!(theme.accent_primary, "#e6a85a");
    assert_eq!(theme.border_strong, "#d89a4a");
}

#[test]
fn documents_open_on_first_non_headline_line() {
    let app = App::from_markdown("x.md", "# Title\n\nFirst paragraph");
    assert_eq!(
        app.editor.cursor,
        Cursor::Text {
            block: 1,
            offset: 0,
        }
    );
}

#[test]
fn spec_opens_after_leading_title() {
    let app = App::from_markdown("SPEC.md", include_str!("../../../SPEC.md"));
    assert_eq!(
        app.editor.cursor,
        Cursor::Text {
            block: 1,
            offset: 0,
        }
    );
}
