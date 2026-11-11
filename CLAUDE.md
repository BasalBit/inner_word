# The Writer

A keyboard-first rich-text writing app for macOS (books, blog posts, social posts).
Rust + GPUI (the Zed team's UI framework, crates.io `gpui` 0.2.2). Storage is SQLite.

## Workspace layout

| Crate | Kind | Responsibility |
|---|---|---|
| `crates/writer-core` | pure Rust | Document model, headless editing engine (selection, commands, undo/redo), comments, input rules, search. No GPUI, no IO. |
| `crates/writer-store` | pure Rust | SQLite library: folders, documents, settings, full-text search. Does **not** depend on `writer-core`; document content is an opaque JSON string. |
| `crates/writer-export` | pure Rust | Markdown and DOCX exporters over `writer-core`. |
| `crates/writer-vim` | pure Rust | Vim emulation engine over `writer-core`'s editing engine plus a small visual-layout trait. |
| `crates/writer-ui` | GPUI | Theme tokens (light/dark) and shared widgets (text field, picker, modal, key hints, icons). |
| `crates/writer-editor` | GPUI | The rich text editor: `Editor` entity, custom element, layout (flow + page view), overlays, vim integration. |
| `crates/writer-app` | GPUI bin `the-writer` | Window, library sidebar, command palette, settings, comments panel, export wiring. |
| `scripts/` | shell | `.app` bundling and `.dmg` creation (unsigned / ad-hoc signed). |
| `docs/gpui-notes.md` | notes | Verified GPUI API notes for this exact gpui version. Read it before writing GPUI code. |

Dependency direction: `app → editor → {ui, vim, core}`, `app → {store, export, ui}`,
`export → core`, `vim → core`. Never the reverse.

## Build and test

```bash
cargo build -p writer-app            # debug build
cargo run -p writer-app              # run the app
cargo test -p <crate>                # per-crate tests
THE_WRITER_DATA_DIR=/some/tmp cargo run -p writer-app   # run against a throwaway library
scripts/bundle.sh                    # release .app + .dmg in dist/
```

The real library lives in `~/Library/Application Support/The Writer/`. Tests and manual
QA must set `THE_WRITER_DATA_DIR` to a temp directory and never touch the real one.

## Working rules

- **One `target/` directory.** Disk on this machine is nearly full. Do not create extra
  target dirs, git worktrees, or copies of the repo. Do not run `cargo clean`.
- **Cargo may block** with "waiting for file lock on build directory" while another build
  runs. That is normal; wait for it (use long command timeouts).
- **Stay in your crate.** Edit only the crate(s) you were assigned. If another crate needs a
  change, make the smallest additive change only if its owner is not currently working on it;
  otherwise report it. Compile errors in a crate you do not own are usually another change in
  flight: wait a minute and retry rather than "fixing" it.
- **Dependencies** go in your crate's own `Cargo.toml`. The root `Cargo.toml` changes only
  for dependencies shared by several crates.
- **Commits are incremental.** Commit each working milestone, limited to the paths you own:
  `git add <your paths> && git commit -m "<message>" -- <your paths>`. Never `git add -A`,
  never amend, rebase, reset, stash or check out files you do not own. If `index.lock` is
  busy, wait a few seconds and retry. Only commit code that compiles.
- **Done means verified.** No `todo!()` / `unimplemented!()` / placeholder behaviour left
  behind. `cargo test -p <crate>` passes and `cargo build -p writer-app` still compiles
  before you finish.
- UI text is plain and specific. No emoji in the UI.

## Product rules

1. **Keyboard first.** Every feature must be reachable and completable without a mouse:
   a shortcut, the command palette, or keyboard navigation inside whatever is open. The mouse
   works too, but is never required. Every overlay closes with `escape` and returns focus to
   where it came from.
2. **Few menus.** No ribbon or permanent formatting toolbar. Formatting is contextual:
   shortcuts, Markdown-style input rules, the `cmd-.` format menu, the `/` insert menu, and
   the command palette.
3. **One source of truth for shortcuts.** Bindings are registered through GPUI key bindings;
   the shortcuts overlay and palette hints are generated from that registry, not hand-written.
4. **Never lose text.** Documents autosave (debounced) and on close/quit. No save button.

## Document model in one paragraph

A document is an ordered list of blocks. A block has a stable id, a kind (paragraph,
heading 1–6, bullet / numbered / todo list item, quote, code block, divider, page break),
alignment, indent level, an optional block-level text size, and text with inline spans.
Inline attributes: bold, italic, underline, strikethrough, code, highlight, text colour,
font family, link, and comment anchors. A `\n` inside a block's text is a soft line break
(shift-enter); a code block is one block with multi-line text. Font *family* is inline;
font *size* is per block (GPUI shapes a line at a single size). Comment threads are anchored
to text through span attributes so they move with edits.

## Default keymap

Global (work anywhere in the window):

| Keys | Action |
|---|---|
| `cmd-shift-p` | Command palette |
| `cmd-p` | Quick open document |
| `cmd-n` / `cmd-shift-n` | New document / new folder |
| `cmd-0` / `cmd-1` | Focus library / focus editor |
| `cmd-opt-s` | Show or hide the library sidebar |
| `cmd-shift-f` | Search the whole library |
| `cmd-shift-s` | Export (Markdown, DOCX) |
| `cmd-,` | Settings |
| `cmd-/` | Keyboard shortcuts overlay |
| `cmd-opt-p` | Toggle page view |
| `cmd-opt-v` | Toggle Vim mode |
| `cmd-opt-c` | Show or hide comments |
| `cmd-opt-m` | Add comment on selection |
| `cmd-opt-]` / `cmd-opt-[` | Next / previous comment |

Editor:

| Keys | Action |
|---|---|
| `cmd-b` `cmd-i` `cmd-u` | Bold, italic, underline |
| `cmd-shift-x` / `cmd-e` / `cmd-shift-h` | Strikethrough / inline code / highlight |
| `cmd-k` | Add or edit link |
| `cmd-\` | Clear formatting |
| `cmd-opt-0`, `cmd-opt-1` … `cmd-opt-6` | Paragraph, heading 1–6 |
| `cmd-shift-7` / `cmd-shift-8` / `cmd-shift-9` | Numbered list / bullet list / todo list |
| `cmd-'` / `cmd-shift-'` | Quote / code block |
| `tab` / `shift-tab`, `cmd-]` / `cmd-[` | Indent / outdent |
| `alt-up` / `alt-down` | Move block up / down |
| `shift-enter` / `cmd-enter` | Soft line break / page break |
| `cmd-shift-enter` | Toggle todo checked |
| `cmd-shift-l` `cmd-shift-e` `cmd-shift-r` `cmd-shift-j` | Align left, centre, right, justify |
| `cmd-shift-.` / `cmd-shift-,` | Larger / smaller text |
| `cmd-t` | Font picker |
| `cmd-.` | Contextual format menu |
| `/` at the start of an empty block | Insert menu |
| `cmd-f` `cmd-opt-f` `cmd-g` `cmd-shift-g` | Find, replace, next, previous |
| `cmd-z` / `cmd-shift-z` | Undo / redo |
| `cmd-a` `cmd-c` `cmd-x` `cmd-v` `cmd-shift-v` | Select all, copy, cut, paste, paste as plain text |

Plus the standard macOS text-navigation keys (arrows, `alt`/`cmd` + arrows, `shift` to
extend, `alt-backspace`, `cmd-backspace`, and the `ctrl-a/e/f/b/n/p/k/d/h` Emacs set).

## Design language

Paper and ink. The chrome is quiet so the text is the loudest thing on screen.

- Light theme "Paper": warm off-white canvas, near-black warm ink, one accent (a red-pencil
  terracotta) used sparingly for focus, the caret, and active items. Dark theme "Night": the
  same idea inverted, warm charcoal rather than pure black.
- Document type defaults to a serif made for long reading (Charter, falling back to Iowan Old
  Style / Georgia), 17px, line height about 1.6. Code uses a monospace (SF Mono / Menlo). The
  interface uses the system UI font at 13px.
- Flow view: a single centred column about 700px wide. Page view: white sheets at real paper
  proportions with a soft shadow on a slightly darker desk, page numbers in the footer.
- Spacing on a 4px grid, hairline borders, small radii (4–8px), no gradients, no heavy shadows
  except the page sheets and floating overlays.
- Overlays (palette, pickers, format menu) share one look: a rounded panel, a query field on
  top, a list where the selected row is clearly marked, and key hints right-aligned.
