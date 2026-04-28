---
name: awesome-tui
description: "Terminal UI craft skill for building polished, keyboard-first TUIs inspired by sqv. Use whenever designing, generating, reviewing, or refactoring terminal interfaces, especially Ratatui/crossterm, Bubble Tea, Textual, curses, or prompt-style CLIs. Enforces dogmatic theming, sacred accent colors, excellent help/status surfaces, tabs, panel navigation, scrolling, mouse support, and shortcut design."
---

# Awesome TUI

Build terminal interfaces that feel intentional, fast, tactile, and calm. The model is `sqv`: keyboard-first, mouse-friendly, warm dogmatic theme, precise status/help surfaces, strong panel and tab affordances, and no generic AI terminal chrome.

## Non-Negotiables

1. **The theme is dogma.** Do not invent colors, random gradients, glow effects, or user-configurable theme sprawl unless explicitly requested.
2. **Accent is sacred.** Use accent only for active focus, primary mode/status, selected matches, sort direction, important labels, and direct action affordances.
3. **Keyboard first, mouse complete.** Every mouse action must have a keyboard path; every keyboard action should expose visible affordance through status/help.
4. **Help/status are product surfaces, not afterthoughts.** Add or update them with every interaction change.
5. **Behavior before decoration.** Every glyph, color, border, and icon must convey state, navigation, data type, actionability, or progress.
6. **No noisy animation.** Data-focused TUIs should be still. A minimal loading stripe/spinner is acceptable; decorative motion is not.
7. **Never strand the user.** Esc closes popups or backs out, Ctrl-Q quits, Tab/BackTab moves focus, help is reachable with `?`, and destructive actions require confirmation.

## Sacred Palette

Prefer a single `Theme`/palette struct and pass it everywhere. Never inline arbitrary colors in render code.

| Token | Hex | Use |
|------|-----|-----|
| `bg` | `#1d1b1a` | Main canvas |
| `bg_soft` | `#24211f` | Secondary surfaces, inactive tabs, statusbar |
| `bg_raised` | `#2a2624` | Active row, active tab, popup body, headers |
| `line` | `#3a332f` | Normal borders and dividers |
| `line_soft` | `#2f2a27` | Shadows, subtle tracks |
| `fg` | `#e8dfd3` | Primary text |
| `fg_dim` | `#a89c8a` | Secondary readable text |
| `fg_mute` | `#6b6459` | Metadata, inactive hints |
| `fg_faint` | `#4a453e` | Nulls, gutters, disabled state |
| `accent` | `#d99a5e` | Sacred focus/action color |
| `red` | `#e06c75` | Errors, destructive state |
| `yellow` | `#e5c07b` | Warnings, numeric/int badges |
| `green` | `#a3b565` | Success, truthy values |
| `teal` | `#7cb7a8` | Text/table entities |
| `blue` | `#82aadc` | Numbers/reals, not primary accent |
| `purple` | `#c08bc0` | Blobs/indexes |
| `pink` | `#d88aa0` | Dates/datetimes |

**Style rule:** active focus gets `accent`; normal structure gets `line`; inactive structure gets `fg_mute`/`line_soft`; content is typed with semantic colors. Do not promote blue/cyan into the app identity.

## Layout Doctrine

Design layouts as stable regions with remembered rectangles for hit-testing.

1. **Bottom statusbar:** always reserve one row at the bottom. It is the live control surface.
2. **Main split:** optional sidebar on the left; body on the right. Sidebar width should be bounded, not proportional chaos: about one third of the terminal, clamped to a useful range.
3. **Tabs above body:** show tabs only when there are open items. A three-row tabbar can draw a roof, label, and active join into the panel below.
4. **Primary panel:** render content inside a rounded bordered block. Focused panel border uses `accent`; unfocused panel border uses `line`.
5. **Right-aligned frame label:** panel titles can sit on the top-right border in bold `accent`, e.g. ` TABLE - users `.
6. **Popups:** centered, bounded by terminal size, raised background, rounded accent border, subtle one-cell shadow using `line_soft`, and `Clear`/equivalent behind them.
7. **Responsive degradation:** if width or height is too small, render less, not broken. Clamp dimensions, saturating-subtract, and avoid panics.

## Statusbar Standard

The statusbar should answer: where am I, what mode am I in, what is selected, what changed, and what can I do next?

Use segmented status:

- Mode pill: inverted `accent` background with `bg` foreground, bold.
- Current object: table/file/view name in dim bold text.
- Conditional state: filter count, sort direction, jump breadcrumb, read-only/dirty/error state.
- Position: row/total and column, formatted compactly.
- Contextual hints: only actions that are valid now, e.g. `[enter] open  [e] modify  [s] sort  [f] filter  [ctrl-q] quit`.
- Right preview: selected value or focused content preview, truncated to avoid eating the bar.

Do not show stale or impossible hints. If a focused cell cannot be set to null, omit `[n] set null`. If a panel is hidden, do not hint panel switching except the toggle.

## Help Surface Standard

Every mature TUI needs an in-app help popup.

- `?` toggles help from any non-confirming mode.
- `Esc` closes help.
- Help must be grouped by workflows, not dumped alphabetically: Navigation, Editing, Filtering & Sorting, Tabs & Sidebar, Command Palette, Misc.
- Include keyboard and mouse equivalents: wheel scroll, Shift-wheel horizontal scroll, click cell, click tab, middle-click close.
- Help is scrollable with Up/Down and PageUp/PageDown.
- Show a bottom hint like `? scroll 1 / 3`.
- When adding or changing shortcuts, update help and status hints in the same change.

## Input Model

Prefer an explicit message/update architecture:

```text
terminal event -> translate_event -> Message -> App::update -> state mutation -> dirty render
```

Rules:

- Translate raw terminal events into domain messages early.
- Keep app modes explicit: Browse vs Edit, popup kind, focus pane.
- Store render-time rectangles (`screen_area`, `tabbar_area`, `sidebar_area`, `grid_inner_area`) for precise mouse hit-testing.
- Render only when dirty. A 30Hz tick is enough for responsiveness and minimal loading indicators.
- Use background work for blocking IO; send results back as messages.
- On any interaction that affects visible state, set dirty.

## Keyboard Doctrine

Shortcuts must be predictable, mnemonic, and layered:

- Navigation: arrows and `h j k l`.
- Bounds: Home/End for row bounds; Ctrl-Home/Ctrl-End for whole table/document bounds.
- Paging: PageUp/PageDown and Ctrl-Up/Ctrl-Down.
- Panel focus: Tab and BackTab.
- Sidebar: Ctrl-B toggles.
- Command palette: Ctrl-P / Ctrl-Shift-P.
- Help: `?`.
- Quit: Ctrl-Q.
- Edit/open primary thing: Enter.
- Direct edit: `e`.
- Insert/delete: `i` / `d`.
- Sort/filter: `s` / `f`; clear filters with Shift-F.
- Jump: `j` when on a link/foreign key; otherwise keep `j` as down.
- Back: Backspace returns through jump stack.
- Undo: Ctrl-Z.

Context matters: the same key may do different things by mode only when the status/help makes it obvious.

## Mouse Doctrine

Mouse support should feel native, not bolted on.

- Wheel scrolls the panel under the pointer, not whichever panel was previously focused.
- Pointer scrolling should also move focus to that panel.
- Shift-wheel or equivalent should scroll columns horizontally when the grid supports it.
- Click headers to focus and sort.
- Click cells to focus.
- Click row gutters to focus rows.
- Click tabs to activate; click close glyph or middle-click tab to close.
- Click sidebar rows to select/open/toggle.
- Scrollbar thumb supports click-to-jump and drag-to-scroll.
- Popup-specific hit-testing is allowed for rich controls; otherwise ignore mouse while editing to avoid accidental writes.

Hit-testing must use the exact rectangles and glyph positions from render code. Do not duplicate approximate layout math in a way that drifts.

## Tabs

Tabs should be functional objects, not decorative pills.

- Active tab uses `fg` on `bg_raised`, bold.
- Inactive tabs use `fg_dim` on `bg_soft`.
- Focused active tab border and close glyph use `accent`.
- Draw close affordance as `×` and hit-test it exactly.
- Active tab can visually join into the panel below; this makes the body feel attached.
- Provide next/previous tab messages even if not yet bound; keep tab state isolated from content loading.

## Panels and Sidebar

Panels must make focus obvious.

- Focused border: `accent`.
- Unfocused border: `line`.
- Sidebar is a schema/navigation list with collapsible sections.
- Section headers are bold muted text; entity icons/colors carry type.
- Selection highlight uses `accent` foreground on `bg_raised`, bold, with a clear symbol like `▸ `.
- Sidebar has its own scrollbar when content exceeds viewport.
- `Esc` from grid may return focus to sidebar when sidebar is visible.

## Scrolling and Large Data

Great TUIs make huge data feel local.

- Separate focused item from viewport start.
- Keep a scroll margin around focused rows so movement does not pin to the top/bottom.
- Use virtual windows for large datasets; prefetch before the focused row reaches the window edge.
- Fetch enough before and after the viewport to make navigation feel continuous.
- Clamp all target rows and offsets.
- Page size should be viewport minus one row, not a magic constant.
- Use a visual scrollbar with proportional thumb.
- Dragging the scrollbar maps thumb position back to absolute row with rounding.
- For sorted text datasets, consider an alphabet rail (`#`, `A`-`Z`) that jumps to approximate offsets.

## Grid and Table Craft

Data grids need typographic discipline.

- Header is multi-row: name row, metadata/type row, divider row.
- Use badges for data types: `INT`, `REA`, `TXT`, `BLB`, `NUM`, `DAT`, `DT`.
- Primary keys and links/foreign keys get glyph affordances when fonts support them.
- Focused row uses `bg_raised`; alternate rows use `bg`/`bg_soft`.
- Focused cell gets an accent box/border and bold accent content.
- Row gutter shows right-aligned row numbers; focused row gutter uses accent.
- Nulls are faint/italic; blobs are purple/italic; booleans can be centered glyphs; numbers right-align; text left-align.
- Text truncation must respect Unicode display width.
- Column sizing should be content-aware and bounded. Sample rows, ignore outliers, cap huge text, preserve minimum header readability, and allow horizontal scrolling when columns do not fit.
- Extra horizontal space should go to text/blob columns before numeric columns.

## Command Palette

A command palette is the escape hatch for discoverability.

- Open with Ctrl-P / Ctrl-Shift-P.
- Center near the upper third, not dead center if it blocks context.
- Use fuzzy matching.
- Highlight matched characters with `accent` and bold.
- Include command labels and object-specific commands like `Switch Table: users`.
- Executing a command should close the palette then dispatch a normal domain message.

## Feedback, Errors, and Confirmations

- Use toasts for transient outcomes; cap the stack and expire messages.
- Success/info can be short-lived; errors last longer.
- Toasts appear top-right and use solid semantic backgrounds with `bg` foreground.
- Confirmation prompts should be visually distinct, near the lower right, and time out or be explicitly cancellable.
- Destructive actions require `y`/`n` or Esc.
- Read-only violations must surface as errors, not silent no-ops.

## Icons and Fonts

Nerd Font icons are allowed only behind a config flag or capability check. Always provide plain-text fallbacks (`[T]`, `[V]`, `[I]`, `?`). Unicode box drawing is part of the visual language; use it intentionally and test terminals that may render narrow/wide glyphs differently.

## Implementation Checklist

When creating or changing a TUI:

1. Define or reuse a single theme/palette; map all colors through it.
2. Define app modes, focus panes, popup kinds, and messages before rendering details.
3. Build stable layout rectangles; store those needed for mouse hit-testing.
4. Render statusbar first or reserve it first; keep it truthful and contextual.
5. Implement keyboard paths for every action.
6. Add mouse hit-tests for the same actions.
7. Add/update help text for every shortcut and mouse behavior.
8. Make scrolling clamp-safe, viewport-aware, and large-data-ready.
9. Add tests for state transitions, hit-testing, scroll math, and conditional hints.
10. Verify with formatting, linting, and tests provided by the repository.

## Anti-Patterns

Do not build:

- Random color palettes, neon cyan accents, gradients, glows, or theme generators.
- Decorative panels that do not map to focus or information architecture.
- Mouse-only controls.
- Hidden shortcuts not present in help/status.
- Static help that lies after shortcuts change.
- Popups without Esc behavior.
- Infinite scrolling that loses focus position.
- Render loops that redraw constantly when nothing changed.
- Broad silent fallbacks for terminal events, IO errors, or write failures.
- Fancy animation for data tools.

## sqv Reference Points

Use these files as pattern references in this repository:

- `src/theme.rs` - sacred warm palette.
- `src/ui/statusbar.rs` - contextual mode, position, preview, and action hints.
- `src/ui/popup/help.rs` - grouped scrollable help.
- `src/ui/tabbar.rs` - custom tabs, active joins, close hit-testing.
- `src/ui/sidebar.rs` - focused panel border, collapsible sections, sidebar scrollbar.
- `src/ui/mod.rs` - global split layout, statusbar reservation, popup/toast layering.
- `src/grid/mod.rs` - grid rendering, focus border, type-aware cells, scrollbar drag math.
- `src/grid/layout.rs` - content-aware column sizing.
- `src/grid/virtual_scroll.rs` - virtual window and prefetch thresholds.
- `src/grid/alphabet_rail.rs` - sorted text jump rail.
- `src/app.rs` - message/update model, keyboard shortcuts, mouse routing, popup modes.
- `src/main.rs` - 33ms tick, dirty rendering, crossterm event stream.

The goal is not to copy `sqv` mechanically. The goal is to preserve its taste: warm, sharp, useful, discoverable, and fast.
