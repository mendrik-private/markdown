# mdtui — TUI GFM Markdown Editor

**A Rust + Ratatui rendered-only editor for GitHub Flavored Markdown.**

> The editor is always a *rendered* editor. Markdown is only an import/export format. Users must never be forced to edit Markdown source syntax such as `###`, `-`, `[x]`, pipe table delimiters, emphasis markers, or link syntax.

Thissss document is the single source of truth. It supersedes earlier plans and combines the product thesis, visual design, technical architecture, implementation order, and acceptance harness into one specification. The harness at the end defines completion.

---

## Table of contents

1. [Project identity](#1-project-identity)
2. [Non-negotiable product rules](#2-non-negotiable-product-rules)
3. [Design thesis](#3-design-thesis)
4. [Standards and external references](#4-standards-and-external-references)
5. [Visual design](#5-visual-design)
6. [Product shape and layout](#6-product-shape-and-layout)
7. [Input model](#7-input-model)
8. [Document model](#8-document-model)
9. [Transactions and mappings](#9-transactions-and-mappings)
10. [Layout engine and display list](#10-layout-engine-and-display-list)
11. [GFM import and export](#11-gfm-import-and-export)
12. [Editing semantics](#12-editing-semantics)
13. [Selection and styling popover](#13-selection-and-styling-popover)
14. [Rendered widgets](#14-rendered-widgets)
15. [Tables](#15-tables)
16. [Lists and task lists](#16-lists-and-task-lists)
17. [Headings and Kitty Graphics](#17-headings-and-kitty-graphics)
18. [Images and Kitty Graphics](#18-images-and-kitty-graphics)
19. [Raw HTML preservation](#19-raw-html-preservation)
20. [Code blocks and syntax highlighting](#20-code-blocks-and-syntax-highlighting)
21. [Spellcheck, languages, hyphenation](#21-spellcheck-languages-hyphenation)
22. [AI support](#22-ai-support)
23. [Document width and softwrap](#23-document-width-and-softwrap)
24. [Side panels, status bar, workspace](#24-side-panels-status-bar-workspace)
25. [File watching, sessions, conflicts](#25-file-watching-sessions-conflicts)
26. [Async architecture](#26-async-architecture)
27. [Undo and redo](#27-undo-and-redo)
28. [Find and replace](#28-find-and-replace)
29. [Configuration schema](#29-configuration-schema)
30. [Implementation order](#30-implementation-order)
31. [Test harness](#31-test-harness)
32. [Acceptance tests](#32-acceptance-tests)
33. [Visual snapshots](#33-visual-snapshots)
34. [Forbidden implementations](#34-forbidden-implementations)
35. [Memory file requirement](#35-memory-file-requirement)
36. [Definition of done](#36-definition-of-done)
37. [Agent self-check](#37-agent-self-check)
38. [Source notes](#38-source-notes)

---

## 1. Project identity

`mdtui` is a Rust + Ratatui terminal UI editor for GitHub Flavored Markdown. It is a **rendered-only structured document editor**. Markdown is only the load/save interchange format. Users edit the rendered document directly and must not see Markdown syntax artifacts during normal editing.

This is **not**:

- a Vim clone,
- a source Markdown editor,
- a split preview/source editor.

The previous build failed because it violated the product thesis. This specification exists to prevent that from happening again.

---

## 2. Non-negotiable product rules

These rules are the contract. A feature that violates any of them is broken regardless of how it tests in isolation.

1. **No modal insert mode.**

The app opens directly in editing mode. Typing printable characters inserts text immediately. Pressing `i` inserts the literal character `i` unless a popover or palette has captured focus. There is no Normal/Insert/Visual mode and no mode indicator in the status bar.

1. **No visible Markdown syntax during normal editing.**

`#`, `###`, `-`, `*`, `**`, `_`, `` ` ``, `[ ]`, `[x]`, table delimiter rows, and pipe-table syntax must not appear as structural markers. They may appear only when literally part of document text. Markdown syntax is import/export only.

1. **The rendered document is the editor.**

There is no separate preview. Cursor, selection, mouse hit-testing, typing, deletion, paste, and formatting all operate on the rendered document model.

1. **Tables are first-class widgets.**

Tables render with Unicode box drawing, never pipe ASCII. Cells are editable. Empty rows, empty columns, and empty cells remain visible and editable. `Ctrl+Arrow` table commands add rows and columns.

1. **Lists are first-class widgets.**

List markers are visual atoms, not text. `Enter` inside a list item creates a new list item. `Backspace` at the start of a list item merges or unwraps structurally without exposing `-` or numbering.

1. **Selection works.**

`Shift+Arrow` selection is required. Mouse drag selection is required. `Ctrl+A` select-all is required. Selection works across inline marks, wrapped lines, blocks, list items, table cells, images, and raw-HTML atoms.

1. **The styling popover works.**

Selecting text shows a small floating styling bar supporting bold, italic, strikethrough, inline code, link, heading level, list conversion, blockquote, code block, AI action (when configured), and clear style. Both keyboard and mouse can drive it. Do render a shadow but making the chars column to the right and one below the popup a bit darker.

1. **Headline renderer.**

H1 and H2 headlines render as two-row visual headlines using the Kitty Graphics Protocol, not Kitty Text Sizing / OSC-66. The heading text is rasterized to an embedded image and placed in the document display list. When the cursor enters any cell covered by the headline, the heading switches to regular editable rendered text with no `#` marker. When the cursor leaves the heading, the cached headline image is restored immediately without flicker.

1. **GFM is primary.**

The editor targets the official GitHub Flavored Markdown specification at <https://github.github.com/gfm/>. Raw HTML is preserved on import/export but is not rendered as a browser-like or TUI layout engine.

1. **The harness is authoritative.**

A feature without tests is not done. If any test would allow modal editing, pipe-source tables, list-spacing regressions, marker leakage, or non-working selection, the harness is wrong and must be tightened before coding continues.

1. **The app must look intentional.**

The `look-and-feel` reference is a polished Ghostty-amber terminal aesthetic. Default-terminal ugliness is a regression. No noisy ASCII fallback unless the terminal genuinely requires it.

---

## 3. Design thesis

The editor is a structured rich-text editor whose persistence format is GFM.

### Core invariant

```text
Visible document = rendered semantic model
Saved file       = deterministic GFM serialization of that model
Parser           = import adapter
Serializer       = export adapter
```

Markdown markers are not editable characters. They are serializer output produced from structural nodes and marks.

### Consequences

For a list item written as:

```markdown
- hello
```

the rendered editor shows:

```text
• hello
```

The bullet is a list adornment, not text. The editable text is only `hello`. Backspace at the start of `hello` performs a structural operation and never walks backward onto `- `.

For a task item:

```markdown
- [x] done
```

the rendered editor shows:

```text
☑ done
```

The checkbox is a toggle atom. It is clickable and space-toggleable. `[x]` is never visible as source.

For a table:

```markdown
| Name | Done |
| ---- | ---- |
| API  | yes  |
```

the rendered editor shows a real grid:

```text
┌──────┬──────┐
│ Name │ Done │
├──────┼──────┤
│ API  │ yes  │
└──────┴──────┘
```

Pipes and the delimiter row are serialization only.

---

## 4. Standards and external references

The implementation primarily targets the official GitHub Flavored Markdown specification at <https://github.github.com/gfm/>. Treat that document as the source of truth for Markdown syntax and semantics.

The following are architectural references, not dependencies:

- **ProseMirror** — transactions, steps, position mapping, schemas, WYSIWYG editing discipline.
- **Lexical** — editor state as source of truth; selection as part of state; update/reconciliation batching.
- **Xi editor** — rope-backed text storage, efficient edits, stable text operations.
- **Unicode UAX #29** — grapheme-cluster cursor movement and deletion.
- **Tree-sitter** — incremental parsing for code-block highlighting.
- **Kitty Graphics Protocol** — images and rasterized headline typography. Do not use Kitty Text Sizing / OSC-66 for headings.
- **Ratatui** — terminal rendering surface.

Concrete pinned choices:

```toml
[markdown]
parser       = "comrak"
gfm_spec_url = "https://github.github.com/gfm/"
gfm_profile  = "0.29-gfm"
```

`comrak` is the initial parser/serializer adapter. It is a Rust CommonMark/GFM-compatible library with a stable Rust API. The exact crate version is pinned in `Cargo.lock`, not here. Parser choice is isolated behind the `mdtui-markdown` crate so it can be replaced without disturbing the model.

---

## 5. Visual design

The aesthetic is a state-of-the-art terminal UI with a warm Ghostty-amber feel: dark espresso background, amber/honey/muted-gold accents, thin glowing borders, soft terminal-style shadows, crisp monospace typography. Quiet, dense, elegant — never flashy.

### Palette: `dark_amber`

```toml
[theme.dark_amber]
# Surfaces
app_bg            = "#0f0c08"   # espresso/charcoal app background
panel_bg          = "#17120c"   # side panels, popovers
panel_raised      = "#21180f"   # raised surfaces, code block bg
active_row        = "#5a3518"   # active tab/row fill
hover_row         = "#342414"   # hover row

# Borders and glow
border            = "#4a3420"   # default border
border_strong     = "#d89a4a"   # focus/glow border

# Accent
accent_primary    = "#e6a85a"   # amber primary
accent_highlight  = "#f1b96d"   # bright amber highlight

# Text
text_primary      = "#ead8bd"   # body text (warm off-white)
text_secondary    = "#b99f7a"   # captions, secondary
text_muted        = "#7d6a50"   # disabled, hints

# Semantic
success           = "#9fca55"   # green
warning           = "#e0b64f"   # warning gold
error             = "#d66a45"   # red-orange
link              = "#7da6c8"   # muted blue

# Selection
selection_bg      = "#5a3518"   # uses active_row
selection_fg      = "#ead8bd"   # uses text_primary

# Specific roles
code_bg           = "#21180f"   # uses panel_raised
table_border      = "#4a3420"   # uses border
heading_underline = "#d89a4a"   # uses border_strong
```

### Style rules

- **Borders:** thin, single-line by default; rounded corners reserved for popovers/cards. The active panel/popover gets an amber border in `border_strong`. Tables use `table_border` (dim amber), never ASCII pipes.
- **Headings:** color, weight, and underline; oversized via Kitty Graphics.
- **Code blocks:** `code_bg` background, language tag at the corner, no Markdown backticks. Syntax highlight for common languages
- **Links:** `link` color with underline.
- **Inline code:** `code_bg` background, no backticks visible.
- **Selection:** `selection_bg` fill, anchor stable across scroll.
- **Status bar:** dense but readable; one line; muted text with amber for active values.
- **Shadows:** subtle dark shadow under floating popovers; one or two cells of soft fade.
- **No browser-like chrome.** Pure TUI.

### Mood

Warm, focused, technical, premium. Like a modern Ghostty terminal with a brown/amber theme. Suitable for developers and writers working in a terminal-native documentation editor.

---

## 6. Product shape and layout

The app is a fullscreen TUI. Suggested 16:9 layout:

```text
╭──────────────┬──────────────────────────────────────────────────────┬──────────╮
│ EXPLORER     │ ┌── README.md ●  roadmap.md   prosa-spec.md   ────┐  │ OUTLINE  │  ← tab strip
│ ⌄ project    │ │                                                 │  │ ⌄ Intro  │
│   - README   │ │   Prosa: A Productive Markdown Dialect          │  │  ⌄Tasks  │
│   - roadmap  │ │   ═══════════════════════════════════════════   │  │ ⌄ Tables │
│   - docs     │ │                                                 │  │ ⌄ Code   │
│ ⌄ config     │ │   Prose wraps softly at the document width.     │  │          │
│   * Cargo    │ │                                                 │  │          │
│              │ │   • Task list                                   │  │          │
│ OUTLINE      │ │   • Real tables                                 │  │          │
│ (mirrors →)  │ │                                                 │  │          │
├──────────────┴──────────────────────────────────────────────────────┴──────────┤
│ main ↑2  │  H2  row 42 col 7  │  words 1,204  │  en-US  │  width 80 ◀━━●━━▶    │  ← status bar
╰────────────────────────────────────────────────────────────────────────────────╯
```

### Panels

- **Left sidebar (18–22% width).** Two stacked sections: `EXPLORER` (file/workspace tree with markdown/config/folder icons, active row in `active_row` with `accent_highlight` text) and `OUTLINE` (nested heading tree with disclosure arrows; muted text for inactive, amber for active section).
- **Tab strip.** Horizontal across the top of the main area. Active tab uses raised amber background. Inactive tabs use `panel_bg` with muted text. File icons; small circular dirty-state indicators after the filename.
- **Document panel.** Central rendered editor, always editable. Centered content at document width when `center_document = true`.
- **Right mini rail (optional).** Section/index jump list or scroll markers. Collapsible.
- **Status bar.** One line, dense. Shows: file name, dirty state, current logical row/column, current block type, word count, selection length, document width slider, column mode, spell language, AI state, terminal capability indicators, table cell coordinate when inside a table, link target when inside a link.

### Content variant: rendered document canvas

The document panel is **not** a Markdown source view. It renders:

- Large H1 title rendered via Kitty Graphics when available; amber gradient/shadow treatment; clean styled-text fallback.
- Body text in `text_primary` (warm off-white).
- Numbered hierarchy in `accent_primary` for section headings.
- Cards, tables, blockquotes (with oversized quote glyph), task lists, diagrams, code blocks.
- Inline emoji/icons.
- Graphical checkboxes — checked in `success` with crisp tick; empty in `accent_primary` outline.
- Blockquote with oversized opening quote.
- Code blocks with line numbers and a language label badge.
- A small diagram/workflow row using native TUI boxed nodes and arrows.
- Column rendering mode is a first-class display setting: 1, 2, or 3 balanced prose columns using hyphenation-aware text flow. See §23.

### Inline style popup (floating)

```text
╭─ Inline Style ──────────────────────────────╮
│  B   I   S   `   L   x²   x₂  [more]        │
│  ●   ●   ●   ●   ●   ●    ●                 │  ← swatches: amber,
╰─────────────────────────────────────────────╯       orange, olive,
                                                       steel blue, gray,
                                                       red, rose
```

Rounded bordered panel with warm shadow. Toolbar row of icon buttons; swatches row beneath; label `Inline Style` at the bottom. Terminal-native but polished — it must not look like a default-styled popover.

---

## 7. Input model

### One primary editing mode

```rust
enum UiFocus {
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
```

Forbidden:

```rust
// Never
enum Mode { Normal, Insert, Visual }
```

The document focus always accepts text input. Popovers and palettes temporarily capture input while open; closing them returns to direct editing. There is no Vim-style mode and no mode status anywhere in the UI.

### Keyboard expectations

| Key | Required behavior |
| --- | --- |
| Printable char | Insert at selection/cursor |
| i | Insert literal i (no special mode meaning) |
| Enter | Split block; create list item; create table row/cell paragraph depending on context |
| Backspace | Delete selection or previous grapheme/structural boundary |
| Delete | Delete selection or next grapheme/structural boundary |
| Arrow keys | Move cursor by grapheme / line / block as appropriate |
| Ctrl+Arrow | Word movement; in tables, add row/column in arrow direction |
| Shift+Arrow | Extend selection |
| Ctrl+B | Toggle bold |
| Ctrl+I | Toggle italic (alternate binding when terminal reserves this for Tab) |
| Ctrl+Shift+X | Strikethrough |
| Ctrl+E | Inline code |
| Ctrl+K | Link popover |
| Ctrl+F | Find rendered text |
| Ctrl+H | Replace rendered text |
| Ctrl+S | Save |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z / Ctrl+Y | Redo |
| Ctrl+P | Command palette |
| Alt+[ / Alt+] | Decrease/increase document width |
| Ctrl+Alt+Left/Right | Decrease/increase document width |
| Space on checkbox atom | Toggle checkbox |
| Click on link | Place cursor inside link text |
| Ctrl+Click / Cmd+Click | Open link externally |
| Tab in table | Next cell (creates new row at the last cell) |
| Shift+Tab in table | Previous cell |
| Tab in list | Indent list item |
| Ctrl+Enter in table | Insert row after current |
| Ctrl+Shift+Enter in table | Insert row before current |
| Alt+Arrow in table | Move cell focus without editing text |

---

## 8. Document model

### Source of truth

```rust
struct Document {
    id:          DocumentId,
    blocks:      Vec<BlockId>,
    nodes:       NodeArena,
    references:  ReferenceRegistry,
    frontmatter: Option<Frontmatter>,
    metadata:    DocumentMetadata,
    version:     u64,
}
```

`Document` is the only source of truth while editing. GFM source is **not** stored as the editable backing store.

### Stable node identity

```rust
struct NodeMeta {
    id:          NodeId,
    generation:  u64,
    parent:      Option<NodeId>,
    version:     u64,
    source_span: Option<SourceSpan>,
}
```

Rules:

- Text edits preserve node IDs.
- Paragraph ↔ heading conversions may preserve node ID if only block kind changes.
- Structural replacement may destroy node IDs, but transactions must record position recovery.
- Layout, diagnostics, spellcheck, AI results, and syntax highlights must include node version and be discarded when stale.

### Required block nodes

`Document`, `Paragraph`, `Heading`, `BlockQuote`, `BulletList`, `OrderedList`, `TaskList`, `ListItem`, `CodeBlock`, `Table`, `TableRow`, `TableCell`, `ThematicBreak`, `ImageBlock`, `HtmlBlock` (raw atomic), `Frontmatter`.

```rust
enum Block {
    Paragraph(Paragraph),
    Heading(Heading),
    BlockQuote(BlockQuote),
    List(List),
    CodeBlock(CodeBlock),
    Table(Table),
    ThematicBreak(ThematicBreak),
    ImageBlock(ImageBlock),
    HtmlBlock(HtmlBlock),
    Frontmatter(Frontmatter),
}
```

### Inline content

```rust
enum InlineNode {
    Text(TextNode),
    SoftBreak,
    HardBreak,
    Emphasis  { children: Vec<InlineId> },
    Strong    { children: Vec<InlineId> },
    Strike    { children: Vec<InlineId> },
    InlineCode { text: Rope },
    Link  { target: LinkTarget, title: Option<String>, children: Vec<InlineId> },
    Image { src: String, alt: Vec<InlineId>, title: Option<String> },
    HtmlInline(HtmlInline),
}
```

### Text storage and graphemes

Use a rope for editable text payloads.

```rust
struct TextNode {
    text:    Rope,
    metrics: TextMetrics,
}

struct GraphemeIndex(usize);
```

Cursor positions are **grapheme-based**, not byte-based or char-based.

### Positions, anchors, selections

```rust
enum Position {
    Text                { node: NodeId, grapheme: GraphemeIndex },
    InlineAtomBefore    { node: NodeId },
    InlineAtomAfter     { node: NodeId },
    BlockBoundaryBefore { node: NodeId },
    BlockBoundaryAfter  { node: NodeId },
    TableCell           { table: NodeId, row: usize, col: usize, inner: Box<Position> },
}

struct Anchor {
    pos:        Position,
    bias:       Bias,
    stickiness: Stickiness,
}

struct Selection {
    anchor: Anchor,
    head:   Anchor,
}

enum SelectionKind {
    Caret,
    TextRange,
    BlockRange,
    TableCellRange,
    NodeSelection,
}
```

Positions must support text grapheme positions, block boundaries, atom boundaries (checkboxes, images, link atoms), and table cell positions. No screen-coordinate cursor state is authoritative — screen cursors are projections from model positions.

**Document model rule:** *Markdown source markers are never represented as editable text.*

---

## 9. Transactions and mappings

Every edit is a transaction. Direct mutation from event handlers is forbidden.

```rust
struct Transaction {
    id:                TxId,
    intent:            EditIntent,
    ops:               Vec<Op>,
    mapping:           PositionMapping,
    before_selection:  Selection,
    after_selection:   Selection,
    dirty_nodes:       DirtySet,
    timestamp:         Instant,
}
```

### Operation families

```rust
enum Op {
    InsertText        { node: NodeId, at: GraphemeIndex, text: String },
    DeleteText        { node: NodeId, range: GraphemeRange },
    SplitTextNode     { node: NodeId, at: GraphemeIndex, right_id: NodeId },
    MergeTextNodes    { left: NodeId, right: NodeId },
    InsertBlock       { parent: NodeId, index: usize, block: Block },
    DeleteBlock       { node: NodeId },
    ReplaceBlockKind  { node: NodeId, from: BlockKind, to: BlockKind },
    WrapInline        { range: ModelRange, mark: InlineMark },
    UnwrapInline      { range: ModelRange, mark: InlineMark },
    SetLinkTarget     { node: NodeId, href: String, title: Option<String> },
    ToggleTask        { item: NodeId },
    TableInsertRow    { table: NodeId, index: usize },
    TableDeleteRow    { table: NodeId, index: usize },
    TableInsertColumn { table: NodeId, index: usize },
    TableDeleteColumn { table: NodeId, index: usize },
}
```

### Mapping

```rust
enum MapResult {
    Mapped(Position),
    Deleted   { nearest: Position },
    Ambiguous { candidates: Vec<Position>, preferred: Position },
}
```

Required behavior:

- Selection deletion maps the final cursor to the start of the deleted rendered range.
- Structural delete never leaves visible syntax markers.
- Cursor inside a deleted node moves to the nearest valid semantic position.
- Cursor inside a table cell stays in the corresponding cell when possible.
- Cursor in a removed row/column moves to the nearest surviving cell.
- Cursor before/after a raw HTML atom maps to the nearest valid block boundary when the atom is deleted.

---

## 10. Layout engine and display list

### Pipeline

```text
Document model
  → block layout tree
  → wrapped visual lines
  → display items
  → ratatui buffer + kitty side-channel commands
```

### Display items

```rust
enum DisplayItem {
    TextRun        { range: ModelRange, text: String, style: StyleId, rect: Rect },
    CursorTarget   { pos: Position, rect: Rect },
    SelectionRange { range: ModelRange, rects: Vec<Rect> },
    Adornment      { kind: AdornmentKind, owner: NodeId, rect: Rect },
    TableGrid          { table: NodeId, rect: Rect, cells: Vec<CellDisplay> },
    ImagePlacement     { node: NodeId, rect: Rect, image_key: ImageKey },
    HeadlinePlacement  { node: NodeId, rect: Rect, image_key: ImageKey, level: HeadingLevel },
    RawHtmlAtom        { node: NodeId, rect: Rect },
}
```

### Required mapping functions

```rust
fn hit_test(point: ScreenPoint, display: &DisplayList) -> HitResult;
fn position_to_cursor(pos: Position, display: &DisplayList) -> Option<ScreenPoint>;
fn range_to_rects(range: ModelRange, display: &DisplayList) -> Vec<Rect>;
```

These are correctness-critical. Do not stub them. The display list is the source of truth for: cursor movement, `Shift+Arrow` selection, mouse click placement, mouse drag selection, link activation, checkbox toggling, and styling-toolbar positioning.

### Reflow stability

- Reflow must not change model positions.
- Width changes invalidate layout but not document/selection state.
- Softwrap produces visual rows, not logical document rows.
- Status bar `row/col` means logical document row/column in the exported rendered text, not the terminal-wrapped row.

### Caching

```rust
struct LayoutCacheKey {
    node:                          NodeId,
    node_version:                  u64,
    available_width:               u16,
    theme_version:                 u64,
    terminal_capabilities_version: u64,
}
```

Invalidate on: edited node; parent block; parent list/table container where geometry may change; column-flow root when width or column mode changes; outline when headings change; headline raster key changes; spell/highlight diagnostics when node version changes.

### Performance targets

The app is a 60 fps interactive editor. Rendering may be event-driven, but when input, cursor motion, scrolling, image placement, or animations are active, frame pacing must meet a 16.6 ms budget. Performance is a product requirement, not a best-effort optimization.

```text
target refresh during interaction: 60 fps
frame budget: <= 16.6 ms from event receipt to terminal flush
p50 input-to-redraw latency: < 8 ms
p99 input-to-redraw latency: < 16 ms for ordinary docs
no redundant terminal flushes for unchanged frames
scrolling stays visually continuous on large documents
```

Input handling has a stricter rule: typing and cursor movement must stop immediately when the key is released. The event loop must not accumulate stale key-repeat, mouse-drag, scroll, or movement events and replay them after input stops.

Rules:

- Poll input every frame while interaction is active.
- Coalesce repeated movement events to the most recent state before layout.
- Drop stale key-repeat and motion events whose matching key/button is no longer down.
- Never process a backlog of old movement events after a key-up/release.
- Printable text insertion may coalesce into undo groups, but individual UI input events must still be reflected or discarded within the current frame budget.
- Long work, including parsing, spellcheck, highlighting, image decode, AI, and file watching, must not block the UI thread.
- If the renderer misses a frame, skip stale intermediate render states rather than replaying them.
- Performance tests must include held-key repeat, held-arrow movement, key release, and burst typing.

---

## 11. GFM import and export

### Required GFM coverage

Paragraphs; ATX headings; setext headings on import; thematic breaks; block quotes; bullet lists; ordered lists; tight and loose lists; task list items; fenced and indented code blocks; tables; emphasis, strong, strikethrough; inline code; links and reference-style links; images; autolinks; raw HTML blocks and inline HTML; disallowed-raw-HTML handling per parser capability.

### Serializer rules

Export deterministic GFM:

```toml
[markdown.export]
heading_style            = "atx"
list_marker              = "-"
ordered_marker           = "."
table_style              = "pipe"
line_width               = 80
preserve_reference_links = true
preserve_html            = true
```

Visual rendering may be rich. The saved file remains normal GFM.

### Round-trip rule

For every fixture:

```text
source.md → import → model → export → import → model
```

The semantic model must be equivalent after the second import. Exact byte-for-byte source preservation is **not** required, except for raw HTML, frontmatter, and code blocks where possible.

---

## 12. Editing semantics

Printable text inserts at the cursor immediately. Typing replaces a non-empty selection. Typing inside a styled range inherits active marks; typing immediately after bold text continues bold only if cursor stickiness says so, otherwise it inserts plain text.

### Backspace

| Context | Behavior |
| --- | --- |
| Non-empty selection | Delete rendered selection structurally |
| Middle of text | Delete previous grapheme |
| Start of paragraph | Merge with previous block if compatible |
| Start of heading | Convert to paragraph or merge per command intent |
| Start of first list item | Unwrap list item to paragraph |
| Start of later list item | Merge with previous item |
| Empty list item | Exit or remove list item |
| Start of table cell | Move to previous cell or merge cell content per command — never delete the table border |
| Start of code line | Delete previous grapheme/line break inside code block |
| Before/after raw HTML atom | Delete or select the atom by explicit command; do not enter a hidden HTML renderer |

### Delete

Mirrors Backspace forward.

### Enter

| Context | Behavior |
| --- | --- |
| Paragraph | Split paragraph |
| Heading | Split; text after cursor becomes paragraph (unless configured otherwise) |
| Empty heading | Convert to paragraph |
| List item with content | Create next item of same kind |
| Empty list item | Exit list or outdent |
| Task list item | Create next task item, unchecked |
| Block quote | Create new quoted paragraph; empty quoted line exits quote |
| Table cell | Insert newline inside cell when multi-line cells enabled, otherwise move/create per key |
| Code block | Insert newline inside code block |
| Raw HTML atom | Open raw edit command only when explicitly requested; Enter otherwise moves past atom |

### Paste

Pasting Markdown creates structured nodes. Pasting plain text inserts plain text. No operation may leak hidden Markdown markers.

---

## 13. Selection and styling popover

### Selection inputs

`Shift+Left/Right/Up/Down`, `Shift+Home/End`, mouse drag, double-click word, triple-click block, drag across blocks, drag across table cells.

### When the styling bar appears

- Selection is non-empty, **and**
- the document panel has focus, **and**
- no higher-priority popover is open.

### Required actions

Bold, Italic, Strikethrough, Inline code, Link, Heading level (H1–H3 quick), Bullet/Numbered/Task list conversion, Code block, Block quote, AI (disabled unless configured), Clear style.

### Suggested visual

```text
╭─ Style ─────────────────────────────╮
│  B   I   S   `code`   🔗   H1 H2 H3 │
│  •  list   1. list   ☑ task   “ ”   │
│  AI   Clear                          │
╰──────────────────────────────────────╯
```

The popover is themed (rounded border, warm shadow, amber accents) and renders above the selection without shifting document layout.

### Behavior

- Mouse click activates buttons.
- Arrow keys move within the bar when focused.
- `Esc` closes the bar, preserving selection.
- Keyboard shortcuts work without focusing the bar.
- Applying a mark does not collapse the selection unless configured.
- Toggling marks on a partially-formatted selection normalizes correctly.
- Marks operate **structurally**; they never insert `**`, `_`, `[…](…)`, or any source syntax.

### Link popover

```text
╭─ Link ───────────────────────────────╮
│ Text:   selected rendered text        │
│ URL:    https://example.com           │
│ Title:  optional                      │
│ [Apply] [Remove link] [Open] [Cancel] │
╰───────────────────────────────────────╯
```

Plain click on a link places the cursor. `Ctrl+Click` / `Cmd+Click` / popover `[Open]` opens externally.

---

## 14. Rendered widgets

This section sets the visual standard for every rendered block. Widgets must look intentional, never default.

- **Headings.** H1/H2 visually distinct, rendered via Kitty Graphics when supported (see §17). Fallback: styled two-line text or octant-block raster fallback. Never render `#`.
- **Lists.** Bullets `•` `◦` `▪` (theme-controlled). Ordered lists use computed numbers. Task lists use `☐` and `☑`. No extra blank line between tight items unless the model says loose.
- **Tables.** Unicode box drawing. See §15.
- **Links.** Show visible link text. Hide URL unless cursor is inside the link, link is selected, or link popover is open. `Ctrl+Enter` opens the link; `Ctrl+K` edits.
- **Images.** Kitty Graphics Protocol when available; otherwise a bordered placeholder with alt text and path. Images are selectable atoms. See §18.
- **Code blocks.** Boxed/shaded with `code_bg`. Syntax highlighted (see §20). Language badge in the corner. No spellcheck inside code.
- **Raw HTML blocks.** Preserved as atomic blocks with a compact placeholder and explicit raw-edit command. No HTML layout renderer. See §19.
- **Block quotes.** Oversized opening quote glyph; left bar in `accent_primary`.
- **Thematic breaks.** Soft amber rule, not a row of dashes.

---

## 15. Tables

Tables are first-class nodes.

```rust
struct Table {
    id:         NodeId,
    alignments: Vec<Alignment>,
    rows:       Vec<TableRow>,
}

struct TableCell {
    id:         NodeId,
    blocks:     Vec<BlockId>,
    width_hint: Option<u16>,
}
```

### Rendering

- Unicode box drawing only. Pipe ASCII is **not** acceptable for rendered tables.
- Header separator uses `├ ┼ ┤`.
- Cropped cells show ellipsis.
- Wrapped cells increase row height.
- Selected cells use `selection_bg`.
- Active cell has an amber outline or inverted border.

Example:

```text
┌──────────────┬─────────────┬──────────┐
│ Name         │ Status      │ Owner    │
├──────────────┼─────────────┼──────────┤
│ Parser       │ Done        │ Core     │
│ Renderer     │ In progress │ TUI      │
└──────────────┴─────────────┴──────────┘
```

### Oversized tables

```toml
[table]
cell_wrap            = true
max_cell_width       = 32
min_cell_width       = 4
horizontal_scroll    = true
sticky_header        = true
sticky_first_column  = false
oversize_indicator   = true
```

- The table has its own horizontal viewport when wider than the document panel.
- Status bar shows `table r3 c5 · 12×9 · xscroll 24`.
- Scroll wheel scrolls the document; `Shift+Wheel` scrolls the table horizontally.
- Cursor inside clipped text auto-scrolls the cell/table viewport to keep the caret visible.

### Editing

- Cell text is normal rich text unless the table is configured plain-cell.
- `Tab` and `Shift+Tab` move cells.
- `Ctrl+Left/Right` inserts a column before/after based on caret side or active edge.
- `Ctrl+Up/Down` inserts a row before/after.
- Commands exist for delete row/column, align column, split table, convert table to paragraphs.
- Empty cells/rows/columns must remain visible and editable; they survive save and reload.

### Export

GFM pipe tables. If a cell contains multiple paragraphs or unsupported block structure, fall back per config:

```toml
[table.export]
complex_cells = "flatten"   # flatten | error
```

---

## 16. Lists and task lists

### Tight/loose preservation

```rust
enum ListTightness { Tight, Loose }
```

The renderer must not add blank lines to tight lists.

### Markers as adornments

```rust
enum ListAdornment {
    Bullet  { glyph: char },
    Ordered { number: usize, delimiter: OrderedDelimiter },
    Task    { checked: bool },
}
```

Markers are hit-testable but **not editable text**. They are visual atoms.

### Task interactions

- Click checkbox toggles state.
- `Space` toggles when the checkbox atom has focus.
- The cursor can move before/after a checkbox atom.
- Deleting around a checkbox manipulates task-list structure, never literal `[ ]`/`[x]`.

### Enter and Backspace at boundaries

- `Enter` inside a list item with content creates a new item of the same kind.
- `Enter` on an empty item exits the list.
- `Backspace` at the start of a later item merges with the previous item.
- `Backspace` at the start of the first item unwraps to a paragraph.

No marker text (`-`, `1.`, `[ ]`) may ever leak into the editable text after these operations.

---

## 17. Headings and Kitty Graphics

| Level | Visual treatment |
| --- | --- |
| H1 | Two terminal rows tall via Kitty Graphics; amber raster fill/shadow; extra top spacing |
| H2 | Two terminal rows tall via Kitty Graphics; slightly smaller raster; amber underline or divider treatment |
| H3 | Bold/accent styled text |
| H4–H6 | Muted accent with small prefix/adornment |

H1 and H2 are not implemented with Kitty Text Sizing Protocol, OSC-66, font-size escapes, terminal zooming, or any source-level ASCII art. They are rendered as image placements created by the editor and sent through the Kitty Graphics Protocol.

### Headline rendering lifecycle

1. Layout reserves a two-row visual rectangle for each H1/H2.
2. The heading text is converted into a raster image using a bundled pixel font.
3. The raster is uploaded or reused through the Kitty Graphics Protocol.
4. The display list emits a `HeadlinePlacement` item whose rectangle maps back to the heading node.
5. If the caret, mouse, or selection enters any cell covered by the placement, the placement is hidden and the heading is rendered as normal editable styled text.
6. When the caret leaves the heading, the cached placement is restored.

The editable version is still rendered text, not Markdown source. It must not expose `#`, setext underline markers, or any hidden Markdown syntax.

### Raster style

The renderer owns a small pixel-font atlas for ASCII letters, digits, punctuation, and a fallback replacement glyph. Unicode text is supported by best-effort fallback: use known glyphs where available, otherwise fall back to styled editable text rather than rendering broken boxes.

- H1 glyphs use a blocky pixel alphabet sized for a two-terminal-row headline. The logical design target is a 16×4 subpixel glyph per source character across the two rows.
- H2 glyphs use a narrower 12×4 subpixel glyph. The upper quadrants remain blank where needed to make H2 visibly lighter than H1.
- The fallback rasterizer may use UTF-8 octant block characters to preview the same pixel-font geometry in terminals without Kitty Graphics.
- The Kitty Graphics path remains authoritative when available; octant blocks are fallback only, not a replacement for the image path.

### Cache keys and flicker prevention

```rust
struct HeadlineImageKey {
    node:                 NodeId,
    node_version:         u64,
    level:                HeadingLevel,
    text_hash:            u64,
    target_cols:          u16,
    target_rows:          u16,
    theme_version:        u64,
    rasterizer_version:   u64,
    terminal_pixel_ratio: PixelRatio,
}
```

Rules:

- Cache raster bytes and transmitted Kitty image IDs separately.
- Reuse the transmitted image ID while only updating placement coordinates during scroll/reflow.
- Do not delete/recreate an on-screen headline placement just because the cursor moved elsewhere.
- Invalidate only when heading text, level, theme, width, rasterizer version, or terminal pixel ratio changes.
- Decode/raster work may happen off-thread, but the UI must draw a stable styled-text fallback until the exact-version image is ready.
- Worker responses include node version and are discarded when stale.
- The cache must have an LRU byte budget and a placement budget shared with normal Kitty images.

### Capability adapter

```rust
struct TerminalCapabilities {
    kitty_graphics: CapabilityState,
    truecolor:      bool,
    mouse:          bool,
    focus_events:   bool,
}

enum CapabilityState {
    Unknown,
    Supported,
    Unsupported,
    DisabledByConfig,
}
```

Rules:

- Probe Kitty Graphics support at startup and when the terminal changes.
- Do **not** infer support only from `$TERM`.
- The heading path must never emit Kitty Text Sizing / OSC-66 commands.
- If Kitty Graphics is unsupported, disabled, or over budget, use styled two-line text or octant-block fallback with stable cursor mapping.
- Snapshot tests must verify Kitty Graphics, unsupported fallback, and edit-mode fallback paths.

---

## 18. Images and Kitty Graphics

Images render via Kitty Graphics Protocol where supported; otherwise a neat bordered placeholder.

```text
╭─ image ─────────────────────╮
│ architecture-diagram.png    │
│ alt: system architecture    │
╰─────────────────────────────╯
```

### Lifecycle budget

```toml
[kitty.images]
enabled                    = true
max_transmitted_bytes      = 128000000
max_cached_images          = 128
max_visible_placements     = 32
max_decode_jobs            = 4
offscreen_retention_ms     = 3000
delete_on_scroll_offscreen = true
```

Rules:

- Decode down to display size before transmission.
- LRU cache of transmitted image IDs.
- Delete offscreen placements where supported.
- If deletion is unsupported or fails, stop creating placements after the budget and fall back to placeholders.
- Worker responses include node version and are discarded when stale.
- Images are selectable atoms; cursor can sit before/after an image.

---

## 19. Raw HTML preservation

The editor preserves raw HTML from GFM, but it does not render HTML as TUI boxes, flex rows, floats, details widgets, or a browser-like layout tree. There is no HTML renderer.

### Policy

```toml
[html]
preserve_raw         = true
render               = false
raw_edit_command     = true
show_compact_preview = true
sanitize_preview     = true
```

Rules:

- Raw HTML blocks and inline HTML survive import/export when the parser preserves them.
- Unknown, known, safe, and unsafe HTML all share the same rendering model: an atomic raw-HTML placeholder.
- The main document editor never exposes raw HTML unless the user explicitly opens the raw edit command.
- The raw edit command opens a contained code editor or popover for that one atom. It is not a split Markdown source mode and not a whole-document source editor.
- Sanitization is only for the compact preview label; it is not an HTML rendering pipeline.
- Links, images, boxes, columns, and floats must be modeled with native Markdown/editor nodes when they need rich editing behavior.

### Block rendering

A raw HTML block renders as a compact atom:

```text
╭─ raw HTML ───────────────────╮
│ <div class="callout">…       │
│ [Edit raw]                   │
╰──────────────────────────────╯
```

The preview line is a clipped, escaped summary of the opening tag or first text characters. It must not execute, parse layout, fetch resources, or apply CSS.

### Inline rendering

Inline raw HTML renders as an inline atom such as:

```text
‹html span›
```

The atom is selectable as a unit. Cursor positions are before/after the atom. Direct text insertion beside the atom works normally.

### Editing

- Raw HTML atoms are selected, cut, copied, pasted, moved, and deleted structurally.
- `Enter`, `Backspace`, and `Delete` near an atom must never reveal partial HTML in the main rendered document.
- `[Edit raw]` edits the raw payload and creates one undo step when applied.
- Invalid HTML remains preservable raw text; validation warnings appear in the problems panel only.

---

## 20. Code blocks and syntax highlighting

```rust
struct CodeBlock {
    id:                 NodeId,
    language:           Option<String>,
    text:               Rope,
    detected_language:  Option<String>,
    highlight_version:  u64,
}
```

### Language detection order

1. Fence language.
2. Shebang.
3. Filename comment hints.
4. Tree-sitter parser confidence (if grammar available).
5. Heuristics.
6. Plain text.

### Highlighting

Tree-sitter for incremental parsing where grammars are available; Syntect as fallback for broader coverage.

- Highlighting runs off the UI thread.
- Results carry node version; stale results are discarded.
- Editing remains responsive without highlights.
- Code-block selection/editing obeys normal text rules; spellcheck is off by default.

---

## 21. Spellcheck, languages, hyphenation

### Goals

- Multi-language document support.
- Per-document default language.
- Per-block language override.
- Per-selection language override where possible.
- Workspace dictionary; user dictionary.
- Ignore code, URLs, inline code, paths, identifiers by default.
- Ignore raw HTML internals unless raw-editing.
- Suggestions in popover.

### Backend

Use Nuspell/Hunspell-compatible dictionaries where available. Provide a trait so the backend can be swapped.

```rust
trait SpellBackend {
    fn check(&self, lang: LanguageId, word: &str) -> SpellResult;
    fn suggest(&self, lang: LanguageId, word: &str) -> Vec<String>;
}
```

Spellcheck must never block typing. Worker responses carry node version and are discarded if stale.

### Hyphenation

```toml
[hyphenation]
enabled         = true
language        = "en-US"
min_word        = 8
min_prefix      = 3
min_suffix      = 3
show_soft_hyphen = true
```

- Hyphenation affects visual line breaking only.
- It does not insert characters into the model.
- Copy/export uses the original text.
- Selection across hyphenated lines maps to original grapheme positions.

---

## 22. AI support

AI is disabled unless fully configured.

```toml
[ai]
enabled            = false
provider           = "openai"
model              = ""
api_key_env        = "OPENAI_API_KEY"
base_url           = "https://api.openai.com/v1"
temperature        = 0.2
max_output_tokens  = 1200
stream             = true
```

Rules:

- Empty `model` disables AI.
- Missing API key disables AI.
- No network call when AI is disabled.
- AI operates on a snapshot of the selected text plus the prompt.
- AI never mutates the document while streaming.
- AI results appear in a preview popover with `[Replace selection]`, `[Insert after]`, `[Copy]`, `[Cancel]`.
- Applying AI output is **one** undoable transaction.
- The configured model is shown in the AI popover.
- Use the current OpenAI API documentation when implementing. Do not hardcode a dated default model.

---

## 23. Document width and softwrap

Document prose width is separate from terminal width.

```toml
[editor]
default_document_width = 80
min_document_width     = 48
max_document_width     = 120
softwrap               = true
center_document        = true
grapheme_cursor        = true
mouse                  = true

[columns]
mode              = 1
balance           = "height"
min_column_width  = 32
gap               = 4
hyphenate         = true
span_headings     = true
span_tables       = true
span_code_blocks  = true
span_images       = true
```

### Width slider

A status-bar control:

```text
width 80  ◀━━━━━━●━━━━▶
```

Interactions:

- Click/drag the slider changes document width.
- `Alt+[` and `Alt+]` adjust by 2 columns.
- `Ctrl+Alt+Left/Right` adjust by 2 columns.
- Holding `Shift` adjusts by 10 columns.
- Reflow is live and must preserve cursor and scroll anchor.
- Softwrap changes are visual only — they must not insert Markdown line breaks.
- Tables and code blocks may exceed prose width and scroll horizontally.

### Column rendering mode

Column rendering is a visual flow mode for prose-heavy documents. It is controlled from the status bar and config, and can be set to one, two, or three columns.

```text
cols 1  ◀━━●━━━━▶
cols 2  ◀━━━━●━━▶
cols 3  ◀━━━━━━●▶
```

```toml
[columns]
mode              = 1        # 1 | 2 | 3
balance           = "height" # height | greedy
min_column_width  = 32
gap               = 4
hyphenate         = true
span_headings     = true
span_tables       = true
span_code_blocks  = true
span_images       = true
```

#### Flow rules

- Column mode changes layout only. It never inserts hard line breaks or changes saved Markdown.
- Paragraphs, list items, blockquote paragraphs, and plain text inside table cells may flow inside a column.
- H1/H2 headlines span all columns by default so Kitty headline placements are stable.
- Tables, fenced code blocks, block images, thematic breaks, raw HTML atoms, and oversized widgets span all columns by default.
- H3–H6 may either flow or span according to `span_headings`; the default is to span.
- A list item is kept together when it fits in the remaining column height; otherwise it may break between child blocks, never inside the bullet/checkbox atom.
- Task checkbox atoms stay visually attached to the first line of their item.

#### Balancing

The default `height` balancer targets equal visual height columns for the current viewport and document width.

1. Layout candidate blocks at the effective column width.
2. Measure wrapped visual height, including hyphenation opportunities.
3. Choose breakpoints between block boundaries first.
4. If a single paragraph exceeds a column, break at a wrapped line boundary.
5. Prefer hyphenation points only when they reduce raggedness or avoid a severely imbalanced column.
6. Preserve model order for cursor movement, selection, find/replace, copy, and export.

The `greedy` balancer fills column 1, then column 2, then column 3. It is simpler and may be useful for debugging, but `height` is the polished default.

#### Cursor, hit testing, and selection

- Display-list hit testing maps each column cell back to model positions.
- Arrow movement follows visual geometry within the same column, then moves to the nearest visual line in the next/previous column.
- `Shift+Arrow` selection remains model-order selection even when the highlight spans multiple columns.
- Mouse drag selection across columns follows visual drag path but normalizes to model order.
- Scroll anchoring uses model position plus visual column index to avoid jumps during reflow.

#### Performance

Changing column mode or document width invalidates the column-flow root, not the entire document model. The layout engine may skip stale intermediate column layouts when dragging the status-bar control, but the final displayed state must correspond to the latest slider value.

---

## 24. Side panels, status bar, workspace

### Outline panel

Headings and document sections; click jumps to section. Drag-reorder is a later affordance, but the model must allow it.

### Workspace panel

Open files; recent files; project tree; document symbols; problems (spell, broken links, missing images, parse warnings); saved searches. Collapsible.

No Git integration is part of the application. Do not shell out to Git, link against Git libraries, display branch/status data, or watch repository state. Git-aware workflows belong outside `mdtui`.

### Status bar (single dense line)

Shows: file name, dirty state, current logical row/column, current block type, selection length, word count, document width slider, column mode, spellcheck language, AI state, terminal capability indicators, table cell coordinate when inside a table, link target when inside a link, and raw-HTML atom status when applicable.

---

## 25. File watching, sessions, conflicts

```toml
[files]
watch_external_changes  = true
external_check_on_focus = true
conflict_policy         = "prompt"

[session]
restore_open_tabs       = true
remember_closed_tabs    = false
store_path              = ".mdtui/session.json"
```

### Conflict bar

```text
DISK CHANGED │ README.md changed outside editor │ [Diff] [Reload] [Keep Mine] [Save As]
```

Rules:

- Clean buffer reloads automatically.
- Dirty buffer shows the conflict bar.
- Saving over an externally-changed file requires explicit confirmation.
- A deleted file keeps the buffer and warns the user.

### Session restore

- Remember open tabs between runs unless a tab was explicitly closed.
- Each tab stores scroll position, selection, width, side panel state, and dirty state.
- Closed tabs are removed from session restore.

---

## 26. Async architecture

### UI ownership

The UI thread owns the document model, selection, undo stack, layout cache mutation, terminal draw scheduling, and command routing. Workers never mutate the document.

### Worker protocol

```rust
enum WorkerRequest {
    HighlightCode    { node: NodeId, version: u64, text: String, language: Option<String> },
    Spellcheck       { node: NodeId, version: u64, text: String, language: LanguageId },
    DetectLanguage   { node: NodeId, version: u64, text: String },
    DecodeImage      { node: NodeId, version: u64, src: PathBuf, target: ImageTarget },
    AiRequest        { request_id: RequestId, selection_snapshot: String, prompt: String },
}

enum WorkerResponse {
    Highlighted        { node: NodeId, version: u64, spans: Vec<StyleSpan> },
    Spelling           { node: NodeId, version: u64, issues: Vec<SpellIssue> },
    LanguageDetected   { node: NodeId, version: u64, language: Option<String> },
    ImageDecoded       { node: NodeId, version: u64, image: DecodedImage },
    AiChunk            { request_id: RequestId, text: String },
    AiDone             { request_id: RequestId },
    AiError            { request_id: RequestId, message: String },
}
```

### Backpressure

```toml
[performance]
worker_queue_size         = 1024
max_inflight_spellcheck   = 128
max_inflight_highlight    = 64
max_inflight_images       = 8
drop_stale_worker_jobs    = true
```

When full: drop stale diagnostics → drop offscreen image jobs → keep save/conflict/AI-cancel messages → never block UI.

UI input and draw scheduling are higher priority than all worker responses. Worker messages are drained opportunistically within the frame budget; stale responses are discarded without blocking typing, cursor movement, or key-release handling.

---

## 27. Undo and redo

Undo uses grouped transactions.

```rust
struct UndoStep {
    transactions:     Vec<Transaction>,
    before_selection: Selection,
    after_selection:  Selection,
    description:      String,
}
```

### Coalescing

Typing coalesces only when: same text node, same insertion direction, no selection-replacement boundary, no cursor jump, under coalesce timeout, no newline, not paste.

### Always separate undo steps

Enter; Backspace at structural boundary; Delete selection; Paste; Toggle mark; Create/edit link; Add/remove table row/column; Toggle checkbox; AI replacement; raw-HTML atom edit.

Width changes do not affect document undo unless saved as document metadata.

Undo/redo restores **both** document state and selection.

---

## 28. Find and replace

Find searches rendered semantic text, not source Markdown.

**Included by default:** paragraphs, headings, list-item text, table-cell text, code-block text, link visible text, image alt text.

**Excluded by default:** Markdown syntax, link URLs, image paths, raw HTML internals unless raw-edit mode is open, frontmatter.

Matches may cross inline mark boundaries. Replace is transaction-based and never exposes markers.

---

## 29. Configuration schema

Precedence:

```text
CLI flags
Environment variables
Workspace config:  .mdtui.toml
User config:       ~/.config/mdtui/config.toml
Built-in defaults
```

Master schema:

```toml
[app]
theme                = "dark_amber"
autosave             = false
autosave_interval_ms = 30000

[markdown]
parser       = "comrak"
gfm_spec_url = "https://github.github.com/gfm/"
gfm_profile  = "0.29-gfm"

[markdown.export]
heading_style            = "atx"
list_marker              = "-"
ordered_marker           = "."
table_style              = "pipe"
line_width               = 80
preserve_reference_links = true
preserve_html            = true

[editor]
default_document_width = 80
min_document_width     = 48
max_document_width     = 120
softwrap               = true
center_document        = true
grapheme_cursor        = true
mouse                  = true

[columns]
mode              = 1
balance           = "height"
min_column_width  = 32
gap               = 4
hyphenate         = true
span_headings     = true
span_tables       = true
span_code_blocks  = true
span_images       = true

[selection]
mouse_drag         = true
shift_arrow        = true
show_style_popover = true

[table]
cell_wrap            = true
max_cell_width       = 32
min_cell_width       = 4
horizontal_scroll    = true
sticky_header        = true
sticky_first_column  = false
oversize_indicator   = true

[table.export]
complex_cells = "flatten"   # flatten | error

[html]
preserve_raw         = true
render               = false
raw_edit_command     = true
show_compact_preview = true
sanitize_preview     = true

[spellcheck]
enabled              = false
language             = "en-US"
ignore_code          = true
ignore_links         = true
ignore_inline_code   = true
workspace_dictionary = ".mdtui/dictionary.txt"

[hyphenation]
enabled         = true
language        = "en-US"
min_word        = 8
min_prefix      = 3
min_suffix      = 3
show_soft_hyphen = true

[kitty]
enabled  = true
graphics = true

[kitty.headlines]
enabled              = true
use_graphics         = true
allow_text_sizing    = false
fallback             = "octant" # octant | styled_text
max_cached_headlines = 64

[kitty.images]
enabled                    = true
max_transmitted_bytes      = 128000000
max_cached_images          = 128
max_visible_placements     = 32
max_decode_jobs            = 4
offscreen_retention_ms     = 3000
delete_on_scroll_offscreen = true

[ai]
enabled            = false
provider           = "openai"
model              = ""
api_key_env        = "OPENAI_API_KEY"
base_url           = "https://api.openai.com/v1"
temperature        = 0.2
max_output_tokens  = 1200
stream             = true

[performance]
target_fps              = 60
frame_budget_ms         = 16.6
input_poll_per_frame    = true
drop_stale_input_events = true
coalesce_motion_events  = true
key_release_stops_now   = true
worker_queue_size       = 1024
max_inflight_spellcheck = 128
max_inflight_highlight  = 64
max_inflight_images     = 8
drop_stale_worker_jobs  = true

[session]
restore_open_tabs    = true
remember_closed_tabs = false
store_path           = ".mdtui/session.json"

[files]
watch_external_changes  = true
external_check_on_focus = true
conflict_policy         = "prompt"
```

Validation:

- Unknown keys warn.
- Invalid enum values fail with the path and expected values.
- Empty AI model disables AI; missing API key disables AI.
- Workspace config cannot store secret values directly.

---

## 30. Implementation order

This is the order to work in. Do **not** stop after any section and claim the product is done. The acceptance harness defines completion.

1. **Remove modal editing completely.** Delete any Normal/Insert/Visual logic. Ensure printable input edits directly. Add the regression test for `i` inserting `i`.
2. **Establish the document model and transaction layer as authoritative.** All input commands produce transactions. Selection and cursor are model anchors. Markdown parser/serializer are adapters.
3. **Build GFM import/export with the comrak adapter.** Cover core GFM and raw-HTML preservation. Add semantic round-trip fixtures.
4. **Build the display list, hit testing, and 60 fps input loop.** Implement `position_to_cursor`, `hit_test`, `range_to_rects`. Softwrap by document width. Preserve cursor through reflow. Prove held-key release stops movement immediately and stale input is dropped.
5. **Implement direct text editing.** Typing, paste, Enter, Backspace, Delete. Grapheme-correct deletion. Undo/redo.
6. **Implement selection fully.** Shift arrows, mouse drag, cross-block selection, structural deletion of selection.
7. **Implement marks and the styling popover.** Bold, italic, strike, inline code, link. Keyboard shortcuts and popover actions.
8. **Implement lists and task lists.** Tight rendering, Enter-creates-item, empty-item-exits, checkbox toggle, structural merge/delete.
9. **Implement real TUI tables.** Unicode borders, editable cells, navigation, `Ctrl+Arrow` row/column insertion, oversized-table behavior.
10. **Implement headings via Kitty Graphics.** Capability detection, rasterization, image cache, edit-mode fallback, and clean non-Kitty fallback. Do not implement Kitty Text Sizing / OSC-66.
11. **Implement images with Kitty Graphics.** Inline/block images, placeholders, cache lifecycle.
12. **Implement raw HTML preservation.** Atomic raw HTML blocks/inline atoms, compact preview, explicit raw edit command, and round-trip tests. No HTML renderer.
13. **Implement code highlighting and language detection.** Tree-sitter first; Syntect fallback; async versioned responses.
14. **Implement spellcheck and hyphenation.** Language config, dictionaries, softwrap hyphenation, suggestions UI.
15. **Implement side panels, status bar, width slider, column mode, and workspace.**
16. **Implement the AI selection workflow.** Disabled-unless-configured; prompt popover; streaming preview; one-undo-step apply.
17. **Implement session restore and file watching.** Restore tabs unless closed; external-change conflict bar.
18. **Polish theme and snapshots until the app looks intentional.** Use the visual design in §5 as the reference. No rough ASCII placeholders where Unicode UI is required.

### Vertical-slice discipline

Prefer small, complete vertical slices.

A good slice for *task list checkbox*:

```text
model node
parser import
renderer
hit test
keyboard toggle
mouse toggle
markdown export
tests
```

A bad slice:

```text
renderer only with no editing
keyboard shortcut only with no model support
visual mock with no save/load behavior
```

---

## 31. Test harness

A feature is not done until tests prove it. Run before every final response:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If snapshot tests exist:

```bash
cargo insta test --workspace
```

If property/fuzz tests exist:

```bash
cargo test --workspace --features proptest
```

### Dev dependencies

```toml
[dev-dependencies]
insta             = "*"
proptest          = "*"
pretty_assertions = "*"
vt100             = "*"
rexpect           = "*"
tempfile          = "*"
```

### Test layout

```text
crates/mdtui-core/src/tests/
crates/mdtui-render/src/tests/
crates/mdtui-tui/tests/interaction/
crates/mdtui-markdown/tests/fixtures/
crates/mdtui-terminal/tests/
```

### Test types

- Unit tests for transactions and mappings.
- Golden tests for GFM import/export.
- Snapshot tests for rendered Ratatui buffers.
- Pseudo-terminal interaction tests for keyboard/mouse.
- Property tests for edit sequences.
- Fuzz tests for parser/import stability.
- Capability simulation tests for Kitty support.

---

## 32. Acceptance tests

These tests must exist. Names are normative — they document required behavior.

### No modal editing

```rust
#[test] fn app_starts_in_direct_editing_mode() {}
#[test] fn pressing_i_inserts_literal_i_not_insert_mode() {}
#[test] fn printable_character_inserts_without_entering_insert_mode() {}
#[test] fn status_bar_never_shows_vim_insert_or_normal_mode() {}
```

### Markdown syntax invisibility

```rust
#[test] fn rendered_heading_does_not_show_hash_markers() {}
#[test] fn rendered_bold_does_not_show_star_markers() {}
#[test] fn rendered_link_does_not_show_markdown_link_syntax() {}
#[test] fn rendered_task_item_does_not_show_bracket_marker() {}
#[test] fn rendered_table_does_not_show_pipe_delimiter_source() {}
#[test] fn rendered_editing_never_exposes_markdown_markers_after_deletion() {}
```

The deletion test must cover headings, bold, italic, inline code, bullet list, task list, table, and link. Use fixture-aware assertions so literal user text containing `#`, `|`, or `*` is still allowed.

### Selection

```rust
#[test] fn shift_right_extends_selection() {}
#[test] fn shift_arrow_extends_selection() {}
#[test] fn shift_down_extends_selection_across_softwrap() {}
#[test] fn mouse_drag_selects_rendered_text() {}
#[test] fn selection_can_cross_inline_mark_boundaries() {}
#[test] fn deleting_selection_across_marks_leaves_no_orphan_nodes() {}
#[test] fn deleting_selection_across_inline_marks_removes_empty_marks() {}
#[test] fn selecting_text_shows_floating_styling_toolbar() {}
#[test] fn styling_bar_appears_for_non_empty_selection() {}
#[test] fn toolbar_bold_wraps_selection_in_strong_without_inserting_markers() {}
```

### Lists

```rust
#[test] fn tight_list_renders_without_blank_lines_between_items() {}
#[test] fn tight_list_renders_without_extra_blank_lines() {}
#[test] fn enter_inside_list_item_creates_new_list_item() {}
#[test] fn enter_in_list_item_creates_next_list_item() {}
#[test] fn enter_in_task_item_creates_unchecked_task_item() {}
#[test] fn enter_on_empty_list_item_exits_list() {}
#[test] fn backspace_at_start_of_second_list_item_merges_without_marker_text() {}
#[test] fn backspace_at_start_of_first_item_unwraps_without_marker_text() {}
#[test] fn clicking_checkbox_toggles_task_state() {}
#[test] fn space_on_checkbox_toggles_task_state() {}
```

Rendered fixtures for the merge test:

Before — `• abc \n • ▌def` → After — `• abc▌def`. No `-` text node may exist after the merge.

### Tables

```rust
#[test] fn table_renders_with_unicode_borders_not_pipes() {}
#[test] fn gfm_table_renders_as_unicode_grid_not_pipes() {}
#[test] fn typing_inside_table_cell_updates_cell_content() {}
#[test] fn table_cell_accepts_typing() {}
#[test] fn enter_inside_table_cell_does_not_destroy_table() {}
#[test] fn tab_moves_to_next_cell() {}
#[test] fn ctrl_right_in_table_adds_column_after() {}
#[test] fn ctrl_left_in_table_adds_column_before() {}
#[test] fn ctrl_down_in_table_adds_row_after() {}
#[test] fn ctrl_up_in_table_adds_row_before() {}
#[test] fn ctrl_right_adds_column_after_current_cell() {}
#[test] fn ctrl_down_adds_row_after_current_cell() {}
#[test] fn oversized_table_has_horizontal_viewport() {}
#[test] fn table_empty_row_survives_save_reload() {}
```

The Unicode-borders test must verify the rendered buffer contains `┌`, `┬`, `│`, `└` and does **not** use pipe-table syntax as the table structure.

### Kitty

```rust
#[test] fn heading_uses_kitty_graphics_when_supported() {}
#[test] fn heading_never_emits_text_sizing_or_osc66() {}
#[test] fn heading_image_cache_reuses_id_across_cursor_moves() {}
#[test] fn heading_switches_to_editable_text_when_cursor_enters_placement() {}
#[test] fn heading_restores_cached_image_when_cursor_leaves() {}
#[test] fn heading_falls_back_cleanly_without_kitty_graphics() {}
#[test] fn image_uses_kitty_graphics_when_supported() {}
#[test] fn image_renders_placeholder_when_graphics_unsupported() {}
#[test] fn offscreen_images_are_evicted_by_lru_budget() {}
```

### Raw HTML

```rust
#[test] fn raw_html_unknown_block_is_preserved_on_roundtrip() {}
#[test] fn raw_html_block_renders_as_atomic_placeholder() {}
#[test] fn raw_html_inline_renders_as_selectable_atom() {}
#[test] fn raw_html_edit_command_updates_payload_and_export() {}
#[test] fn raw_html_never_invokes_tui_layout_renderer() {}
```

### Columns

```rust
#[test] fn two_column_mode_balances_paragraph_blocks() {}
#[test] fn three_column_mode_balances_with_hyphenation() {}
#[test] fn h1_h2_span_columns_by_default() {}
#[test] fn selection_across_columns_is_model_ordered() {}
#[test] fn key_release_stops_held_arrow_without_replayed_events() {}
#[test] fn burst_typing_drops_no_committed_text_and_accumulates_no_stale_motion() {}
```

### Width and wrapping

```rust
#[test] fn changing_document_width_reflows_without_moving_cursor_model_position() {}
#[test] fn document_width_slider_changes_softwrap_without_changing_markdown() {}
#[test] fn width_slider_changes_document_width() {}
#[test] fn hyphenation_affects_visual_wrap_only_not_exported_text() {}
```

### Undo and mapping

```rust
#[test] fn undo_restores_selection_after_structural_delete() {}
#[test] fn redo_restores_selection_after_undo() {}
#[test] fn destroyed_cursor_node_recovers_to_nearest_valid_position() {}
#[test] fn stale_worker_response_is_discarded_after_node_version_change() {}
#[test] fn stale_spellcheck_result_is_discarded_after_edit() {}
```

### AI

```rust
#[test] fn ai_is_disabled_when_model_empty() {}
#[test] fn ai_is_disabled_when_api_key_missing() {}
#[test] fn ai_replace_selection_is_single_undo_step() {}
```

---

## 33. Visual snapshots

Add snapshot fixtures:

```text
fixtures/visual/headings.md
fixtures/visual/lists-tight.md
fixtures/visual/task-list.md
fixtures/visual/table-simple.md
fixtures/visual/table-oversized.md
fixtures/visual/links-and-marks.md
fixtures/visual/code-blocks.md
fixtures/visual/raw-html.md
fixtures/visual/columns-two.md
fixtures/visual/columns-three.md
fixtures/visual/images.md
fixtures/visual/spellcheck.md
fixtures/visual/selected-text.md
fixtures/visual/style-popover.md
```

Each snapshot includes: normal terminal fallback; Kitty-capable rendering where relevant; narrow-width rendering; wide-width rendering.

Required named snapshots:

```text
snapshots/
  heading_kitty_fallback.snap
  tight_bullet_list.snap
  task_list.snap
  unicode_table.snap
  selected_text_toolbar.snap
  editable_table_cell.snap
  raw_html_atom.snap
  columns_two_balanced.snap
  columns_three_balanced.snap
  image_fallback.snap
  outline_panel.snap
  status_bar_columns.snap
```

### Forbidden visual regressions

- Vim mode indicator anywhere in the UI.
- Pipe table source as the rendered table structure.
- Blank lines between tight list items.
- Markdown markers for headings, emphasis, task states, links, or table delimiters.
- Default-looking, unstyled popovers.
- Broken borders.
- Cursor lost after reflow.

---

## 34. Forbidden implementations

Do not implement:

- Vim-style mandatory insert mode.
- Pipe ASCII tables as rendered table UI.
- Markdown-source editing disguised as rich editing.
- Fake checkboxes using literal `[ ]` / `[x]` text.
- Fake bold by inserting `**`.
- Fake links by inserting `[text](url)` into editable text.
- Kitty Text Sizing / OSC-66 for headings.
- ASCII-art or oversized-terminal-text headings as the primary renderer.
- HTML safe-subset rendering, HTML flex/floats, or browser-like HTML layout inside the editor.
- Source-level find/replace as the only find implementation.
- Full-document reparsing on every keystroke.
- Replaying accumulated key-repeat, movement, scroll, or mouse events after the key/button is released.
- Blocking spellcheck on the UI thread.
- Blocking AI on the UI thread.
- Hidden network calls.
- Unbounded image cache.
- Mouse features without tests.

---

## 35. Memory file requirement

The agent must create and maintain:

```text
.agent/markdown-editor-memory.md
```

with at minimum the following content. Update this file whenever an architectural decision changes.

```markdown
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
```

---

## 36. Definition of done

A task is done only when **all** of the following are true:

1. It works through the real TUI interaction path.
2. It updates the structured document model.
3. It saves to valid GFM.
4. It reloads from GFM correctly.
5. It has model tests.
6. It has render snapshot tests where visual.
7. It has interaction tests where input behavior matters.
8. It does not expose hidden Markdown syntax.
9. It does not regress 60 fps interaction, typing latency, or immediate key-release stop behavior (§10 targets).
10. It is documented in `.agent/markdown-editor-memory.md` if architectural.

The full feature set is done when:

- The acceptance harness passes.
- Visual snapshots match the amber/charcoal UI quality of §5.
- The editor can open, edit, and save real GFM files without exposing source markers.
- Tables, lists, selection, styling popover, Kitty headline/images, raw-HTML atoms, width slider, column mode, spellcheck, AI config, side panels, and session restore are all implemented or cleanly disabled by explicit config with visible fallback.
- `.agent/markdown-editor-memory.md` exists and matches current decisions.
- No TODO stubs remain in core editing paths.
- Performance logging shows 60 fps interaction, the input-to-redraw latency targets, and immediate stop on key release on representative fixtures.

### Bug-fix protocol

For every bug:

1. Add a failing regression test first.
2. Fix the underlying model/transaction/rendering issue.
3. Verify no syntax leakage.
4. Update snapshots if the visual change is intentional.
5. Update the memory file if the fix changes architecture.

---

## 37. Agent self-check

The agent must not claim completion until it can answer **yes** to all of these:

```text
[ ] Does the app open in direct editing mode with no insert-mode requirement?
[ ] Does pressing i insert the letter i?
[ ] Do Shift+Arrow selections work?
[ ] Does mouse drag selection work in a pseudo-terminal test?
[ ] Does a styling popover appear for selected text?
[ ] Does the styling popover apply marks structurally (no inserted ** or _)?
[ ] Are GFM tables rendered with Unicode borders, not pipe ASCII?
[ ] Can users edit table cells?
[ ] Do Ctrl+arrows add table rows/columns as specified?
[ ] Do tight lists render without blank lines?
[ ] Does Enter in a list item create a new item?
[ ] Does Enter on an empty list item exit the list?
[ ] Does Backspace at list boundaries avoid exposing markers?
[ ] Do checkboxes toggle by click and by Space?
[ ] Do H1/H2 headings render through Kitty Graphics under simulated support without emitting text-sizing/OSC-66 commands?
[ ] Does inline image rendering use Kitty Graphics under simulated support?
[ ] Does unsupported Kitty fallback look clean?
[ ] Do raw HTML blocks and inline HTML render as atomic placeholders/atoms and round-trip unchanged?
[ ] Does 2/3-column mode balance prose while preserving model-order selection?
[ ] Does held-key movement stop immediately on key release with no replayed backlog?
[ ] Does changing document width preserve cursor model position?
[ ] Does spellcheck run off-thread and discard stale results?
[ ] Is AI disabled with empty config and undoable when used?
[ ] Does the theme match the dark_amber palette in §5?
[ ] Is .agent/markdown-editor-memory.md present and current?
```

If any answer is no, continue coding or tighten the harness.

---

## 38. Source notes

Use these sources while implementing:

- Official GitHub Flavored Markdown spec — <https://github.github.com/gfm/>
- Comrak parser — <https://github.com/kivikakk/comrak>, <https://docs.rs/comrak>
- Ratatui — <https://ratatui.rs/>
- Kitty Graphics Protocol — <https://sw.kovidgoyal.net/kitty/graphics-protocol/>
- ProseMirror guide — <https://prosemirror.net/docs/guide/>
- Lexical editor state and selection — <https://lexical.dev/docs/concepts/editor-state>, <https://lexical.dev/docs/concepts/selection>
- Xi editor rope science — <https://xi-editor.io/docs/rope_science_00.html>
- Unicode UAX #29 — <https://www.unicode.org/reports/tr29/>
- Tree-sitter — <https://tree-sitter.github.io/>
- Nuspell — <https://nuspell.github.io/>
- Rust hyphenation — <https://docs.rs/hyphenation>
- Syntect — <https://github.com/trishume/syntect/>

---

## Final reminder

The hardest part of this project is not parsing Markdown. The hardest part is **stable rendered editing**.

Cursor mapping, selection mapping, transactions, undo, and display-list hit-testing are not incidental details. They are the editor.