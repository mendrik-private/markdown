# Markdown TUI Editor Memory

## Product invariant
This is a rendered-only structured editor. Markdown is import/export only.
Users must never need to edit Markdown source markers.

## No modal editing
There is no Vim-style Normal/Insert mode. The app opens in direct editing mode.
Printable keys insert text unless a popover captures focus.

## Parser decision
Use comrak as the initial GFM parser/serializer adapter. Target the official
GitHub Flavored Markdown specification at https://github.github.com/gfm/.

Current scaffold keeps the parser behind `mdtui-markdown`; the adapter is
line-based until comrak AST integration is wired behind the same crate API.

## Model decision
The structured document tree is authoritative. NodeIds are stable. Markdown
source is not the editable backing store. Text positions are grapheme-based,
not byte offsets.

## Rendering decision
The display list maps rendered terminal cells back to document positions.
Cursor movement, hit testing, selection, and mouse behavior all use this
display list.

## Transaction decision
Every edit creates a transaction with a position mapping. Cursor and selection
are anchors with bias and stickiness. Undo/redo restores both document state
and selection.

## Syntax-leak decision
Backspace, Delete, and selection deletion must cleanly remove or merge
structures. Hidden Markdown markers must never appear as leftover text.

## Table decision
Tables render as Unicode TUI grids and are edited structurally. Pipe tables are
export syntax only. Empty cells/rows/columns survive editing and save/load.

## List decision
Lists are real nodes. List bullets, ordered numbers, and checkboxes are visual
atoms. Enter creates list items; Backspace merges/unwraps structurally.

## Selection decision
Shift+Arrow and mouse drag selection are required. Styling popover appears on
non-empty selection.

## Kitty decision
Kitty Graphics Protocol is used for images and H1/H2 headline image placements.
Do not use Kitty Text Sizing / OSC-66 for headings. Clean fallback is mandatory.

## HTML decision
Preserve raw HTML as atomic inline/block nodes with explicit raw-edit commands.
Do not implement an HTML renderer, safe-subset layout, flex rows, floats, or TUI boxes.

## Column decision
Column rendering is visual-only. Prose may flow into 1/2/3 balanced columns,
while headings, tables, code blocks, images, and raw HTML atoms span by default.

## Performance decision
The app must sustain 60 fps during interaction. Typing and cursor movement must
stop immediately on key release; stale repeated input or motion events must be
dropped rather than replayed.

## Worker decision
The UI thread owns the document. Workers process immutable snapshots and return
versioned results. Stale results are discarded.

## Config decision
AI has no default model. Empty model or missing API key disables AI.

## Testing decision
No feature is complete without model tests, render snapshot tests, and
interaction tests. The acceptance harness in SPEC.md is mandatory; tests are
not removed to make the build pass.
