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
| `crates/writer-editor` | GPUI | The rich text editor: `Editor` entity, custom element, layout (flow + page view), input, and the seams the crates below plug into. |
| `crates/writer-editor-tools` | GPUI | Contextual editor overlays: format menu, insert menu, link popover, font picker, find and replace. |
| `crates/writer-vim-mode` | GPUI | Glue that attaches the `writer-vim` engine to an `Editor` (key interception, caret style, command line, status). |
| `crates/writer-comments` | GPUI | Comments panel: threads, composer, replies, resolve, navigation. |
| `crates/writer-library` | GPUI | `Library` model (the single owner of the store, emits change events) and the library sidebar panel. |
| `crates/writer-app` | GPUI bin `the-writer` | Window and workspace shell, command palette, quick open, library search, settings (the app's and the open document's), version history, shortcuts overlay, status bar, autosave, export flow, menu bar. Mounts everything above. |
| `scripts/` | shell | `.app` bundling and `.dmg` creation (unsigned / ad-hoc signed). |
| `docs/gpui-notes.md` | notes | Verified GPUI API notes for this exact gpui version. Read it before writing GPUI code. |

Dependency direction: `app → {editor-tools, vim-mode, comments, library, export}`,
`{editor-tools, vim-mode, comments} → editor → {ui, core}`, `vim-mode → vim → core`,
`library → {store, ui}`, `export → core`. Never the reverse. Every internal crate has an
entry in the root `[workspace.dependencies]`; depend on one with `writer-x.workspace = true`.

## Build and test

```bash
cargo build -p writer-app            # debug build
cargo run -p writer-app              # run the app
cargo test -p <crate>                # per-crate tests (GPUI crates)
CARGO_TARGET_DIR=target-pure cargo test -p writer-core   # pure crates use target-pure
THE_WRITER_DATA_DIR=/some/tmp cargo run -p writer-app   # run against a throwaway library
scripts/bundle.sh                    # release .app + .dmg in dist/
```

The real library lives in `~/Library/Application Support/The Writer/`. Tests and manual
QA must set `THE_WRITER_DATA_DIR` to a temp directory and never touch the real one.

Scripted runs of the real app (visual QA; grammar in `crates/writer-app/src/automation.rs`,
recipe in `docs/gpui-notes.md`):

```bash
# THE_WRITER_SEED_DEMO=1: a few folders and documents in an empty library (optional)
# THE_WRITER_EXPORT_DIR:  file exports are written there, without the save panel (optional)
THE_WRITER_DATA_DIR=/some/tmp \
THE_WRITER_SEED_DEMO=1 \
THE_WRITER_EXPORT_DIR=/some/tmp/out \
THE_WRITER_AUTOMATION="wait:1200; keys:cmd-n; type:Hello; state; screenshot:/some/tmp/a.png; quit" \
./target/debug/the-writer
```

Steps: `wait:<ms>`, `keys:<keystrokes>`, `type:<text>`, `action:<name>`, `resize:<w>x<h>`,
`theme:<light|dark|system>`, `screenshot:<path>`, `info`, `state` (one line: document, save
state, what the last save cost, focus, modal, Vim, tool, comments, status bar), `dump`,
`log:<text>`, `quit`, and for measuring `book:<blocks>` (creates and opens a generated
document), `savecost[:<rounds>]` (every part of a synchronous save, timed) and
`limitcost[:<rounds>]` (the character-limit count). Useful lines: open a document
`keys:cmd-p; type:chap 2; keys:enter`; a palette command `keys:cmd-shift-p; type:version
history; keys:enter`; the settings of the open document `keys:cmd-shift-p; type:document
settings; keys:enter`.

A scripted run stays in the background and draws its own frames, so it works while the
window is covered. Do not script anything that writes the system clipboard (copy, cut,
"Copy as Markdown"): it is the user's clipboard. A scripted run does not write user defaults
either (the accent-menu default below) unless `THE_WRITER_USER_DEFAULTS=1` is set. The window
manager on this machine sometimes tiles the window and then ignores `resize:`; check with
`info`.

Measure in a release build (`cargo build --release -p writer-app`, then
`./target/release/the-writer` with the same variables). What an autosave of a book costs:

```bash
THE_WRITER_AUTOMATION="wait:1500; book:10000; wait:3000; savecost:7; type:a; wait:1600; state; quit"
# savecost: the synchronous save, part by part (about 70 ms, the library write is 60 of them)
# state:    last_save=[main 0.7 ms, background 90 ms, ...] for the autosave after "a"
```

The story test (`crates/writer-app/tests/suite/story.rs`) walks through the whole product
headlessly; `tests/suite/keymap.rs` checks every key of the tables below against the
assembled keymap; `tests/suite/saving.rs` holds the rules of saving in the background.

## Working rules

- **Two target directories, no more.** Disk on this machine is nearly full. Everything that
  depends on GPUI builds in the default `target/`. The pure crates (`writer-core`,
  `writer-store`, `writer-export`, `writer-vim`) build and test with
  `CARGO_TARGET_DIR=target-pure` so they do not queue behind GPUI builds. Do not create other
  target dirs, git worktrees, or copies of the repo. Do not run `cargo clean`.
- **Test binaries are expensive.** Every test binary or example that links GPUI costs about
  150 MB. Keep at most one integration-test binary per GPUI crate (`tests/main.rs` with
  submodules) or use unit tests; add an example only when your task says so.
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
   The autosave of a library on disk is written by a background thread through a second
   connection, so a book does not stutter when it is saved; the flush on switching
   documents, closing and quitting is synchronous and waits for a save in flight, and an
   older copy is never written after a newer one (`crates/writer-app/src/workspace/saver.rs`).
   The library keeps version snapshots; "Version history" in the palette brings one back.

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

`opt` below is the option key; in GPUI keystroke strings it is spelled `alt`
(`KeyBinding::new("cmd-alt-s", ..)`; `opt` does not parse). The shortcuts overlay (`cmd-/`)
is generated from the keymap and lists everything, grouped by where the keyboard has to be
("Anywhere", "In the text", "In the find bar", "In the library", "In the comment list", ...),
including the keys of the panels and overlays that are not repeated here.

Without a key of their own, from the command palette: "Version history" (the snapshots of the
open document with a preview; `up` / `down` choose, `space` scrolls the text, `enter`
restores, after keeping the current text as a version) and "Document settings" (the settings
modal at its "This document" section: page view, paper including a custom size, orientation,
margins, font, text size, line height, paragraph spacing; lengths are typed, `20mm`, `0.75in`).
In the settings `up` / `down` move, `left` / `right` change, `space` switches.

Global (work anywhere in the window):

| Keys | Action |
|---|---|
| `cmd-shift-p` | Command palette |
| `cmd-p` | Quick open document |
| `cmd-n` / `cmd-shift-n` | New document / new folder |
| `cmd-0` / `cmd-1` / `cmd-2` | Focus library / focus editor / focus comments |
| `cmd-opt-s` | Show or hide the library sidebar |
| `cmd-opt-right` / `cmd-opt-left` | Widen / narrow the library sidebar |
| `cmd-shift-f` | Search the whole library |
| `cmd-shift-s` | Export (Markdown, DOCX) |
| `cmd-,` | Settings |
| `cmd-/` | Keyboard shortcuts overlay |
| `cmd-opt-p` | Toggle page view |
| `cmd-opt-v` | Toggle Vim mode |
| `cmd-opt-c` | Show or hide comments |
| `cmd-opt-m` | Add comment on selection (or on the word at the caret) |
| `cmd-opt-]` / `cmd-opt-[` | Next / previous comment |
| `cmd-w` / `cmd-q` | Close the window / quit |

`cmd-opt-c` has a second meaning in one place: inside the find bar it switches "case
sensitive" (the bar shows the hint), and `cmd-opt-w` "whole word".

Editor:

| Keys | Action |
|---|---|
| `cmd-b` `cmd-i` `cmd-u` | Bold, italic, underline |
| `cmd-shift-x` / `cmd-e` / `cmd-shift-h` | Strikethrough / inline code / highlight |
| `cmd-k` / `cmd-shift-k` | Add or edit link / remove link |
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
| `/` at the start of a block or after a space | Insert menu |
| `cmd-f` `cmd-opt-f` `cmd-g` `cmd-shift-g` | Find, replace, next, previous |
| `cmd-z` / `cmd-shift-z` | Undo / redo |
| `cmd-a` `cmd-c` `cmd-x` `cmd-v` `cmd-shift-v` | Select all, copy, cut, paste, paste as plain text |

Plus the standard macOS text-navigation keys (arrows, `alt`/`cmd` + arrows, `shift` to
extend, `alt-backspace`, `cmd-backspace`, and the `ctrl-a/e/f/b/n/p/k/d/h` Emacs set).

Keymap facts that bite (details in `docs/gpui-notes.md`):

- **Shift plus a digit or punctuation** reaches GPUI as the shifted character without the
  shift flag (`cmd-shift-7` is `cmd-&` on a US keyboard). Bind both spellings, the readable
  one last so key hints show it (see `crates/writer-editor/src/actions.rs`).
- **A binding without a context ranks like the deepest context**, not like the shallowest:
  it beats every binding of an outer context on the same key, and among bindings at the
  focused element's own level the one added later wins. `writer_app::init` therefore adds
  the app's bindings first, and a crate that needs a window-wide key inside one of its
  views binds it at the level of the focused element (the find bar does, for `cmd-opt-c`).
- **Who takes a key in the text**: an open insert menu, then Vim mode (it lets every `cmd`
  chord through), then the bindings from the editor outwards, then text input. Popups with
  a field (format menu, link, font, find bar) and modals have the keyboard themselves.
- **Vim mode**: `:w` saves at once, `:q` closes the document (the window stays), any other
  ex name is looked up among the palette's commands (`:togglepageview`, `:export`).
- **Held keys with Vim mode on** repeat instead of opening the macOS accent menu: the app
  stores `ApplePressAndHoldEnabled = false` in its own user defaults while Vim mode is on
  and removes it when it is off (`crates/writer-app/src/press_and_hold.rs`). macOS reads
  that default once per launch, so a change made while the app runs acts from the next
  launch; the settings say so. In Normal mode a held key should repeat either way, because
  Vim takes the key before the text input system sees it (read from GPUI's key path, not
  tried on a keyboard).
- **The Edit menu** (undo, redo, cut, copy, paste, select all) carries the editor's actions.
  The workspace turns them into the text field's own when a field has the keyboard, so the
  menu works in the palette, a rename field, the find bar and the settings.

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
