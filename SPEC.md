# Implementation Spec

This repository implements the supplied projectional Markdown editor spec in milestones.

Current milestone coverage:

- Ratatui app shell with one rendered document panel.
- Top tabs, right scrollbar, bottom status bar, help popup, fuzzy command palette with typed-command compatibility, and search prompt.
- Rope-backed Markdown source buffer.
- Semantic component graph for headings, paragraphs, lists, task items, code blocks, tables, blockquotes, alerts, HTML, footnotes, math, diagrams, rules, and gaps.
- Direct rendered editing through source patches, undo, redo, and source export.
- Source-backed text selection for Shift+Arrow, mouse drag, Ctrl+A, selected-text replacement, and selected-range inline style toggles.
- Search prompt for forward/backward match navigation.
- Render-time document hit-testing for mouse cursor placement, drag selection, task checkbox toggles, and scrollbar click/drag-to-jump.
- Gap-target insert menu for paragraphs, headings, code blocks, quotes, alerts, lists, tables, images, link references, math, and diagrams.
- Projectional component actions for task checkbox toggles, current-word link insertion, fenced code language edits, table row/column insertion, and table row/column removal.
- Rich link prompt for label, URL, title, and reference definition fields.
- Source-backed table cell cursor navigation for Tab/Shift+Tab, Enter to the next row with row creation at the end, and Backspace removal of empty rows.
- Source-backed pipe table alignment preservation and Ctrl+Alt+Left/Right alignment edits.
- Table row/column selection actions, Delete removal for selected targets, and focused-table normalization.
- Source-backed heading text/level editing through F2 and Ctrl+Alt+1..6.
- Session data model and TOML persistence for file tabs, active tab, source cursor byte, scroll offset, and pinned flag.
- Dirty quit and dirty tab close confirmation prompts.
- Native clipboard integration for copying focused code block bodies, with OSC 52 fallback.
- Current-word inline formatting toggles for bold, italic, and inline code.
- Current-word inline formatting toggles for bold, italic, strikethrough, and inline code.
- Selection-triggered keyboard and mouse inline style palette for existing bold, italic, strikethrough, inline code, and link actions.
- Footnote and missing link-reference diagnostics plus key workflows for jumping to definitions, creating missing definitions, and renaming focused labels.
- Markdown image component parsing, broken local image diagnostics, rendered image cards, and F2 editing for alt/source/title.
- Visual task checkbox atoms, framed blockquote rendering, GFM strikethrough styling, emoji shortcode substitution where resolvable, and accent-styled table headers.
- Syntect-backed fenced code highlighting with a bounded cache keyed by language and body content.
- Fenced code block focus cycle for language, body, and Copy action targets.
- Tree-sitter-md parse tree cache with `InputEdit` reuse after source patches.
- Kitty Graphics Protocol capability detection, upload/delete command builders, and image-id cache keys.
- H1/H2 render graphic requests, deterministic heading raster fallback, capability-gated KGP upload/cache integration, and Unicode placeholder cells for scroll-safe placement.
- Terminal compatibility matrix coverage for Ghostty, Kitty, WezTerm, iTerm2, and basic ANSI fallback.
- Local image graphic requests with file-relative path resolution, supported-format decoding, preview-bounded RGBA scaling, capability-gated KGP upload, Unicode placeholder cells, and text-card fallback for remote or unsupported images.
- Math and diagram blocks queue nonblocking preview requests, keep text fallback while cold, and render cached KGP placeholder previews once warm.
- Optional external preview renderer commands for math and diagrams, configured by `MDTUI_MATH_RENDERER`, `MDTUI_DIAGRAM_RENDERER`, `MDTUI_PREVIEW_RENDERER`, or label-specific `MDTUI_PREVIEW_<LABEL>_RENDERER`; source is piped on stdin, size and theme metadata are exposed through args/env, decoded rasters are cached by source/size/theme, and deterministic placeholder previews remain the failure fallback.
- Bounded render-window output for visible rows plus overscan, with a document-persisted prefix-style block row index, exact-height skipping for fully offscreen components, and intra-block row virtualization for large fenced code blocks, lists, tables, and boxed blocks.
- Deterministic large-document render-window regression tests, inline edit fuzz regressions, and an ignored benchmark matrix for paragraphs, code, lists, tables, boxed math, and cached scrolls.
- GFM oracle path through Comrak.
- Event-driven redraw.
- Ghostty-style warm dark palette.
- Correct terminal background protocol: `OSC 11` set and `OSC 111` reset using `ST`.

Planned follow-up milestones:

- In-terminal manual validation on real Ghostty/Kitty/WezTerm/iTerm2 sessions beyond automated capability-matrix tests.
- Renderer-specific adapters for common math and diagram tools beyond the generic external-command hook.
