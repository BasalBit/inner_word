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
| `crates/writer-ui` | GPUI | Theme tokens (light/dark) and shared widgets (text field, picker, modal, context menu, tooltip, toasts, key hints, icons). |
| `crates/writer-editor` | GPUI | The rich text editor: `Editor` entity, custom element, layout (flow + page view), input, and the seams the crates below plug into. |
| `crates/writer-editor-tools` | GPUI | Contextual editor overlays: format menu, insert menu, link popover, font picker, find and replace, the context menu of the text, the floating format bar over a mouse selection. |
| `crates/writer-vim-mode` | GPUI | Glue that attaches the `writer-vim` engine to an `Editor` (key interception, caret style, command line, status). |
| `crates/writer-comments` | GPUI | Comments panel: threads, composer, replies, resolve, navigation. |
| `crates/writer-spell` | GPUI | Spell checking: the spell service behind a trait (the macOS system checker `NSSpellChecker`, and a fake built from word lists for the tests of every crate) and the `SpellChecker` entity that attaches to an `Editor` and keeps its spelling marks current (what is on screen first, the rest in slices while idle, one paragraph per edit), with the rows for the context menu, the actions and the queries the Vim keys need. |
| `crates/writer-library` | GPUI | `Library` model (the single owner of the store, emits change events) and the library sidebar panel. |
| `crates/writer-app` | GPUI bin `the-writer` | Window and workspace shell, command palette, quick open, library search, settings (the app's and the open document's), version history, shortcuts overlay, status bar, autosave, export flow, the native menu bar, the menus of the title and the status bar, the single-instance guard, the spelling settings and the language picker. Mounts everything above. |
| `scripts/` | shell | `.app` bundling and `.dmg` creation (unsigned / ad-hoc signed). |
| `docs/gpui-notes.md` | notes | Verified GPUI API notes for this exact gpui version. Read it before writing GPUI code. |

Dependency direction: `app → {editor-tools, vim-mode, comments, spell, library, export}`,
`{editor-tools, vim-mode, comments, spell} → editor → {ui, core}`, `vim-mode → vim → core`
(the Vim crates do not depend on `spell`: the app connects the two, `VimMode::set_spelling`),
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

Steps: `wait:<ms>`, `keys:<keystrokes>` (the key `;` is written `\;`: `keys:cmd-\;`), `type:<text>`, `action:<name>`, `menu:<name>` (opens
a menu of the workspace at its control: `title`, `save`, `statistics`, `page`, `limit`),
`choose:<label>` (chooses that item of the open context menu, as a click would),
`bar` (shows the format bar over the selection, as a selection made with the mouse does;
`bar:bold`, `bar:highlight`, `bar:turn-into`, `bar:extra-0`, `bar:more` press that button of
it), `resize:<w>x<h>`, `theme:<light|dark|system>`, `screenshot:<path>`, `info`, `state` (one
line: document, save state, what the last save cost, focus, modal, the open context menu,
Vim, tool, the format bar, comments, spelling, status bar), `dump`, `log:<text>`, `quit`, and for measuring
`book:<blocks>` (creates and opens a generated document, and says how long it took until
the text on screen was checked for spelling), `savecost[:<rounds>]` (every part of a
synchronous save, timed), `limitcost[:<rounds>]` (the character-limit count),
`typecost:<text>` and `keycost:<keystrokes>` (press the keys at a writer's pace and time
each: dispatch, the frame after it, how long the main thread kept a timer waiting, the
spell checker's slices) and `stall:<ms>` (watches the main thread without pressing
anything). Useful
lines: open a document `keys:cmd-p; type:chap 2; keys:enter`; a palette command
`keys:cmd-shift-p; type:version history; keys:enter`; the settings of the open document
`keys:cmd-shift-p; type:document settings; keys:enter`; a context menu `keys:ctrl-enter`
(in the text, the library, the comment list or a text field), then `choose:Add Comment`;
the format bar `keys:shift-right shift-right; bar; bar:bold`.
There are no mouse steps (GPUI cannot be handed pointer events from outside); mouse
behaviour is tested headlessly with the test context's `simulate_*` calls.

**One running copy per data directory.** A launch takes an exclusive lock on
`instance.lock` in its data directory and keeps it until the process ends
(`crates/writer-app/src/single_instance.rs`). A second launch on the same directory does
not open the library: it shows a window that says another copy has it (and prints the same
to stderr), with a button that brings the other copy to the front. So two scripted runs
must not share a `THE_WRITER_DATA_DIR`; the second one's `state` step fails with "the window
does not show a workspace". A build from before the guard takes no lock and is not seen.

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

What spell checking costs in a book (measured for 0.2.0: the text on screen is checked
about 20 ms after the first frame, the whole book in about 20 s of idle time in slices of
4 to 6 ms, a key reaches the screen in 6 to 8 ms while the pass waits for the writer):

```bash
THE_WRITER_AUTOMATION="wait:1500; book:10000; stall:3000; typecost:Teh quick brown fox.; keycost:pageup pageup pageup; stall:20000; state; quit"
# run it a second time with "keys:cmd-shift-p; type:check spelling while typing; keys:enter" first:
# single waits of 20 to 40 ms turn up with the checker off too (docs/gpui-notes.md, Visual QA)
```

The story test (`crates/writer-app/tests/suite/story.rs`) walks through the whole product
headlessly; `tests/suite/keymap.rs` checks every key of the tables below against the
assembled keymap; `tests/suite/saving.rs` holds the rules of saving in the background;
`tests/suite/mouse.rs` does everything with simulated pointer events (the menu bar, the
menus of the title, the status bar, the text and the library rows, modals);
`tests/suite/context_menus.rs` right-clicks every kind of object in the assembled workspace
(a document row, a folder row, the empty library, a selection, a link, a comment thread, a
text field, the title, a status bar item), clicks an item of each menu, and holds the format
bar and `shift-f10`; `tests/suite/findings.rs` keeps what the testers found from coming back;
`tests/suite/spelling.rs` holds spell checking in the assembled app, with the fake checker.

**Tests never ask the Mac's dictionary** (its answers differ between macOS versions, and
its learned words are shared with every other app). A workspace checks spelling only when
the app was given a spell service: `main` installs the system one
(`writer_app::spelling::use_system_service`), a test installs
`writer_spell::FakeSpellService` with `writer_app::spelling::set_service` before it opens a
workspace, and without one there is no checker at all. A scripted run of the real binary
does use the system checker: do not choose "Learn Spelling" there except on an invented
word that is unlearned again in the same run.

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

1. **Keyboard and mouse are equals.** Every feature must be reachable and completable
   without a mouse: a shortcut, the command palette, or keyboard navigation inside whatever
   is open. And every feature must be reachable and completable with the mouse alone, for
   people who are not comfortable driving everything from the keyboard: a right-click
   context menu on every object (library rows and empty space, text and selections, links,
   comment threads, text fields, the title, status bar items), the native menu bar for
   app-level and formatting commands, and a small visible control wherever a right-click
   would not be discovered. Context menus also open from the keyboard (`ctrl-enter`, and
   `shift-f10` as a second key).
   Every overlay closes with `escape` or a click outside and returns focus to where it came
   from.
2. **Few menus in the window.** No ribbon or permanent formatting toolbar. Formatting is
   contextual: shortcuts, Markdown-style input rules, the `cmd-.` format menu, the `/`
   insert menu, right-click context menus, the command palette, and the floating format
   bar: a small bar that comes up by itself over a selection made with the mouse (a drag, a
   double or triple click, a shift-click), never for a keyboard selection, never takes the
   keyboard, and can be switched off in the settings ("Format bar").
3. **One source of truth for shortcuts.** Bindings are registered through GPUI key bindings;
   the shortcuts overlay and palette hints are generated from that registry, not hand-written.
4. **Never lose text.** Documents autosave (debounced) and on close/quit. No save button;
   "Save Now" (`cmd-s`, the save state in the status bar, the File menu) saves at once for
   whoever wants to be sure. `SIGTERM`, `SIGHUP` and `ctrl-c` in the terminal quit the way
   `cmd-q` does, saving first (`crates/writer-app/src/signals.rs`). Two copies of the app
   never have one library open (the single-instance guard, see "Build and test").
   The autosave of a library on disk is written by a background thread through a second
   connection, so a book does not stutter when it is saved; the flush on switching
   documents, closing and quitting is synchronous and waits for a save in flight, and an
   older copy is never written after a newer one (`crates/writer-app/src/workspace/saver.rs`).
   The library keeps version snapshots; "Version history" in the palette brings one back.
   The title above the text is the title of the text as it is now (it follows every edit,
   undo included, `Workspace::mark_dirty`); the library's row follows with the save.

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

Without a key of their own, from the command palette (and the menu bar): "Version history"
(the snapshots of the open document with a preview; `up` / `down` choose, `space` scrolls the
text, `enter` restores, after keeping the current text as a version), "Document settings"
(the settings modal at its "This document" section: page view, paper including a custom
size, orientation, margins, font, text size, line spacing, paragraph spacing; lengths are
typed, `20mm`, `0.75in`), "Rename document", "Reveal in library" and "Welcome document"
(opens it; written again when it is gone). In the settings `up` / `down` move, `left` /
`right` change, `space` switches; the "Writing" section has the switch for the format bar
and the two spelling rows, the "Status bar" section chooses the character limit and which
counts are shown.

Spelling: misspelled words get a wavy red underline (not the word being typed, not code).
A right-click on a marked word, or `ctrl-enter` / `shift-f10` with the caret in it, starts
the text menu with up to five suggestions (or "No Guesses Found"), "Learn Spelling" (the
Mac's own learned words, shared with other apps) and "Ignore Spelling" (this document,
until it is closed). All of it is in the palette and under Edit > Spelling in the menu
bar. Settings (stored in the library like the others): `spelling.check_while_typing`
(bool, on by default; "Check spelling while typing") and `spelling.language` (`"auto"` or
a system language identifier such as `"en_GB"`; "Spelling language": Automatic detects the
language of each paragraph). A document can carry a language of its own
(`DocumentSettings::spelling_language`, "Spelling language" in the "This document"
settings: follow the app setting, Automatic, or a language), which overrides the app's and
becomes the proofing language of a DOCX export. `left` / `right` step through the
languages; `enter` or `space` on either row, or the palette command "Spelling language"
(for the open document), opens a searchable list. The list is the Mac's
(`writer_spell::language_choices`): the user's own language first, the others by name; a
Mac left on "Automatic by Language" reports nearly every language as preferred, which is
then not taken as a choice. The checker's plain `en` is American English and is named so.
Compact editors (the comment composer) and read-only ones are not checked.

Global (work anywhere in the window):

| Keys | Action |
|---|---|
| `cmd-shift-p` | Command palette |
| `cmd-p` | Quick open document |
| `cmd-n` / `cmd-shift-n` | New document / new folder |
| `cmd-0` / `cmd-1` / `cmd-2` | Go to library / text / comments |
| `cmd-opt-s` | Show or hide the library sidebar |
| `cmd-opt-right` / `cmd-opt-left` | Widen / narrow the library sidebar |
| `cmd-shift-f` | Search the whole library |
| `cmd-shift-s` | Export (Markdown, DOCX) |
| `cmd-s` | Save now (the text is saved as it is written anyway) |
| `ctrl-enter` or `shift-f10` | Context menu of what has the keyboard: the text, a library row, a comment thread, a text field |
| `cmd-,` | Settings |
| `cmd-/` | Keyboard shortcuts overlay |
| `cmd-opt-p` | Turn page view on or off |
| `cmd-opt-v` | Turn Vim mode on or off |
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
| `cmd-;` / `cmd-shift-;` | Next / previous misspelling (both wrap around the document) |
| `cmd-z` / `cmd-shift-z` | Undo / redo |
| `cmd-a` `cmd-c` `cmd-x` `cmd-v` `cmd-shift-v` | Select all, copy, cut, paste, paste as plain text |

Plus the standard macOS text-navigation keys (arrows, `alt`/`cmd` + arrows, `shift` to
extend, `alt-backspace`, `cmd-backspace`, and the `ctrl-a/e/f/b/n/p/k/d/h` Emacs set).

Keymap facts that bite (details in `docs/gpui-notes.md`):

- **The context menu has two keys.** `ctrl-enter` is the one macOS itself uses and the one
  key hints show; `shift-f10` is bound beside it everywhere (the text, the library, the
  comment list, text fields; bound first, so hints keep showing `ctrl-enter`), because a
  real `ctrl-return` posted to a background window never reached GPUI (keypad enter did)
  and nobody has pressed it on a keyboard in an active window yet. The shortcuts overlay
  names the second key in the row ("Context menu (also ⇧F10)", `shortcuts::EVERY_KEY`).
- **Panel keys that are easy to miss**: in the library `alt-right` / `alt-left` open or close
  every folder (of the selected folder, or all of them) and a click on a folder row opens
  or closes it; in the comment list `cmd-c` copies the comment's text; `cmd-opt-f` on an
  open find bar with a query moves to the replace field; `escape` in the text closes the
  format bar before the find bar.
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
  Spelling: `]s` / `[s` next / previous misspelling, `z=` the suggestions as a menu at the
  caret (`1z=` takes the first), `zg` learns the word and `zug` unlearns it
  (`Workspace::connect_vim_spelling`).
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
- **Commands for the text work wherever the keyboard is.** With a document open the
  workspace root takes every formatting, block, insert and tool command that nothing nearer
  to the keyboard handled and passes it on to the text, which takes the keyboard
  (`Workspace::forward_to_text`). That is what makes them work from the palette and the menu
  bar while the keyboard is in the library, the comment list or a modal. The Edit menu's
  standard items are not passed on. A menu bar item that is enabled also answers its key
  through AppKit when GPUI has no binding for it where the keyboard is, so `cmd-b` in the
  library reaches the text too (read from the two key paths, not tried on a keyboard).
- **A modal has the keyboard or it closes.** When the keyboard leaves an open modal (a
  window-wide key of another crate, a menu bar command) the workspace closes the modal and
  leaves the keyboard where it went; the comment keys and the view toggles close it first.
- **The key beside a menu bar item** is not the one key hints show: GPUI takes the first
  binding of the action that holds in a made-up `Workspace Pane Editor` context. The keys
  the menu bar should show are therefore bound once more in the `Pane` context, which no
  element has (`menus::menu_key_bindings`, added before all other bindings; they never run).
  A key with `enter` in it cannot be given to AppKit: those items show no key.

## Mouse

Every feature is reachable with the mouse alone (product rule 1). How it fits together:

- **The menu bar** (`crates/writer-app/src/menus.rs`) is the complete map: File, Edit (with
  Spelling), Format (with Turn Into and Align), Insert, View (with Theme), Help. Every item
  is an existing action; it is enabled when the action has a handler on the way from the
  keyboard. GPUI's menu items carry no state, so the check mark of "Check Spelling While
  Typing" is set on the native item (`menus::set_check_mark`, only in the real app).
- **Context menus** are `writer_ui::ContextMenu`: one widget, drawn by the toast layer, which
  never takes the keyboard. A right-click, `ctrl-enter` or a small "more" control opens one.
  A menu is composed where its object lives: the text menu in `writer-editor-tools` (the
  editor only emits `EditorEvent::ContextMenuRequested`), library rows in `writer-library`,
  comment threads in `writer-comments`, the Vim chip in `writer-vim-mode`, text fields in
  `writer-ui`. Each offers a hook through which the app adds its items
  (`Workspace::extend_the_menus_of_other_crates`: Add Comment and Go to Comment in the text
  menu, Export and Version History on library rows). The text menu has a second hook for
  what goes first (`EditorTools::set_text_menu_leading_extension`): the spell checker's
  suggestions on a marked word.
- **The format bar** (`crates/writer-editor-tools/src/format_bar.rs`) comes up 150 ms after
  the left button is released on a selection the pointer made: bold, italic, underline,
  strikethrough, inline code, highlight and text colour (menus), link, turn into (a menu),
  the app's "Add Comment" (`EditorTools::set_format_bar_extension`, set beside the text
  menu's hook) and "more" (the format menu). Each button runs what its key runs and keeps
  the selection and the bar. `editor.format_bar` (the "Format bar" switch in the settings)
  turns it off through `EditorTools::set_format_bar_enabled`. It is not a `ToolKind`.
- **The app's own menus** (`crates/writer-app/src/workspace/menus.rs`): the title above the
  text (right-click, or the chevron beside it) and the items of the status bar (the save
  state, the counts, the page indicator, the character limit; a click or a right-click).
- **Modals** close on `escape`, a click on the backdrop, or their close control; rows and
  controls take clicks.
- An item that is a command is the command's action, so it shows the key and does what the
  key does. Controls made for the pointer do not move the keyboard when pressed.

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
