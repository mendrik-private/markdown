# mdtui

`mdtui` is a Rust/Ratatui prototype for a projectional Markdown editor: the main surface renders Markdown as document components while edits are applied back to Markdown source.

Run it with:

```sh
cargo run -p mdtui-tui -- example.md
```

Key bindings in the current MVP:

- `Ctrl-S` saves the active document.
- `Ctrl-Q` quits, with confirmation when unsaved tabs would be discarded.
- `?` opens help.
- `:` or `Ctrl+P` opens the fuzzy command palette; typed commands like `:open path` still work.
- `/` or `Ctrl+F` opens search; Enter jumps to the next match and Shift+Enter jumps to the previous match.
- Arrow keys move visually, `Ctrl+Left/Right` move by word, and `Alt+Arrow` moves structurally between components.
- `Shift+Arrow` and mouse drag select source-backed rendered text, `Ctrl+A` selects the document, and typing or inline style actions apply to the selection.
- Mouse click in the rendered document moves the semantic cursor; clicking a task checkbox toggles it, and clicking or dragging the scrollbar jumps scroll position.
- On a focused gap, `Alt+Enter` opens the insert menu for paragraphs, headings, code blocks, quotes, alerts, lists, tables, images, link references, math, and diagrams.
- In tables, `Alt+Arrow` inserts rows/columns and `F2` opens row/column selection, removal, and normalization actions.
- In tables, `Delete` removes a selected row or column.
- In tables, `Tab`/`Shift+Tab` move between cells, `Enter` moves down and adds a row at the end, and Backspace removes an empty row.
- In tables, `Ctrl+Alt+Left/Right` changes the focused column alignment.
- On headings, `F2` edits `level|text` and `Ctrl+Alt+1..6` changes the heading level.
- On task list items, Space toggles the checkbox.
- `Ctrl+K` turns the current word into a link; `F2` on a link edits `label|url|title|ref`.
- A text selection shows the mouse-clickable inline style palette; `Ctrl+E` opens it manually for bold, italic, strikethrough, inline code, and link actions.
- `Ctrl+L` edits a fenced code block language.
- `Ctrl+B`, `Ctrl+I`, and `Ctrl+`` toggle bold, italic, and inline code on the current word.
- On footnote refs, `Enter` jumps to the definition, `Alt+Enter` creates a missing definition, and `F2` renames the focused label.
- On image cards, `F2` edits `alt|source|title`; missing local image paths and missing link reference definitions are reported as diagnostics.
- In fenced code blocks, `Tab` cycles language, body, and Copy focus targets.
- `Ctrl+Shift+C` copies the focused code block body to the native clipboard, with OSC 52 fallback.
- `Ctrl+Tab` switches to the next tab and `Ctrl+Shift+Tab` switches to the previous tab.
- Closing a dirty tab asks for confirmation before discarding changes.
- Typing, Enter, Backspace, Delete, paste, undo, and redo edit the source-backed rope.

Rendered fenced code blocks use `syntect` syntax highlighting with a bounded line-state cache.
Lists render task items as visual checkbox atoms, blockquotes render as framed quote surfaces, GFM strikethrough renders as crossed-out text, emoji shortcodes render as Unicode emoji where resolvable, and tables use accent-styled headers.
The Markdown CST uses tree-sitter-md with cached trees and `InputEdit` reuse after source edits.
The terminal layer includes Kitty Graphics Protocol capability detection, upload/delete command builders, and image-id cache keys.
H1/H2 headings emit capability-gated KGP raster uploads and Unicode placeholder cells while keeping styled text fallback.
The terminal compatibility matrix covers Ghostty, Kitty, and WezTerm graphics paths plus iTerm2/basic ANSI styled-text fallback.
Local image cards decode supported formats, scale to the preview rectangle, and emit capability-gated KGP RGBA uploads with placeholder cells; remote and unsupported images stay as text cards.
Math and diagram blocks render safe text fallbacks first, queue preview work outside the render callback, then use cached KGP placeholder previews once warm.
Optional external preview renderers can provide real math/diagram rasters: set `MDTUI_MATH_RENDERER`, `MDTUI_DIAGRAM_RENDERER`, `MDTUI_PREVIEW_RENDERER`, or a label-specific `MDTUI_PREVIEW_<LABEL>_RENDERER`. Commands are executed directly, not through a shell; the Markdown source is piped on stdin, `{label}`, `{width}`, and `{height}` are expanded in args, theme colors are exposed through `MDTUI_PREVIEW_THEME_*` env vars, and the command should write a supported image format to stdout.
The TUI requests a bounded render window for visible document rows plus overscan; the document layout cache persists the block row index, the renderer jumps to the first visible component, and large fenced code blocks, lists, tables, and boxed blocks virtualize internal rows while preserving total document row counts.
The test suite includes deterministic large-document render-window regressions, inline edit fuzz coverage, and an ignored render benchmark matrix for large paragraphs, code, lists, tables, boxed math, and cached scrolls.

The terminal lifecycle sets the full terminal background with `OSC 11` on entry and resets it with `OSC 111` on exit.
