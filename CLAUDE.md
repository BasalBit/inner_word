# The Writer

A keyboard-first rich-text writing app for macOS (books, blog posts, social posts).
Rust + GPUI (the Zed team's UI framework, crates.io `gpui` 0.2.2). Storage is SQLite.

## Workspace layout

| Crate | Kind | Responsibility |
|---|---|---|
| `crates/writer-core` | pure Rust | Document model (an image is a block of it: the id of stored bytes, a pixel size, a width, a description; never the bytes), headless editing engine (selection, commands, undo/redo), comments, input rules, search. No GPUI, no IO. |
| `crates/writer-store` | pure Rust | SQLite library: folders, documents, settings, full-text search. Does **not** depend on `writer-core`; document content is an opaque JSON string, in which the store reads one token and nothing else: the id an image block names its bytes with (`"image":"<32 hex digits>"`, `Store::referenced_images`). The bytes of the images are in a **second file**, `images.db` beside `library.db` (made by the first image a library gets; `ImageFile` is that file by itself, for a thread of its own), stored once each under the hash of their bytes (`ImageId`) and deleted by nothing but the collector (`collect_unused_images`, `mark_unused_images`). **What a backup holds:** a daily backup (`backups/library-<ms>.db`, seven kept) is a copy of `library.db`, so it holds the documents only. The pictures have one copy, `backups/images.db`: brought up to date in the background, never before the window opens (`back_up_images`, a slice at a time; only pictures the copy does not have yet are copied; every run of the collector ends with it), read from when `images.db` does not have a picture (which is then written back), and a picture leaves it only when the collector deleted it and no kept backup names it. So a backup that is brought back finds its pictures in `images.db` or in the copy. **A damaged file is set aside, not believed**: an `images.db` or a copy that SQLite reads as damaged (`SQLITE_NOTADB`, `SQLITE_CORRUPT`) is moved aside as `images.db.damaged-<unix ms>` with its journal and made again from the other file (a read meanwhile answers "not there, ask the copy"; `images_moved_aside` lists what was set aside, and the app tells the writer of each once), and bytes whose hash is not their id are not trusted and are written again from the other file. A file of a newer version (`SchemaTooNew`) or an SQLite file that is not an images database (`NotALibrary`) is an error, and the file is left alone. **Pages**: a row of `documents` may be a page (`kind = 'page'`, `DocumentKind`, with the address it came from in `source_url`); its content is a manifest (`writer_page::Manifest` as JSON), in which the store reads one more token, `"file":"<32 hex digits>"` (`Store::referenced_files`), and its files are in a second table `files` of `images.db` (`FileId`, `put_file`, `file_bytes`, `ImageFile::put_file`), covered by the copy among the backups, the damaged-file handling and the collector as pictures are. `save_document` refuses a page and `save_page` a text (`WrongKind`), and `create_page` / `save_page` refuse a manifest that names a file `images.db` does not have (`FilesMissing`), writing nothing. `library.db` is at schema 4 since a library may hold pages (3: documents may hold images), `images.db` at its schema 2 (the `files` table): a version that does not know them refuses the library. |
| `crates/writer-image` | pure Rust | Image files: bytes as they come (a file, the clipboard, a drop, a path an agent names) to bytes as the library stores them (`prepare`, on ImageIO): a PNG, a JPEG or a GIF, upright, at most 4096 pixels on the long edge and 16 MB, without a GPS position. A file that already is that is stored as it came, byte for byte; HEIC, TIFF, BMP, WebP and AVIF are converted; an SVG, a PDF and what is no image are refused with a sentence (`PrepareError::sentence`). `fixtures` makes small picture files for the tests of every crate, so no binary file is in the repository. |
| `crates/writer-page` | pure Rust | The bundler of pages: an input (one `.html` file with the local files it names, its folder never walked; or a folder, taken whole) to the files a page is served from (`read_input`, `bundle`): every reference found by a small tokenizer of HTML, SVG, CSS and JS, every piece on the web fetched once through a `Fetcher` (`CurlFetcher`: `/usr/bin/curl`, public addresses only, every redirect followed and checked by the fetcher; `CachingFetcher`, `KnownPieces` for pieces the caller has, `MapFetcher` for tests) and stored under `_external/<host>/…`, the references rewritten to relative paths. The check as data (`CheckRequest`, `CheckReport`; `static_check`, the fake check of tests), the verdict (`prepare`: whole, or `PageError::Incomplete` naming every problem), the manifest, the folder export (`write_folder`), the normal form of a source address, the limits (`limits`: 32 MB a file, 64 MB a page, 2,000 files, 500 pieces from the web, 101 HTML files). No Objective-C, no GPUI, no network in its tests (a server on 127.0.0.1). |
| `crates/writer-web` | Rust without GPUI (WebKit, AppKit) | Everything that talks to WebKit: the view of an open page (`WebPage`, a `WKWebView` subclass above GPUI's view, served by a scheme handler `writer-page://` from a `PageSource` with ranges and a CSP on every answer, a content rule list that blocks the network, WebRTC and media devices off, a link opened only by the reader; its callbacks go to the owner's `PageEvents`, which only post), the offscreen check (`check`, `check_blocking`: every HTML file loaded with the network blocked, every blocked request named, the text read with what is not shown), the PDF of a page (`print_offscreen`, `print_blocking`, `WebPage::print_to_pdf`), `available()` (macOS 12.3 and every class and selector it sends, the private switches of WebRTC among them). Builds and tests in `target-pure`; `tests/webkit.rs` is a program of its own (`harness = false`). |
| `crates/writer-print` | Rust without GPUI (CoreGraphics, CoreText, PDFKit, AppKit) | Printing, after the layout: what a printed page is as data (`Page`: rectangles, strokes, rows of text with the place of every glyph, single-line labels, pictures; points from the top left corner), the PDF writer (`PdfWriter`: CoreText's glyphs at the places the editor's layout names, in the fonts the layout asked for, found the way GPUI finds them and never by a PostScript name; `Report::rows_off` counts the rows whose glyphs CoreText spaces otherwise than the layout, pair of clusters by pair inside the row, which is how a wrong face shows), a PDF reader for tests and for looking (`PdfFile`: page count, page sizes, the text of a page, where a word stands, a page as pixels or as a PNG), and the macOS print operation (`run_sheet`: the print panel as a sheet on a window; `run_save_job`: the same operation into a file, no panel and no printer; `check_selectors`: every class and selector the two send, asked of the runtime). Builds and tests in `target/` although it has no GPUI ("Working rules"). |
| `crates/writer-export` | pure Rust | Markdown, DOCX and plain-text exporters over `writer-core`, and the Markdown importer (Markdown text to a document or a fragment). Pictures come from whoever exports (`ImageSource`, `ExportSource::with_images`): a DOCX embeds them, Markdown names them `![description](image:<id>)` or, for a file, links picture files that are returned beside the text (`export_package`, `MarkdownImages::Files`), plain text has the description. The importer asks an `ImageResolver` which stored image each image of the Markdown is; `KnownImages` answers for the images a document already holds, with their width and alignment. |
| `crates/writer-vim` | pure Rust | Vim emulation engine over `writer-core`'s editing engine plus a small visual-layout trait. |
| `crates/writer-ui` | GPUI | Theme tokens (light/dark) and shared widgets (text field, picker, modal, context menu, tooltip, toasts, key hints, icons). |
| `crates/writer-editor` | GPUI | The rich text editor: `Editor` entity, custom element, layout (flow + page view), the zoom of the view, input, the print pass (`Editor::print_pass`, `src/print.rs`: the document as page view lays it out, whatever the window shows, handed out in slices as `writer_print::Page`s), images (`src/images.rs`: the picture of an image block decoded in the background at the size it is drawn, the outline and handles of the selected one, resize and move with the pointer, insert, paste, copy and drop; the bytes come through the `ImageProvider` an editor is given, `FakeImages` in tests), and the seams the crates below plug into. No crate draws a GPUI path (`paint_path` makes the window hold a texture its own size and a 4x MSAA copy until the next resize, about 400 MB on a large Retina window): the tick of a to-do is a sprite (`src/tick.rs`). A caret, the editor's or a text field's, blinks only in the active window (`docs/gpui-notes.md`). |
| `crates/writer-editor-tools` | GPUI | Contextual editor overlays: format menu, insert menu, link popover, font picker, find and replace, the context menu of the text and of an image, the popover for an image's description, the floating format bar over a mouse selection. |
| `crates/writer-vim-mode` | GPUI | Glue that attaches the `writer-vim` engine to an `Editor` (key interception, caret style, command line, status). |
| `crates/writer-comments` | GPUI | Comments panel: threads, composer, replies, resolve, navigation. |
| `crates/writer-spell` | GPUI | Spell checking: the spell service behind a trait (the macOS system checker `NSSpellChecker`, and a fake built from word lists for the tests of every crate) and the `SpellChecker` entity that attaches to an `Editor` and keeps its spelling marks current (what is on screen first, the rest in slices while idle, one paragraph per edit), with the rows for the context menu, the actions and the queries the Vim keys need. |
| `crates/writer-library` | GPUI | `Library` model (the single owner of the store, emits change events; `Library::change_outside` is the one way for code that works on the store itself, the app carrying out an agent's request, to make a change and have the same events emitted; `put_image` and `touch_image` store and keep the bytes of images and tell nobody; `create_page`, `save_page` and `put_file` for pages) and the library sidebar panel (a page's row has a globe; files dropped on the panel are `LibraryPanelEvent::DropPaths` with the folder under them). |
| `crates/writer-keymap` | pure Rust | The keyboard shortcuts as data: keystroke text in one normal form (a key bound in two spellings is one key), the defaults in order, the user's changes as the one stored value, the keymap to install in an order that keeps precedence, guard rails and conflicts as a typed verdict about the rows the app lists (two rows under one place name are in one place; the commands of a part of the app that is not shown hold no key), the shortcuts file (export, and an import plan that says what is wrong with a key it cannot read). |
| `crates/writer-keymap-ui` | GPUI | "Keyboard shortcuts" as it is seen (`KeymapView`): the list of every key grouped by place, the search by name, by place and by key, changing a key (it waits for the next keystroke), the verdict in words, the row menu, the import preview. It shows and asks; the app does. |
| `crates/writer-app` | GPUI bin `the-writer` | Window and workspace shell, command palette, quick open, library search, settings (the app's and the open document's), version history, the keymap (the defaults every crate bound, the user's changes installed over them, their storage, the rows of "Keyboard shortcuts", the export and import of the shortcuts file), status bar, autosave, export flow, the zoom of the text (its commands, where it is kept, life size, the pinch), Print (the command, the pages made in slices, where a job goes: the macOS print panel, a recorder in a test, a directory in a scripted run), images (the provider the editor gets over the library, with its own connection to `images.db` so that nothing large is read or written on the main thread; the app's look at the system pasteboard; the pictures for the exporters and for a print; the collector of bytes nothing names), the native menu bar, the menus of the title and the status bar, the single-instance guard, the spelling settings and the language picker, the Agents panel, agents that write while the app is open (the listener of `writer-agent`'s channel, and what a request does in the window), the assistant in the app (whether it is there, its settings, its two chats mounted, what they are given and what is applied to the text), pages (an open page is a second kind of open item: WebKit's view above GPUI's view, its focus, keys, overlays, find, zoom, theme, print and export; the page engine, WebKit or the fake of the tests; File > Import Page… and drops, with the note, Cancel and the report). Mounts everything above. |
| `crates/writer-assistant` | pure Rust | The engine of the assistant: the interface to a terminal agent (`Backend`, nothing in it assumes Claude), the Claude Code backend, the fake backend for the tests of every crate, the conversation model (`Transcript`, `ChatSession`), context assembly, the system prompts, the library's MCP server as read-only tools. |
| `crates/writer-assistant-ui` | GPUI | The assistant as it is seen: the shared chat components (message list with streamed, formatted answers, composer, model and effort pickers, answer actions, notices) and the side chat panel (`AssistantPanel`). |
| `crates/writer-assistant-quick` | GPUI | The quick chat: a popover at the selection or the caret (`QuickChat`), what it captures and what it does to the text (an answer that names an image of the document brings it back; no answer reads a file). |
| `crates/writer-agent` | pure Rust | The command layer for agents: a typed API for every operation on the library (read, create, edit, images, comments, history, export, trash) over `writer-store`, `writer-core`, `writer-export` and `writer-image`, with the app-is-open rule and the safety net (a version before every change) inside it. An image file an agent names is read and made into what the library stores in the process that was asked, and travels with the request. Pages too: `add_page` is prepared (read, fetched, checked by a `PageChecker`) by whoever asks, and added, updated by its address (a version kept first), confirmed unchanged or left in the trash; `pages`, and a page's branch of every read, of the versions and of export (a folder, the original, text, a PDF); exit codes 14 (`page_incomplete`) and 15 (`wrong_kind`). |
| `crates/writer-mcp` | pure Rust | The MCP protocol layer: a stdio server that only has tools, over a line stream (37 tools, 12 of them read-only; `add_image` and `add_page`, one page a call, are among those that change the library, `list_pages` is hidden behind `writer pages`, and no tool returns a picture). |
| `crates/writer-cli` | pure Rust bin `writer` | The command line tool for agents and scripts (`writer image add`, `writer images`, `writer page add` with `--list`, `writer pages` among its commands), and `writer mcp`, the MCP server. WebKit never runs in its process: `writer __page-check <dir>` and `writer __page-pdf <dir> <out>` are its children (`src/page_check.rs`, not in the help). Ships inside the app bundle at `Contents/MacOS/writer`. |
| `crates/writer-agent-setup` | pure Rust | What is known about the agents on a Mac: how each is detected, where its skills live, how an MCP server is added to it; the text of the skill; installing and removing it. Used by the tool (`writer agents ...`) and by the app (the Agents panel). |
| `scripts/` | shell | `.app` bundling and `.dmg` creation (unsigned / ad-hoc signed). |
| `docs/gpui-notes.md` | notes | Verified GPUI API notes for this exact gpui version. Read it before writing GPUI code. |

Dependency direction: `app → {editor-tools, vim-mode, comments, spell, library, export,
assistant-quick, assistant-ui, assistant, keymap-ui, keymap}`, `keymap-ui → {keymap, ui}`
(`keymap` depends on nothing of ours and not on GPUI), `assistant-quick → {assistant-ui, assistant,
editor, export}`, `assistant-ui → {assistant, editor, export, ui}`, `assistant → {mcp, agent-setup}` (for
the list of its read-only tools and for the folders installers put a program in; no GPUI),
`{editor-tools, vim-mode, comments, spell} → editor → {ui, core, print}`, `vim-mode → vim → core`
(the Vim crates do not depend on `spell`: the app connects the two, `VimMode::set_spelling`),
`library → {store, ui}`, `image → store`, `export → core`, `cli → {agent, mcp, agent-setup}`,
`mcp → agent`, `agent → {store, core, export}`, `app → {agent-setup, agent}`,
`{app, agent} → image`, `app → print` (`print` depends on nothing of ours: the editor's print pass
fills its pages, the app has a PDF made of them and hands it to the system),
`{app, agent, mcp, cli} → page`, `{app, cli} → web` (`page` and `web` depend on nothing of ours; the
app and the tool's children map WebKit's check to the bundler's report). Never the reverse. None of the agent crates depends on GPUI. The app does not depend on `writer-cli`
(the tool is a separate program beside it). It depends on `writer-agent` for one thing, the
live channel (`writer_agent::live`: the listener, and `execute`, which carries a request of
the tool out on the library's own store; `crates/writer-app/src/live.rs` and
`workspace/live.rs` are the only callers; `pages.rs` also reads the name of one variable,
`writer_agent::FETCH_LOCAL_ENV`), and the assistant has that crate in its build
through `assistant → mcp`, which reads the table of the server's tools. Every internal
crate has an
entry in the root `[workspace.dependencies]`; depend on one with `writer-x.workspace = true`.

## Build and test

```bash
cargo build -p writer-app            # debug build
cargo run -p writer-app              # run the app
cargo test -p <crate>                # per-crate tests (GPUI crates)
CARGO_TARGET_DIR=target-pure cargo test -p writer-core   # pure crates use target-pure
CARGO_TARGET_DIR=target-pure cargo test -p writer-page   # the bundler (no network: MapFetcher, a server on 127.0.0.1)
CARGO_TARGET_DIR=target-pure cargo test -p writer-web    # WebKit, offscreen, in a program of its own
THE_WRITER_DATA_DIR=/some/tmp cargo run -p writer-app   # run against a throwaway library
CARGO_TARGET_DIR=target-pure cargo build -p writer-cli   # the `writer` tool (target-pure/debug/writer)
cargo build -p writer-cli            # the same beside a debug app (target/debug/writer), where the Agents panel looks for it
scripts/bundle.sh                    # release .app (the app and the tool) + .dmg in dist/
THE_WRITER_DIST_DIR=/some/tmp scripts/bundle.sh   # a trial build that leaves the released images in dist/ alone
```

The real library lives in `~/Library/Application Support/The Writer/`. Tests and manual
QA must set `THE_WRITER_DATA_DIR` to a temp directory and never touch the real one.

Scripted runs of the real app (visual QA; grammar in `crates/writer-app/src/automation.rs`,
recipe in `docs/gpui-notes.md`):

```bash
# THE_WRITER_SEED_DEMO=1: a few folders and documents in an empty library (optional)
# THE_WRITER_EXPORT_DIR:  file exports are written there, without the save panel (optional)
# THE_WRITER_ASSISTANT_FAKE=ready: the assistant on the fake engine (optional, see "The assistant")
# THE_WRITER_FETCH_LOCAL=1: a debug build (app and tool) fetches a page's pieces from 127.0.0.1
#   and localhost too, for a test's own server (optional; a release build does not read it)
# THE_WRITER_PAGE_CHECK=fake: a debug `writer` checks a page with the static check in its own
#   process, without WebKit (optional; a release tool always runs the real check)
THE_WRITER_DATA_DIR=/some/tmp \
THE_WRITER_SEED_DEMO=1 \
THE_WRITER_EXPORT_DIR=/some/tmp/out \
THE_WRITER_AUTOMATION="wait:1200; keys:cmd-n; type:Hello; state; screenshot:/some/tmp/a.png; quit" \
./target/debug/the-writer
```

Steps: `wait:<ms>`, `keys:<keystrokes>` (the key `;` is written `\;`: `keys:cmd-\;`), `type:<text>`, `action:<name>`, `menu:<name>` (opens
a menu of the workspace at its control: `title`, `save`, `statistics`, `page`, `limit`, `zoom`),
`choose:<label>` (chooses that item of the open context menu, as a click would),
`menukey:<title>` (prints the key the native menu bar has beside that item, read from
AppKit's own menu: `menukey:Bold`),
`bar` (shows the format bar over the selection, as a selection made with the mouse does;
`bar:bold`, `bar:highlight`, `bar:turn-into`, `bar:extra-0`, `bar:more` press that button of
it), `shortcut:<action>=<keys>` and `shortcuts:<what>` (the keyboard shortcuts, see "Changing
keys"), `agents:<n>[:<ms>]` (waits until agents have made `n` changes through this app,
the `writer` tool being run from outside meanwhile; fails after `ms`, 10 s unless given) and
`agents:on` / `agents:off` (the switch "Agents can make changes while The Writer is open"), `fake:hold` / `fake:advance:<events>` / `fake:release` (the fake engine of the
assistant: hold its events back and let them through, for a capture of an answer half
written), `zoom:<what>` (the zoom of the text, see "Zoom": `zoom:125`, `zoom:in`, `zoom:fit`, `zoom:auto`),
`print:pdf:<path>` (writes the exact PDF "Print…" would hand to the print panel and says what
was made: pages, paper, rows, `rows off`, images, fonts, the time) and `print:job:<path>` (the
same PDF through the real macOS print operation as a save job, without a panel; see
"Printing"), `image:<path>` (inserts the image file at the caret as Insert > Image… does once
the file is chosen, waits until it is stored and in the text and says `image: inserted <id>
1600x1067 at block 3`, or the sentence for a file that cannot be used) and `image:select:<n>`
(puts the caret on the n-th image, from 1; see "Images"), `page:make:<folder>[|<entry>]`
(stores a folder that is offline already as a page, as it is: QA only), `page:import:<path>`
(adds a file or folder as File > Import Page… does once it is chosen, waits until it is done
and says `page: added page:12 “Orbitarium” 31 files 1.0 MB` or `page: not added: <the
sentence>`), `page:wait[:<ms>]` (until the open page has loaded), `page:text`,
`page:find:<query>`, `page:js:<script>` and `page:menu` (see "Pages"),
`resize:<w>x<h>`, `theme:<light|dark|system>`, `screenshot:<path>`, `info`, `state` (one
line: document, save state, what the last save cost, focus, modal, the open context menu,
the zoom of the view (`zoom=[page 177% fit-width life=177%]`), printing (`print=[idle]`,
`print=[preparing 40%]`, `print=[panel]`), images (`images=3`, and with the caret on one
`image=[block=2 width=50% natural=1600x1067@144 align=center ready alt="..."]`: the block
from 0, the width as a share of the text column, pixels and pixels per inch, and `ready`,
`loading`, `missing` or `broken` for its picture), the open page (`web=[id=12 state=loaded
zoom=100% overlay=none|waiting|snapshot|holes:N focus=page|find|elsewhere responder=web|gpui
find="q":found missing=0]` or `web=[none]`), the pages being added (`adding=[2/3 "Orbitarium"
fetching]`, `checking`, `storing`, or `adding=[idle]`),
Vim, tool, the format bar, comments, spelling, how many keys are changed and what the list
of shortcuts shows while it is open, status bar, the list of fonts of the settings while it
is open (`font_list=[target=... query=... rows=N selected=... in_use=... missing=...]`), agents that
write while the app is open (`live=[channel=open|closed switch=on|off waiting=N changes=N
last="Claude changed “Chapter 2”" run=N offers=Undo|Open|nothing cost=[save, change, window]]`:
the last part is what the last change cost on the main thread), the Agents panel while it is
open, and in a run with an engine the assistant: whether it is shown, the quick chat while
it is open, the side chat while it is in the right panel), `dump` (`dump:<path>`: the whole text in the editor, written to a file), `log:<text>`, `quit`, and for measuring
`book:<blocks>` (creates and opens a generated document, and says how long it took until
the text on screen was checked for spelling), `savecost[:<rounds>]` (every part of a
synchronous save, timed), `limitcost[:<rounds>]` (the character-limit count),
`keyscost[:<rounds>]` (what a change of a keyboard shortcut costs: the whole change as the
list makes it, the frame after it, the parts of installing the keymap),
`typecost:<text>` and `keycost:<keystrokes>` (press the keys at a writer's pace and time
each: dispatch, the frame after it, how long the main thread kept a timer waiting, the
spell checker's slices) and `stall:<ms>` (watches the main thread without pressing
anything). Useful
lines: open a document `keys:cmd-p; type:chap 2; keys:enter`; a palette command
`keys:cmd-shift-p; type:version history; keys:enter`; the settings of the open document
`keys:cmd-shift-p; type:document settings; keys:enter`; a context menu `keys:ctrl-enter`
(in the text, the library, the comment list or a text field), then `choose:Add Comment`;
the format bar `keys:shift-right shift-right; bar; bar:bold`; a picture `image:/some/tmp/a.png;
keys:cmd-shift-,` (smaller), `keys:ctrl-enter; choose:Description…; type:The harbour; keys:enter`.
There are no mouse steps (GPUI cannot be handed pointer events from outside); mouse
behaviour is tested headlessly with the test context's `simulate_*` calls.

**One running copy per data directory.** A launch takes an exclusive lock on
`instance.lock` in its data directory and keeps it until the process ends
(`crates/writer-app/src/single_instance.rs`, over `writer_store::instance_lock`, the one
implementation the app and the `writer` tool share). A second launch on the same directory
does not open the library: it shows a window that says another copy has it (and prints the
same to stderr), with a button that brings the other copy to the front. So two scripted runs
must not share a `THE_WRITER_DATA_DIR`; the second one's `state` step fails with "the window
does not show a workspace". A build from before the guard takes no lock and is not seen.
The `writer` tool takes the same lock for the moment of each write (its line in the lock
file ends in ` tool`): a launch that finds the lock held by a tool waits up to three
seconds for it (`single_instance::TOOL_PATIENCE`) before it says the library is in use.

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

What a change of a key costs (measured for 0.5.0 in the release build: 6 to 14 ms for the
whole change, of which installing the 452 bindings is under 3 ms and the menu bar 1 to 5;
launch takes the same 250 ms with no change, five changes and 136 changed commands
stored):

```bash
THE_WRITER_AUTOMATION="wait:5000; keyscost:15; keys:cmd-/; wait:1000; keyscost:15; quit"
# keyscost: a change of a key, 30 times: 9.10 ms (worst 12.57)
# keyscost: installing 452 bindings: what to bind 1.00 ms, building them 1.29 ms, clear and bind 0.09 ms, the menu bar 4.82 ms
```

What a change by an agent costs while a book is open (the main thread is held for all of
it; measured for 0.6.0 in the release build, with the release tool, on a book of 10,000
blocks and 660,000 words in one document: an append 173 ms, a one-word edit in the middle
179 ms, a comment 108 ms, each the median of five; about 100 ms more when there is unsaved
typing to save first. Of an append the change in the library is 153 ms (writing the
document with its search index 80, counting its words 28, the version kept before it 10,
reading it 10), bringing the window up to date 14 ms. The tool takes 20 to 30 ms on top
to start and to ask. It grows with the document: a chapter of 5,000 words 4 to 6 ms, a
novel of 100,000 words in one document about 40 ms. In the debug build the append to the
book is 2.5 s. **A picture an agent adds** (`writer image add`) is stored on the main thread
too, as part of the change: its bytes come with the request, are hashed and written with a
sync. Measured when images were added, five pictures of each size that the library stores
as they come, with a short document open: 1 MB 4 ms (3.4 to 6.9), 4 MB 15 ms (10.9 to
17.5), 16 MB, the most a picture can weigh, 62 ms (59.8 to 63.6), each "change"; the save
before it and the window after it under half a millisecond; the tool 30, 49 and 124 ms from
start to answer. With the book of 10,000 blocks open the same three are 136, 150 and
190 ms of "change" (the append of a block to the book is most of it) and 14 ms for the
window: the 16 MB picture holds the main thread for 206 ms, 60 more than the smallest.
The way to take the write off the main thread, should that ever be too long, is for the
channel's listener to store what a request brings through an `ImageFile` of its own before
the request is queued; it is built for pages, not for pictures). **A page an agent adds**
(`writer page add`) is stored that way: the thread that accepts the request writes its
files into `images.db` through a connection of its own before the request is queued, so the
main thread writes the page's row only. Measured when pages were added, in the debug build
with the debug tool and the real check, a page with 16 MB of files: "change" 1.8 ms with a
short document open and 0.9 ms with the book of 10,000 blocks open (its save first, 21 ms,
is the book's flush), "window" 0.1 ms; the tool 5 s from start to answer, most of it the
check in WebKit:

```bash
THE_WRITER_AUTOMATION="wait:1500; book:10000; wait:4000; log:book ready; agents:1:60000; wait:300; state; quit"
# when the log says "book ready", from a second terminal, with the same temporary library:
#   writer --library <dir> recent                # the id of the book
#   time writer --library <dir> append doc:<id> --content "One more paragraph." --author Claude
# state: live=[... cost=[save 6.2 ms, change 153.3 ms, window 14.4 ms]]   (release)
#   time writer --library <dir> image add doc:<id> /some/tmp/16mb.png --author Claude
# state: live=[... cost=[save 2.6 ms, change 189.3 ms, window 13.2 ms]]   (release, the book open)
#   time writer --library <dir> page add /some/tmp/heavy-page --author Claude   # index.html and 16 MB beside it
# state: live=[... cost=[save 21.2 ms, change 0.9 ms, window 0.1 ms]]      (debug, the book open)
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
`tests/suite/spelling.rs` holds spell checking in the assembled app, with the fake checker;
`tests/suite/agents.rs` holds the Agents panel against a temporary home directory;
`tests/suite/launch.rs` launches twice with a change made in between, as an agent makes it;
`tests/suite/live.rs` holds agents that write while the app is open: the assembled workspace
with the real listener, and the real client of `writer-agent` asking from a second thread;
`tests/suite/assistant.rs` holds the assistant in the assembled app, with the fake engine;
`tests/suite/keybindings.rs` holds the keys the user changes: the keymap taken apart and put
back, every kind of change, the list, the guard rails, storage, the shortcuts file;
`tests/suite/zoom.rs` holds the zoom in the assembled app: the keys, the palette, the menu
bar, the status bar item and its menu with the pointer, `cmd` with the wheel, what is kept
over a relaunch, what a page opens at, and the tools, the menus, the format bar, the quick
chat, the comments and Vim's caret at 75 %, 100 % and 150 % in both views;
`tests/suite/print.rs` holds printing in the assembled app, every job recorded and none in a
panel: the pages and their text against page view, what of the window is not on paper,
where Print is reached from and when it is there, the note and Cancel, the keys;
`tests/suite/images.rs` holds images in the assembled app, with the app's own provider over
a library in memory or on disk and picture files made when the test runs: every way in, the
bytes stored before a block names them, undo and relaunch, cut and paste between documents,
the pasteboard (the decision, and the read against a pasteboard of the test's own), the
commands and their keys, the exports, a print, the collector's timers, the preview of a
version, Vim mode on an image (the image's menu is in `context_menus.rs`, its description
with the pointer in `mouse.rs`, agents in `live.rs` and `launch.rs`, the assistant in
`assistant.rs`); `tests/suite/pages.rs` holds pages in the assembled app on the fake page
engine (no test makes a `WKWebView`): opening one, the commands that do not apply, focus
and the first responder, the snapshot under an overlay and the holes of toasts, find, zoom,
theme, a crash, print, export, its versions, and adding pages (Import Page…, drops on the
sidebar, a page and the text, a refusal and Show Why, Cancel while a fetch is held, the
note, files that were lost, no engine); agents' pages are in `live.rs`, the side chat over a
page in `assistant.rs`, the library's menus in `context_menus.rs`.

**Tests never ask the Mac's dictionary** (its answers differ between macOS versions, and
its learned words are shared with every other app). A workspace checks spelling only when
the app was given a spell service: `main` installs the system one
(`writer_app::spelling::use_system_service`), a test installs
`writer_spell::FakeSpellService` with `writer_app::spelling::set_service` before it opens a
workspace, and without one there is no checker at all. A scripted run of the real binary
does use the system checker: do not choose "Learn Spelling" there except on an invented
word that is unlearned again in the same run.

**Tests never open the open panel and never read or write the system pasteboard.**
"Insert > Image…" asks for files in the macOS open panel only in a run that was told to
(`writer_app::images::use_open_panel`, called by `main` when the run is not scripted); a
test names the files (`images::choose_files_beforehand`), a scripted run uses `image:<path>`,
and without either the panel was "cancelled". A paste looks at the system pasteboard for a
picture only where `main` said so (`pasteboard::use_system`); a test says what is "on" it
(`pasteboard::stand_in`), and a scripted run has none. Do not script Copy or Cut on an image
either: it puts the picture on the user's clipboard.

**Tests never make a `WKWebView` and never reach the network.** A workspace shows pages only
when the app was given a page engine: `main` installs WebKit (`writer_app::pages::use_webkit`)
in every run of the real binary, scripted ones too, unless `THE_WRITER_PAGES=off` or this
macOS cannot keep a page offline; a test installs the fake (`pages::use_fake`, which draws a
page's text in a GPUI element and answers the check with `writer_page::static_check` or what
the test set), and without one a page shows its text and nothing can be added. The import
asks the open panel only where `main` said so (`pages::use_open_panel`); a test names the
paths (`pages::choose_beforehand`). It fetches with `curl` for public addresses, except
where a test installed a fetcher (`pages::set_fetcher`, a `writer_page::MapFetcher` or one of
its own); on the fake engine without one nothing is fetched at all. Real WebKit is tested in
`writer-web`'s own program and in the cases of `writer-cli` that run the built tool.

**Tests never call the real Claude Code** (every message spends the user's own allowance,
and its answers are not the same twice). A workspace has an assistant only when the app
was given an engine: `main` installs Claude Code (`writer_app::assistant::use_claude_code`),
a test installs `writer_assistant::FakeBackend` with `writer_app::assistant::set_backend`
before it opens a workspace, and without one there is no assistant at all. The engine
crate's own tests of the Claude Code backend run a stand-in program that replays recorded
output, never `claude`. A scripted run of the real binary never gets Claude Code either:
it gets the fake engine when `THE_WRITER_ASSISTANT_FAKE` asks for it, and no assistant
otherwise. A real call is made only where a task says so in so many words: a one-line
prompt, the cheapest model at low effort, session persistence off, never in a loop and
never from a test that runs by default.

## The assistant

An optional writing assistant inside the app, in two chats, running on a terminal agent
the user has installed and signed in to. Claude Code is the first (and for now the only)
engine; others will follow, so nothing outside `writer_assistant::claude` may assume it.
The assistant advises and the user applies: an answer changes a document only when one of
its buttons is pressed. Every message spends the user's own plan in the engine.

- **Whether it is there** (`crates/writer-app/src/assistant/host.rs`,
  `workspace/assistant.rs`). The engine is looked for in the background, at launch and
  each time the settings are opened; the app never waits for it. Until the look of a
  launch has answered, the answer of the last launch stands (`assistant.engine_seen`), so
  the keys work from the first moment where the engine was found before. Not installed: nothing of
  the assistant is shown (no key does anything, no command in the palette or the
  list of shortcuts, no item in the menu bar, the text menu or the format bar, no panel),
  except one line in the settings that says what it needs. Installed and not signed in:
  it is shown and says how to sign in. `Workspace::assistant_shown` is the one answer; the
  commands of the category "Assistant" are hidden with `commands::set_category_hidden`,
  the menu bar is rebuilt by `menus::set_assistant_shown`.
- **The quick chat** (`cmd-j`, "Ask the assistant" in the palette, "Ask the Assistant" in
  the text menu and on the format bar, Edit in the menu bar;
  `writer_assistant_quick::QuickChat`): a popover at the selection, or at the caret. It
  sends the selected text and the question, and with nothing selected the question alone:
  never the document, its title or the library, and it has no tools. Each answer has
  Replace Selection, Insert Below and Copy. `escape` closes it and ends its conversation.
- **The side chat** (`cmd-3`, "Go to assistant" and "Show or hide assistant" in the
  palette, View in the menu bar, and for the pointer the small sparkle at the end of the
  status bar and "Assistant" in the menu of the title;
  `writer_assistant_ui::AssistantPanel`): a panel at the right. With every message it sends the open document as it is in the editor at that
  moment (not as it was last saved), its title and its path in the library, and names the
  documents and folders the user added with "Add" (chips; `cmd-shift-a`). It reads them,
  and the rest of the library, through the library's own MCP server with only the
  read-only tools allowed (`workspace::library_tool_server`: the bundled `writer` beside
  the app's executable, `--library <data directory> mcp`; reading works while the app has
  the library open). Each answer has Insert at Caret, Replace Selection and Copy; the
  workspace applies them as one undo step, the answer's Markdown becoming formatted text
  (`assistant/apply.rs`: one paragraph is running text and joins the block it goes into,
  anything else goes in as whole blocks and is never merged into the text around it).
  An answer that went into the text leaves the keyboard in the text, so `cmd-z` right
  after it takes the answer out again (`Workspace::apply_answer`).
- **Images.** The assistant never sees a picture. In the document the side chat is sent, an
  image is one line, `![description](image:<id>)` (the Markdown export's `References`), and
  its system prompt says what the line is and that it is to be kept unchanged and never
  invented (`writer_assistant::prompts`). The quick chat is sent the selected text, in which
  an image is an empty line. An answer that is applied is read with the images of the open
  document (`writer_export::KnownImages`): a line that names one of them is that image
  again, with its width, alignment and description, so an answer that moves the line moves
  the image. The side chat also takes the id of an image of another document (the assistant
  read it there with its tools): it comes in at its own size and counts as used from then
  on (`assistant/apply.rs`, `LibraryAnswerImages`). **Any other image in an answer stays a
  link: a path or an address in an answer is never read.** The engine has no file tools, so
  a local path in an answer is invented or was planted in a text the model read; reading it
  would let the text of a document pull a file of the user's into the document at the press
  of a button. (Agents, which have the file system anyway, do read local paths: "Agents".)
- **Pages.** With a page open the side chat is sent the text the page showed (the
  manifest's `text`, what is not shown included) in place of the Markdown, after one
  sentence (`writer_assistant::PAGE_PREFACE`: "The open item is a page: a web page saved in
  the library, shown as it was made. It cannot be changed. Its text, as the page showed
  it:"), with `kind="page"` and its address in the tag (`OpenDocument::kind`, `OpenKind`),
  and its system prompt says what such an item is. A page too long to send whole goes as
  its beginning and its end (it has no headings to make an outline of). `cmd-3` and the
  side chat's commands work over a page (`Workspace::open_item_id`); Insert at Caret and
  Replace Selection are off (no caret, no selection), Copy works; the conversation is the
  page's own (`assistant.conversation.<id>`). The quick chat does not open on a page: its
  key says so in a toast and names the side chat's key.
- **The slot at the right is shared with the comments, one at a time**
  (`workspace/right_panel.rs`). A panel is there when it has something to show (comments:
  the document has a thread or one is being written; side chat: the user has it open).
  With both there the slot shows the one used last under two tabs, "Comments" and
  "Assistant"; `cmd-2` and the comment commands bring the comments forward, `cmd-3` and
  the side chat's commands the side chat. Each panel has a width of its own, changed by
  dragging the left edge of the slot (double click: the default) and remembered.
- **Conversations** (`assistant/conversations.rs`): the side chat has one per document.
  Switching documents switches it; it is stored in the library's settings
  (`assistant.conversation.<document id>`, the recent end: 80 messages, 120,000
  characters), comes back after a relaunch, and is removed with the document (also at
  launch, for a document the tool deleted while the app was closed; a conversation is
  never handed to a later document that got the same id). "Clear" removes the messages,
  "New Chat" also what was added.
- **Model and effort** are chosen in each chat from two small pickers and remembered per
  chat; the settings show the same four choices. The app stores them as the engine names
  them (`assistant/choices.rs`) and knows no engine's models. Each menu begins with the
  engine's own default; the model's says which model that is where the engine tells
  ("Claude Code's default (Fable 5.2)"), the effort's names none
  (`Catalog::agent_default_effort_label`). Claude Code changes both in the running
  conversation; an engine that cannot is started again and handed what was said.
- **Privacy**: nothing is sent before the user sends a message. Until the first message
  both chats show a note that the text goes to the engine's vendor through the user's own
  sign-in; it is acknowledged once per library (`assistant.notice_accepted`).
- **Errors** (not signed in, the usage limit with the time it resets, the program missing
  or crashed, no network) are worded by the engine (`AssistantError::text`) and shown in
  the chat with what to do.
- **Isolation** is the engine's: the agent runs in a neutral directory with the app's own
  system prompt, without the user's project instructions, hooks, skills, MCP servers or
  built-in file and shell tools, and leaves nothing in the user's session history.
  Checked for 0.4.0 with ten real messages through the bundled app (the cheapest model):
  `system/init` of every conversation named no tool but the twelve read-only ones of the
  library (none at all in the quick chat), no server but the library's, no skill and no
  command; no line was added to `~/.claude/history.jsonl`, no transcript and no project
  folder was written. What the program itself still does: it overwrites one small file of
  its own bookkeeping with the id of the last session
  (`~/.claude/plugins/store/cc-plugin-agents-md_builtin-*.json`). The markers of an
  enclosing Claude Code session are taken out of the environment (`claude/args.rs`,
  `REMOVED_ENV`): they are there when the app is started from a terminal that Claude Code
  runs, as every scripted run of an agent is.
- **Settings** (in the library, like the others; section "Assistant" of the settings):
  `assistant.enabled` (bool, on by default: the switch), `assistant.quick.model`,
  `assistant.quick.effort`, `assistant.side.model`, `assistant.side.effort` (strings: the
  engine's name for the choice, `@default` for the engine's own default, absent for what
  the engine suggests for that chat), and what the app remembers by itself:
  `assistant.panel_open`, `assistant.panel_width`, `comments.panel_width`,
  `right_panel.choice`, `assistant.notice_accepted`, `assistant.engine_seen`.
- **Scripted runs and screenshots**: `THE_WRITER_ASSISTANT_FAKE=ready` (or `signed-out`,
  `missing`) gives a scripted run the fake engine, which starts no program;
  `THE_WRITER_ASSISTANT_FAKE_REPLIES=<file>` scripts its answers (answers separated by a
  line of `====`; leading lines `!tool <name> <words>` and `!error <kind>`; grammar in
  `crates/writer-app/src/assistant/fake.rs`). `keys:cmd-j; type:Another word?; keys:enter`
  asks in the quick chat, `keys:cmd-3` goes to the side chat, `keys:cmd-shift-a; type:Part
  one; keys:enter` adds a folder there. Do not script "Copy" (it is the user's clipboard;
  the side chat's Copy leaves it alone in a scripted run, the quick chat's does not).
  `THE_WRITER_ASSISTANT_REAL=1` gives a scripted run the real Claude Code instead, for the
  few checks that are meant to reach it without bringing the window to the front: a
  script that only looks (the settings, `state`) sends nothing, and **every message such
  a script sends spends the user's allowance** (see the rule above).

## Agents

The AI agents on the user's Mac (Claude Code, Codex, Gemini CLI, Cursor and the others in
`writer_agent_setup::AGENTS`) work with the library through `writer`, a command line tool
built only from the crates without GPUI. It does not need the app to be running, and it
works while the app is running: then the app makes the tool's changes ("While the app is
open" below).

- **Where it is.** In the bundle at `The Writer.app/Contents/MacOS/writer`, beside the
  app's executable, built from the same commit by `scripts/bundle.sh` and signed with the
  bundle. Agents call it by its absolute path; nothing is added to `PATH`. A debug app
  looks for it at `target/debug/writer` (`cargo build -p writer-cli`). The script's smoke
  test runs the bundled tool on a temporary library: create, read back, the app shows the
  document, a write while the bundled app is open arrives in the window and in the
  library, and with the owner's switch off it is refused with exit code 5.
- **The one rule.** Agents can read and change things at any time, and **the library has
  one writer at a time**. While the app is closed that is the tool, which takes the lock
  of the data directory for the moment of each write. While the app is open it is the app:
  it holds the lock, text that is not saved yet and its own picture of the library, so the
  tool does not write underneath it but hands it the operation, and the app carries it
  out. There is no merging. `crates/writer-agent` enforces it (`Library::write` is the one
  way to a library that can be written to). Where the app does not take the change, the
  tool says so with exit code 5 (`app_is_open`) and words that say what to do: the owner
  switched changes off, the app is a version before 0.6.0 (its lock file names no
  channel), it did not answer in ten seconds, a text was being composed.
- **While the app is open** (`writer_agent::live` is the channel, the protocol, the client
  and `execute`; `crates/writer-app/src/live.rs` and `workspace/live.rs` are the app's
  half). The app opens a Unix socket once it has the lock (`live::open_at_launch`, `main`)
  and its path stands in the lock file; a thread accepts requests and queues them for the
  main thread; every way of quitting closes it (`live::close` in a quit hook: a request
  still in the queue is answered "quitting", and the tool then writes directly once the
  lock is free). A request is **one turn of the main thread** (`Workspace::carry_out`), so
  no key press comes between its steps: (1) what is unsaved in the open document is saved
  with the synchronous flush, which settles the background autosave first (a save that
  fails refuses the request); (2) `live::execute` works on the library's own store through
  `Library::change_outside`, which emits the events the library's own methods emit, so the
  sidebar, the trash and an open version history follow; (3) the open document, when it is
  among what changed, is read back and taken over by the editor
  (`Editor::adopt_document`, `EditorState::adopt`): the blocks that did not change stay,
  caret, selection and view stay, the change is one step of undo on top of the writer's
  own, title and the flag of a chosen title are read again; a document that was trashed or
  deleted closes as it does when the writer trashes it; the keyboard and an open dialog
  stay where they are; (4) one note at the bottom; (5) the answer goes back, after every
  view has heard of the change. A read by the tool first asks the app to save (`flush`),
  so an agent reads the text as it is on screen.
  - **Nothing to save afterwards.** The editor reports the text it took over as an edit.
    `Workspace::text_is_what_an_agent_stored` tells it from the writer's own by the
    revision of the document: the session stays "Saved" and nothing is written twice.
    The first edit of the writer's (an undo of the agent's change is one) is saved.
  - **The note** (`workspace::AgentChanges`): "Claude changed “Chapter 2”", the name being
    the `--author` ("An agent" without one; `workspace::sentence` has the words of every
    operation). A change that comes in while the note of the one before is still up joins
    it: "Claude made 20 changes to “Chapter 2”", never a pile. Its button is **Undo**
    where one step undoes all of it: the steps the run added to the undo history of the
    open document (`Editor::undo_steps`; exactly what `cmd-z` takes out, and the offer
    goes when the writer edits, because then it would take out their words), or what one
    request moved to the trash. For a document that is not the open one the button is
    **Open**. The same two are commands without a key ("Undo the last change by an agent",
    "Go to the last change by an agent": the palette, The Writer menu).
  - **Vim mode keeps what the writer is in the middle of.** The editor reports a change
    it took over as any edit, and Vim mode takes an edit it did not make for the writer's
    own doing elsewhere in the app, which cancels a half-typed command and closes the
    command line (`Vim::sync`). So `Workspace::take_over_open_document` tells Vim mode
    itself, right after the editor took the change over and before the editor's events
    reach it
    (`VimMode::changed_elsewhere`, `Vim::changed_elsewhere`): the mode, an operator waiting
    for its motion, a count and an open `:` or `/` line stay, and the next key means what
    it would have meant. (0.6.0 did not: a search being typed was closed by an agent's
    change, and the rest of the word ran as commands of Normal mode.)
  - **A composition is never broken.** While text is composed with an input method
    (`Editor::is_composing`) a change waits, and the requests behind it; after
    `live::COMPOSITION_PATIENCE` (3 s) it is refused and the composition goes on.
  - **A request is whole or not at all**: carrying out and answering are one turn of the
    main thread, quitting is another. A tool that stopped waiting was told that nothing
    was changed, and then nothing is (`Incoming::begin`).
  - **The switch** "Agents can make changes while The Writer is open" is the setting
    `agents.changes_while_open` of the library (on unless switched off), in the Agents
    panel. Off is the rule of before 0.6.0: agents read (a `flush` is still answered) and
    are told to ask for the switch to be put on, or for the app to be quit (the tool's
    refusal and the panel's sentence `live::SWITCH_OFF` say the same, in that order). `THE_WRITER_AGENT_CHANNEL=off` keeps a run from
    opening the channel at all, which is what a version before 0.6.0 looks like to a tool.
  - **Tests** (`tests/suite/live.rs`): `live::open` on a temporary data directory whose
    lock the test holds, the real client on a second thread, and the test's main thread
    letting the app work until the answer is there (`ask`). The app's timers run on the
    test clock: a wait for a composition needs `advance_clock`.
  - **The tool's half** (`crates/writer-agent/src/live/`, no GPUI). One table,
    `src/operations.rs`, gives every operation that writes its method on `Library`, its
    name on the channel and the function that carries it out in a `Session`; `Library::write`
    is called from nowhere else, so **a new operation that writes goes into that table and
    is routed by that** (a test fails for a public method of `Library` that is neither in
    the table nor a known reader). `Library::write` hands the operation to an app whose
    lock file names a channel, at once and without trying the lock; otherwise it takes the
    lock and writes. `live::execute` runs the same function on the store the app hands in.
    One request and one answer per connection, each one line of JSON with a version
    (`{"version":3,"wait_ms":10000,"op":"edit","request":{...}}`); the answer is the
    result as the direct write gives it, the error as it was (`WireError`), or a
    `Refusal`. **The protocol is version 3** (`live::PROTOCOL_VERSION`): version 2 since
    requests bring image files with them ("Images" below), version 3 since an `add_page`
    brings the files of a page ("Pages"). A request reads past a field it does not know,
    so an app of version 1 (0.6.0) would have taken a request with a picture, kept the
    image as a link and answered "done"; with the number changed the two versions refuse
    each other whole, a `flush` included (`Refusal::Protocol`: exit code 5 with the
    sentence for an app of another version, "nothing was changed" in it; a test sends a
    request with a picture to a stand-in that answers as 0.6.0 does, and a page to one
    that answers as 0.7.0 does). The tool writes the whole line before it connects (a
    page is up to 86 MB of base64, and the listener gives a connection up after a second
    without a byte), and the app reads the version before it reads the request.
  - **Where the socket is.** `the-writer-<uid>/<pid>-<n>.sock` in the user's temporary
    directory (mode 0700, the socket 0600, the peer's user checked): a socket path holds
    about a hundred bytes and a data directory can be longer. The lock file of the data
    directory names it in a second line (`channel <path>`,
    `writer_store::instance_lock::announce_channel` / `app_channel`), which older versions
    read past. The lock says whether an app runs, so a socket or a line left by a crash
    misleads nobody.
  - **What bites there.** macOS refuses `setsockopt` on a socket whose other end is closed
    (`EINVAL`): an answer from an app that hung up at once was taken for lost until
    `read_line` learned to read without a timeout then. macOS clears out of the temporary
    directory, every night, what was not used for three days (`com.apple.bsd.dirhelper`):
    the listener renews the times of its socket every hour. An app that holds the lock and
    names no channel is "a version that takes no changes" only when the lock file is older
    than five seconds (`live::STARTING_GRACE`, `instance_lock::held_since`): a tool waits
    that long for an app that has just started, so **a test that stands for an old app
    sets the lock file's time back** (`age_the_lock`,
    `Sandbox::open_an_app_that_takes_no_changes`). A `Target::Path("1984")` is written
    `{"path":"1984"}` in JSON, or it would arrive as the id 1984.
  - **Its tests** need no app: a thread holds the lock, binds the real listener and calls
    `execute` (`StandIn` in `crates/writer-agent/tests/suite/live.rs`, `LiveApp` in
    `crates/writer-cli/tests/suite/harness.rs`). Every operation is run both ways and the
    two libraries are compared row by row; the whole suite of `writer-agent` runs a second
    time with an app on every library (a child run with
    `WRITER_AGENT_TESTS_THROUGH_THE_APP`, where `fixture()` opens a stand-in and
    `closed_fixture()` is for the tests about a closed app); `writer-cli` runs every
    command whose help says that it changes the library, and every tool the server lists
    as changing something, against a stand-in, and fails when a new one is not run there.
- **The safety net** is in `writer-agent` too: a version snapshot before an agent changes
  the text of a document or deletes a comment (labelled "Before an edit by Claude" with the
  `--author` given; the app's version history shows the label in the row, read through
  `Store::snapshot_labels`), deletion means the trash, `--dry-run` says what would change
  (and "would change nothing" when that is so, `Written::would_change`). Adding, answering,
  resolving and reopening a comment keep no version. Permanent deletion is in the CLI
  behind a flag and not in MCP.
- **An edit keeps what it does not change.** `edit` matches the old text against the plain
  text of the document (then as Markdown), and compares the old and the new text word by
  word: the words they share are not replaced, so words an agent quotes around its change
  keep the formatting, the link and the comment each has; only the words that differ take
  the formatting of what they replace (`writer-agent/src/rewrite.rs`,
  `formatting_to_take`).
- **Images.** To an agent an image of a document is one line of Markdown,
  `![description](image:<id>)` (`read`, `writer get`). The result of `read` lists the
  images with what the line does not say (width in percent, alignment, pixel size,
  whether the library has the bytes: `ImageSummary`), and `writer images <document>`
  prints that list. No tool returns a picture: an agent looks at one by exporting.
  - **Adding one**: `writer image add <document> <file> [--alt] [--width] [--align] [--at]
    [--heading] [--heading-occurrence]`, the MCP tool `add_image` (a row of the table,
    `write_text::add_image`), or a local path in the Markdown of any operation that takes
    text (`![what it shows](/path.png)`, `file:`, `~/`; a relative path is taken from the
    folder of `--file`, else from where the command runs, `Library::with_base_dir`; the
    MCP server is given no folder and reads whole paths only). `add_image` centres the
    image at its own size on the page, Markdown brings one in at the left; it is never
    stored wider than the column or taller than a page (`Image::max_width`), and the
    result says the width that was stored.
  - **The file is read in the process that was asked and travels with the request**
    (`writer-agent/src/attach.rs`: `Prepare::prepared`, which the table calls before
    `Library::write`). `writer_image::prepare_file` makes of it what the library stores,
    the request carries the bytes (`AttachedImage`, base64 on the channel), and the
    function that carries the operation out stores what the request brought
    (`markdown_in::Images`) and **opens no path**: in the app it runs on the main thread,
    and the app must never read a file an agent named. A file that cannot be used is a
    warning that names it with the reason, and the image stays a link (for `add_image`
    it is an error); an address on the web stays a link. The images of one request weigh
    32 MB together at most (`live::MAX_ATTACHED_BYTES`). A dry run stores nothing and
    reports the same.
  - **Every operation keeps the images it does not change, with their size.** Three
    places would have lost one. `rewrite_blocks` (behind `replace_document` and
    `replace_section`) kept a block whose whole kind was the same: the key of an image is
    its picture alone (`rewrite.rs`, `KindKey`), so the block stays with its width and
    alignment, and a reworded description is set on it in place. `edit` matches in a text
    in which an image is an empty line: a match that runs across an image is refused with
    a sentence (`matching::across_an_image`), and an old text given as Markdown that names
    the image where it stands (between its lines, at its ends, or alone) is found there
    and nowhere else. A text written back without the line of an image removes the image:
    the result counts `images_removed` (by picture, so an image that only moved counts
    for nothing) and a warning says that the version kept before the change has it.
  - **An image that is named again is answered from the document as it was before the
    change**, and only then from the library (`markdown_in::Images::know`,
    `writer_export::KnownImages`): the blocks the operation replaces first, then the rest
    of the document, the n-th line that names an id being the n-th image with that id. So
    an image line that moved, one a section got from another section, an `insert` that
    repeats an image and one picture used at two sizes all keep width, alignment and
    description. An image of the library that the document does not hold comes at its
    own size, at the left; an id nobody has stays a link, with a warning.
  - **Export**: a Markdown file links its pictures, and they are written into a folder
    beside it, `<name> images/image-<8 hex>.<ext>` (`Exported::assets`; named by content
    and written over, `--overwrite` is about the text); a DOCX holds them; plain text has
    their descriptions. **An export whose pictures cannot be read fails and writes
    nothing** (`writer_agent::Error::Pictures`, kind `failed`: `images.db` of a newer
    version, busy past its timeout); a picture the library does not have is its
    description. The app's export does the same (`images::StoreImages::failure`).
  - **The plain text an agent reads is the text that is matched**: `get --format text`,
    `read_document` as text and `history show` leave the descriptions of images out (an
    image is an empty line there, as in `edit`), while the plain-text export still writes
    them. `find` marks an occurrence that runs across an image
    (`Occurrence::across_image`), which `edit` would refuse.
  - **The skill** has a paragraph "Images" (`writer-agent-setup/src/skill_body.md`; a
    test holds what it has to say, and its examples are run against the built tool).
- **Pages** (the whole feature is in "Pages" below; `docs/pages-design.md` is its design).
  `writer page add <file|folder>... [--entry] [--title] [--source-url] [--folder]
  [--create-folders] [--include-trashed] [--author] [--dry-run] [--json]`, or `--list
  <file.json|->` for many pages with their own addresses and the pieces the agent has
  already (`externals`: address to a saved file; the recipe of the skill passes claude.ai's
  assets that way); the MCP tool `add_page`, one page a call. Each input is one line of
  output: `added`, `updated` (the page as it was kept as a version first), `unchanged` (the
  same fingerprint from the same address: nothing fetched, nothing written), `trashed` (a
  page with that address is in the trash, so it is not added again unless
  `--include-trashed`), or `not added` with the sentence. **A page is read, fetched and
  checked in the process that was asked** (`impl Prepare for AddPageRequest` always
  prepares again, and a request that names `prepared` is refused by both front ends): the
  check runs in a child of the tool (`writer __page-check <dir>`, standard input closed,
  killed at 135 s), never in the MCP server's process and never in the app for an agent.
  Exit code 14 (`page_incomplete`): the page cannot be kept whole, the reasons named,
  nothing saved; 15 (`wrong_kind`): a text operation on a page (`edit`, `append`, `image
  add`, comments) or a page operation on a text. `writer pages [--json]` lists them with
  their addresses and `content_at` (when this content was taken), `writer get page:12` gives
  the text a page showed, `writer export page:12 <dir>` the served files as a folder
  (`--original`: the input as it was added), `writer export page:12 x.pdf` WebKit's print
  (`writer __page-pdf`). `page:<id>` is the reference of a page in every output; `doc:` and
  `page:` are read alike. While the app is open the tool still prepares the page and the
  app's listener stores its files before the request is queued ("While the app is open",
  "Pages"); the note says "Claude added “Orbitarium”" (Open) or "Claude updated
  “Orbitarium”" (an open page reloads, and the note has no button).
- **Who says what.** `writer-agent` words its errors for both front ends and names fields
  ("occurrence", "permanently"). The command line adds the option to type next, worded
  for the command that failed (`writer-cli/src/main.rs`, `hint_in`: `--occurrence`,
  `--heading-occurrence`, `--all`, `--overwrite`, `--permanently`, `writer find`, and
  `--at` / `--heading` for `writer image add`), the MCP server names the tool
  (`writer-mcp/src/tools.rs`,
  `tool_text`: `restore_version` where the shared text says `writer history restore`).
  Exit code 5 (`app_is_open`) has one sentence per reason (`Refusal`, worded in
  `writer-agent/src/error.rs`, `app_is_open_message`): it says "nothing was changed"
  exactly when that is certain (not when the app took the change up and its answer did
  not arrive) and ends with what to do; as data the reason is `error.refusal.why` with
  `error.nothing_changed` beside it. The command line adds what goes on working (reading,
  `--dry-run`, `writer status`), the server the tools that do (`WHILE_OPEN`). The switch
  has one name everywhere, "Agents can make changes while The Writer is open": the tool's
  message, the skill and the panel (a test of `writer-cli` holds the first two together).
  The skill carries the version of the app (`writer_agent_setup::APP_VERSION`), so a skill
  an older version installed is offered for update without anything else being done. The
  skill's list of exit codes is tested against the help of the built tool
  (`writer-agent-setup/tests/skill_against_binary.rs`): a new exit code fails that test
  until the skill explains it.
- **Which library.** The user's real one by default (`writer_store::data_dir`), another
  with `--library <dir>` or `THE_WRITER_DATA_DIR`. **Every test and every command you run
  must name a temporary library**; the tool pointed at nothing is pointed at the user's
  documents.
- **`writer mcp`** is the same binary as an MCP server over stdio (`crates/writer-mcp` is
  the protocol, `crates/writer-cli` the tools).
- **The skill and the MCP commands** are `crates/writer-agent-setup`: the table of agents
  (how each is detected, where it reads skills, how an MCP server is added, every fact with
  its source and the day it was read), the text of the skill, installing and removing it.
  The tool has it as `writer agents ...`, the app as the Agents panel.
- **The Agents panel** (`cmd-opt-a`, "Agents" in the palette, The Writer > Agents… in the
  menu bar; `crates/writer-app/src/agents.rs` and `agents_view.rs`): the path of the tool
  with a copy button; every agent found that reads skills with the folder and the state of
  the skill, install / update / remove per agent and install for all, with a report of
  every path written; the command or configuration entry that adds the MCP server to each
  agent, with a copy button and a link to its documentation. Its second row is the switch
  for changes while the app is open (`Row::Switch`, a button "Switch Off" / "Switch On"
  and a sentence that says what holds as it stands); it is the library's and not the
  Mac's, so it is there in a run without an agents environment too. The app never edits an
  agent's configuration, and writes into an agent's skills folder only when a button is
  pressed. Keys in the panel: `up` / `down`, `enter` (the row's first button), `backspace`
  (remove the skill), `cmd-c` (copy what the row shows), `o` (documentation), `escape`.
- **Which Mac the panel sees** is `agents::Environment`, an app global. `main` sets it to
  this Mac. A scripted run has none (the panel lists nothing) unless
  `THE_WRITER_AGENT_HOME=<dir>` names a home directory: then that directory is the whole
  machine (`Machine::isolated`), and copy buttons still leave the clipboard alone. Tests
  call `agents::set_environment(Environment::isolated(temp_home, tool))`. **Never run the
  panel's install buttons, `writer agents install`, or an agent's own `mcp add` against the
  real home directory** (`~/.claude`, `~/.codex`, `~/.gemini`, `~/.agents`, ...): looking
  at which of them exist is fine, writing there is the user's decision. **Setting `HOME`
  to a temporary directory does not make an agent's own command safe**: Claude Code and
  Codex follow it, OpenCode 1.19.5 does not and writes to the real
  `~/.config/opencode/opencode.jsonc` whatever `HOME` says (found the hard way while
  0.3.0 was verified; the entry was taken out again). Check the syntax of an agent's
  command with its `--help` and nothing else. (What that accident showed: OpenCode 1.19.5
  writes a server directly under `"mcp"`, where the documentation of OpenCode 2 and the
  panel's entry have `"mcp": {"servers": ...}`; the panel says so and leads with the
  command, which writes the form the installed version reads.) The `writer` tool itself does follow `HOME`
  (`writer agents install` with a temporary `HOME` writes only there; the tests of
  `writer-cli` guard it).
- **The hint.** Once per library the app shows a toast that agents can work with the
  library, with a button that opens the panel (`agents.hint_shown` in the settings). Only
  the real app shows it; a scripted run does with `THE_WRITER_AGENTS_HINT=1`.
- **A launch shows the library as it is.** Agents change the library while the app is
  closed too, so nothing remembered across launches is trusted without a check: the caret
  and scroll position carry a stamp of the text they were taken on
  (`crates/writer-app/src/view_memory.rs`), and a document that was rewritten opens at its
  start and says so (the same holds for a document an agent changed while another one was
  open in the window); the last open document, the selection and the open folders of the
  sidebar are dropped when what they name was trashed or deleted
  (`tests/suite/launch.rs`, one test per kind of change).

## Working rules

- **Two target directories, no more.** Disk on this machine is nearly full. Everything that
  depends on GPUI builds in the default `target/`. The pure crates (`writer-core`,
  `writer-store`, `writer-image`, `writer-export`, `writer-vim`, `writer-agent`, `writer-mcp`, `writer-cli`,
  `writer-agent-setup`, `writer-assistant`, `writer-keymap`, `writer-page`, `writer-web`) build and test with
  `CARGO_TARGET_DIR=target-pure` so they do not queue behind GPUI builds (`writer-page` and
  `writer-web` compile again in `target/` when the app links them; both are small).
  `writer-web`'s `tests/webkit.rs` is a program of its own (`harness = false`): WebKit wants
  the main thread of the process. `writer-print` has no
  GPUI in it and builds and tests in `target/` all the same (`cargo test -p writer-print`):
  CoreText, CoreGraphics and font-kit are compiled there for gpui already, the crate is only
  ever linked into GPUI crates, and a second copy of that stack in `target-pure` would cost
  disk. Its test binaries are small (no GPUI is linked); `tests/save_job.rs` is a program of
  its own (`harness = false`), because AppKit runs a print operation on the main thread of
  the process. Do not create other
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
   the list of keyboard shortcuts and palette hints are generated from that registry, not
   hand-written. The user can change every one of them ("Changing keys"), so no key is ever
   written into a text, a tooltip or a comparison: whatever shows or obeys a key asks the
   keymap when it does.
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
   **The bytes of an image are stored, committed, before a block that names them enters a
   document**, on every way in (a chosen file, a paste, a drop, an agent's request), so no
   save ever names bytes that are not there; and nothing deletes them but the collector
   ("Images"). A document that names an image the library does not have keeps the block: it
   shows a placeholder and is saved as it was.

## Document model in one paragraph

A document is an ordered list of blocks. A block has a stable id, a kind (paragraph,
heading 1–6, bullet / numbered / todo list item, quote, code block, divider, page break,
image), alignment, indent level, an optional block-level text size, and text with inline
spans. Inline attributes: bold, italic, underline, strikethrough, code, highlight, text
colour, font family, link, and comment anchors. A `\n` inside a block's text is a soft line
break (shift-enter); a code block is one block with multi-line text. Font *family* is inline;
font *size* is per block (GPUI shapes a line at a single size). Comment threads are anchored
to text through span attributes so they move with edits.

A divider, a page break and an image are *void* blocks: no text, one caret position. An
image block (`BlockKind::Image`, an `Image`) holds the id of its stored bytes (`ImageId`:
the first 16 bytes of their SHA-256, 32 hex digits; `writer-core` never sees the bytes), their
pixel size and density, its width as a share of the text column in thousandths
(`ImageWidth`, 5 % to 100 %, never stored wider than a page of the document allows) and a
one-line description. It stands left, centred or right (never justified) and has no indent
and no text size. The caret on an image is "the image is selected"
(`EditorState::selected_image`): copy and cut take the image then. On a void block typing
and `enter` start a paragraph below it and `shift-enter` one above it; `backspace` and
`delete` on it remove it. `backspace` at the start of the paragraph below an image, and
`delete` at the end of the block above one, select the image first and delete it at the
second press (a divider goes at the first). In the document's JSON an image is
`{"type":"image","image":"<id>","px":[w,h],"ppi":144,"width":500,"alt":"..."}`; a document
that holds one is written as format version 2, any other as version 1 as before
(`writer_core::serial`).

## Default keymap

`opt` below is the option key; in GPUI keystroke strings it is spelled `alt`
(`KeyBinding::new("cmd-alt-s", ..)`; `opt` does not parse). These are the defaults: the user
can change every one of them (see "Changing keys"). "Keyboard shortcuts" (`cmd-/`) is
generated from the keymap and lists everything, grouped by where the keyboard has to be
("Anywhere", "In the text", "In the find bar", "In the library", "In the comment list", ...),
including the keys of the panels and overlays that are not repeated here.

Without a key of their own, from the command palette (and the menu bar): "Version history"
(the snapshots of the open document with a preview; `up` / `down` choose, `space` scrolls the
text, `enter` restores, after keeping the current text as a version), "Document settings"
(the settings modal at its "This document" section: page view, paper including a custom
size, orientation, margins, font, text size, line spacing, paragraph spacing; lengths are
typed, `20mm`, `0.75in`), "Rename document", "Reveal in library", "Welcome document"
(opens it; written again when it is gone), "Print" and "Cancel printing" (see
"Printing"; `cmd-p` is Quick open here, so Print has no key until the user gives it one),
and the seven commands of an image (see "Images"): "Image" (Insert > Image…, the open panel),
"Image width 25 %", "50 %", "75 %" and "100 %", "Image original size" and "Image
description". The four widths, the original size and the description need a selected image
and say so in a toast without one. The six commands of pages (see "Pages"): "Import page…"
(File > Import Page…, the open panel, files and folders; only with a page engine), "Cancel
adding pages", "Show the report of the pages added last" (the summary's Show Why), "Reload
page", "Copy page address" and "Open page address in browser" (a page with an address). On
an open page (key context `PageView`) `ctrl-enter` / `shift-f10` open the page's context
menu and `cmd-f`, `cmd-opt-f`, `cmd-g`, `cmd-shift-g` find in the page: the four find keys
are bound once more there, with the defaults the text's tools have, because the tools are
not on a page's path. In the settings `up` / `down` move, `left` /
`right` change, `space` switches; the "Writing" section has the switch for the format bar
and the two spelling rows, the "Status bar" section chooses the character limit and which
counts are shown.
The two font rows of the settings ("Font" under "New documents" and under "This document")
offer every font family that can be chosen in a document: the list the font picker of the
text has (`writer_editor_tools::FontList` is the one source of both: the installed writing
fonts, the recently used families, every family by name, without the hidden system
families). `left` / `right` step through the families by name; `enter`, `space` or a click
on the row opens the searchable list (`font_choice::FontChoicePicker`, each family with a
sample in its own face), which takes the place of the settings as the list of spelling
languages does and comes back to the row after a choice and after `escape` (a click outside
the list closes all of it, as it does for the languages). A font that is set but not on this
Mac (a library from another one) stays set until another is chosen: the row says so, and it
is the first row of the list, marked "Not on this Mac".

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
| `cmd-0` / `cmd-1` / `cmd-2` / `cmd-3` | Go to library / text / comments / the assistant's side chat (opens it; only while the assistant is shown) |
| `cmd-j` | Ask the assistant about the selection, in a small chat at the selection (a plain question with nothing selected; only while the assistant is shown) |
| `cmd-opt-s` | Show or hide the library sidebar |
| `cmd-opt-right` / `cmd-opt-left` | Widen / narrow the library sidebar |
| `cmd-shift-f` | Search the whole library |
| `cmd-shift-s` | Export (Markdown, DOCX) |
| `cmd-s` | Save now (the text is saved as it is written anyway) |
| `ctrl-enter` or `shift-f10` | Context menu of what has the keyboard: the text, a library row, a comment thread, a text field |
| `cmd-,` | Settings |
| `cmd-/` | Keyboard shortcuts: every key, to look up and to change |
| `cmd-opt-p` | Turn page view on or off |
| `cmd-+` (and `cmd-=`, the same key without shift) / `cmd--` | Zoom in / zoom out: the text is drawn larger or smaller, nothing of the document changes |
| `cmd-shift-0` | Actual size, 100 % (`cmd-0` is "Go to library") |
| `cmd-opt-v` | Turn Vim mode on or off |
| `cmd-opt-a` | Agents panel (the command line tool, the skill, the MCP server) |
| `cmd-opt-c` | Show or hide comments |
| `cmd-opt-m` | Add comment on selection (or on the word at the caret) |
| `cmd-opt-]` / `cmd-opt-[` | Next / previous comment |
| `cmd-w` / `cmd-q` | Close the window / quit |

In the assistant's chats (both; the list of shortcuts has them under "In the assistant's
chats"): `enter` sends, `shift-enter` is a new line, `cmd-.` stops the answer, `escape`
stops it or leaves the chat, `tab` / `shift-tab` move between the composer, the
conversation and (side chat) the context row and the header, `cmd-shift-m` / `cmd-opt-e`
open the model and effort pickers, `cmd-shift-c` copies the answer, `cmd-shift-enter`
replaces the selection with it, `cmd-enter` inserts it (below the selection in the quick
chat, at the caret from the side chat). In the side chat also `cmd-opt-n` (new chat),
`cmd-opt-k` (clear) and `cmd-shift-a` (add a document or a folder as context).

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
| `cmd-shift-.` / `cmd-shift-,` | Larger / smaller text; on a selected image, a larger / smaller image ("Larger text or image", "Smaller text or image" in the palette and the list) |
| `cmd-t` | Font picker |
| `cmd-.` | Contextual format menu |
| `/` at the start of a block or after a space | Insert menu |
| `cmd-f` `cmd-opt-f` `cmd-g` `cmd-shift-g` | Find, replace, next, previous |
| `cmd-;` / `cmd-shift-;` | Next / previous misspelling (both wrap around the document) |
| `cmd-z` / `cmd-shift-z` | Undo / redo |
| `cmd-a` `cmd-c` `cmd-x` `cmd-v` `cmd-shift-v` | Select all, copy, cut, paste, paste as plain text |

Plus the standard macOS text-navigation keys (arrows, `alt`/`cmd` + arrows, `shift` to
extend, `alt-backspace`, `cmd-backspace`, and the `ctrl-a/e/f/b/n/p/k/d/h` Emacs set).
`pageup` / `pagedown` move by the height of the window less one line of body text, so a
picture or a heading is as far as it is high (`Editor::rows_in_a_page` in
`writer-editor/src/movement.rs`); Vim's `ctrl-f` / `ctrl-d` (`Editor::rows_per_page`,
`scroll_by_pages`) still count lines of body text.

Keymap facts that bite (details in `docs/gpui-notes.md`):

- **The context menu has two keys.** `ctrl-enter` is the one macOS itself uses and the one
  key hints show; `shift-f10` is bound beside it everywhere (the text, the library, the
  comment list, text fields; bound first, so hints keep showing `ctrl-enter`), because a
  real `ctrl-return` posted to a background window never reached GPUI (keypad enter did)
  and nobody has pressed it on a keyboard in an active window yet. The list of shortcuts
  shows both keys on the row.
- **Panel keys that are easy to miss**: in the library `alt-right` / `alt-left` open or close
  every folder (of the selected folder, or all of them) and a click on a folder row opens
  or closes it; in the comment list `cmd-c` copies the comment's text; `cmd-opt-f` on an
  open find bar with a query moves to the replace field; `escape` in the text closes the
  format bar before the find bar.
- **Shift plus a digit or punctuation** reaches GPUI as the shifted character without the
  shift flag (`cmd-shift-7` is `cmd-&` on a US keyboard). Bind both spellings, the readable
  one last so key hints show it (see `crates/writer-editor/src/actions.rs`).
- **The plus key is the one exception, and it is shown as "⌘+" everywhere.** Shift with `=`
  is the key of "Zoom in", which a Mac writes "⌘+" in every menu. Its normal form is
  `cmd-shift-=` like every other pair, but its glyphs are the character without the shift
  (`writer_keymap::Keystroke::glyphs` and `writer_ui::key_hint::format_keystroke`, the two
  writers of a key, both; never "⇧⌘="), and of its two spellings the character is bound
  last (`Keystroke::spellings`: `cmd-shift-=`, then `cmd-+`), because the last one is what
  the menu bar is given and AppKit draws the character as "⌘+". So the list of shortcuts,
  the hints and the menu bar read the same, for the default and for a key the user gives a
  command (`a_key_reads_the_same_in_the_menu_bar_the_hints_and_the_list`). The list's
  search finds a key by the shifted character too ("⌘+", `cmd-&`).
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
  They are computed from the keymap that is installed, each time it is installed: the key
  the list of shortcuts shows for the command, or the next key of the same row that the
  menu bar can draw. A key with `enter` or `tab` in it cannot be given to AppKit: an item
  whose command has only such keys shows none (a placeholder of two keys says so to GPUI).
  The key beside an item is live: AppKit runs an enabled item on its key where GPUI has no
  binding for it, so the menu bar is built again after every change of a key.
- **The keymap GPUI has is one the app built** (`crates/writer-app/src/keymap.rs`). Every
  crate binds its keys in its `init`; at the end of `writer_app::init` they are taken as
  the defaults, and from then on every change clears the keymap and binds it again in the
  same order (a changed key where its default stood). So: bind keys in `init` and nowhere
  else (a binding added later is lost at the next change), and never compare against a key
  or keep a `KeyHint`: ask the keymap when the key arrives or the hint is drawn
  (`writer_ui::keymap`, `KeyHint::for_action`, `writer_app::keymap::hint_in`).
- **A hint for a command bound in several places is asked in its own place.**
  `KeyHint::for_action` and `CommandDescriptor::key_hint` answer keymap-wide (the binding
  added last), which is another place's key once the user changed one of them. A control
  asks where it is (`KeyHint::for_action_at` with its focus handle, `for_action_here`,
  `keymap::hint_in`). `Window::bindings_for_action` and `KeyHint::for_action_focused` do
  not answer for the focused element in gpui 0.2.2 (`docs/gpui-notes.md`).
- **In a list, `enter` is the list's `Confirm` and nothing else.** The query field of a
  picker, of the format menu and of the font picker leaves its submit key to the parent
  (`TextField::set_submit_to_parent`), so a key given to `picker::Confirm` is the key, and
  the old one stops. The filter field of the library and the fields of the settings and of
  the list of shortcuts keep `enter` as the field's own Submit.
- **`escape` closes at several levels**, each a binding of its own: `text_field::Cancel`,
  `picker::Dismiss`, `editor_tools::CloseTool`, `modal::Dismiss` ("Dismiss", in a dialog).
  Taking the key from the dialog's Dismiss leaves the settings, the version history and
  the Agents panel without a key that closes them (the pointer still does, and opening the
  palette or the list of shortcuts takes their place); the palette and the pickers close
  on their own Dismiss, and the list of shortcuts closes on `escape` whatever is bound.
- **Real key events posted to the app in the background do not go the way a key press in
  an active window goes.** A background app has no active window, so GPUI validates only
  the menu bar items whose actions have app-wide handlers (Quit, Hide, Hide Others), and
  AppKit runs those from the menu bar before the window sees the key. A posted `cmd-q`
  quits while the list of shortcuts waits for a key, where `cmd-w` and `cmd-,` are
  recorded. **Never post `cmd-h`, `cmd-alt-h` or `cmd-m` to the app**: Hide Others hides
  every other application of whoever is at the Mac (it happened while 0.5.0 was verified,
  and was undone with `NSRunningApplication.unhide`).

A layout fact that bites: text that wraps (or has several lines) inside a `flex_col`
container that is itself the growing child (`flex_1`) of a flex row is measured at no
width, and the row comes out thousands of pixels tall. Make the growing child a plain block
`div()` and space its children with margins (`crates/writer-app/src/agents_view.rs`,
`render_row`; the test `no_row_is_taller_than_its_text`).

## Changing keys

Every key that is a GPUI binding can be changed by the user, in one place: "Keyboard
shortcuts" (`cmd-/`, the palette, The Writer > Keyboard Shortcuts… and Help in the menu bar,
the row "Keyboard shortcuts" of the settings, in their section "Keyboard" above "This
document"). Three crates: `writer-keymap` knows what a change is,
`writer-keymap-ui` is the list, `writer-app` has the keymap and does what the list asks.

- **What a change is.** For a command in a key context (`writer_keymap::Target`:
  `editor::ToggleBold` in `Editor`) the user has the keys they want: a key replaced, a
  second key added, a key removed (the command may be left without one), a key given to a
  command that has none ("Version history"; it applies anywhere for a command of the
  window, in the text for a command of the text: `keymap::home_context`). One command back
  to its default, or everything. A default bound in two spellings (`cmd-&` and
  `cmd-shift-7`) is one key and is changed as one; a new key that needs two spellings gets
  both.
- **A row of the list is one key to the user.** A command bound in several key contexts
  that are one place (the find bar binds "case sensitive" for the bar and for its field,
  the assistant's chats bind their keys in four) has one row
  (`shortcuts::Shortcut::targets`), and a change of the row is made in all of them
  (`workspace/shortcuts.rs`). A command with keys in several places (undo: the text, the
  comment list) has a row in each. The rows are generated (`shortcuts::collect_shortcuts`)
  from what is installed: the commands of the palette in their categories, then the keys
  without a command (moving the caret, the keys of a text field, of the dialogs and of the
  list itself; the menu bar's Hide, Hide Others and Minimize), named after their actions
  (`shortcuts::KEY_GROUPS`, `KEY_TITLES`). **Every key context of the keymap is in exactly
  one row** (`tests/suite/keybindings.rs`,
  `every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file`): a namespace of actions that
  is bound and not in `KEY_GROUPS` fails that test. With every part of the app shown the
  list has 383 rows (382 commands in places and the `/` of the insert menu, which is no
  binding) and the shortcuts file 408 entries, one for each key context: 13 rows stand for
  several key contexts, 26 more than there are rows. Without the assistant: 353 rows, and
  the same 408 entries. (Counted in a scripted run for 0.8.0: `keys:cmd-/; state` has
  `rows=N`, and "Export…" writes the file; the sixteen zoom commands are among them, since
  0.6.0 nine more without a key: Print and Cancel printing, "Anywhere", and the seven
  commands of an image, "In the text"; and since 0.8.0 the eleven of pages: Import page…,
  Cancel adding pages, Show the report of the pages added last, Reload page, Copy page
  address and Open page address in browser "Anywhere", without a key; Show the page's
  context menu, Find, Find and replace, Find next and Find previous "On a page", with the
  keys of the text's.)
- **A place name is a promise.** Two rows under one place name ("In the library") are in
  the same place to whoever reads the list: a key of the one given to the other is "taken"
  and never "used in another place", in the question about a key and in the plan of an
  import alike. The key contexts alone do not say that (the library binds the arrows
  wherever the keyboard is in it and `r` only in the tree while nothing is typed: the two
  overlap, and by the contexts the arrow's command would "keep the key in the rest of its
  place", which is the filter field). So the keymap is told the rows and their place names
  with every change (`keymap::refresh_listed`, `shortcuts::listed_rows`,
  `writer_keymap::Row`), and judges by them: under one name, the command that loses where
  both apply has lost its key. Key contexts that never have the keyboard together have
  place names of their own, also inside one panel ("Confirming in the comment list", "In
  a question of keyboard shortcuts", "In a chat's conversation", "In the header of the
  side chat", "In the context row of the side chat", "On a button or a switch", "On a
  segmented control", "In a form"); a command of the assistant that works anywhere in a
  chat keeps one row "In the assistant's chats". The tools around the text bind their
  commands (find, the link, the format menu) on a key context of their own and are listed
  "In the text": `keymap::places` says so (`Places::around`), and a command of the text on
  one of their keys has taken it (0.5.0 offered to keep both for Underline on the key of
  "Add or edit link"). `two_rows_under_one_place_name_are_in_the_same_place` holds every
  pair of rows of the real keymap against the promise, and every verdict of "another
  place" against two different names: a new key context needs its lines in
  `keymap::places` and a place name in `shortcuts::Place::of_predicate` that keeps it.
- **A part of the app that is not shown.** While the assistant is not shown its commands
  are no rows, and their keys count for nothing in what the list says: the keymap is told
  which commands are not there (`writer_keymap::Absent`, set by `keymap::refresh_listed`
  from `commands::set_category_hidden`), so their keys are free for every other command,
  no row is warned about them and an import counts no collision with them. Nothing of
  theirs is changed: the bindings stay in the keymap (they do nothing, and a key of the
  user's on the same key runs: the assistant's handlers are on the window only while it is
  shown), and the user's changes to them stay stored. When the assistant is shown again
  the list as it is then says what collides (the warning on the row). An open list follows
  when the assistant is found or switched off behind it
  (`Workspace::shortcuts_rows_changed`).
- **Not changeable**, and the list says so in three lines above its rows
  (`shortcuts::MENUS_NOTE`, `workspace::NOT_CHANGEABLE_NOTE`, `workspace::ALSO_FIXED_NOTE`) and
  the read-me in a paragraph. The whole list of keys that are not bindings:
  - the grammar of Vim mode, the keys its mode menu shows (`Esc`, `i`, `:`, `/`) and its
    command line;
  - the `/` that opens the insert menu (typed into the text; the list has it as a row that
    says "Fixed", and the command can be given a real key besides) and `tab`, which takes
    the marked row of that menu (the other keys of the open menu are the list's:
    `picker::SelectPrev`, `SelectNext`, `Confirm`, `Dismiss`, so `ctrl-p` / `ctrl-n` move in
    it too);
  - everything inside an open context menu (`writer_ui::context_menu`, `handle_keystroke`:
    arrows, `ctrl-p` / `ctrl-n`, `home` / `end` / the page keys, `enter` and `space`, `left`
    / `right` for a submenu, `escape`, letters for type-ahead): the menu never has the
    keyboard, so no binding can apply in it, and its way out must not depend on the keymap.
    Only the key that opens it is a binding (any action named `OpenContextMenu`). While a
    menu is open a chord with `cmd` closes it and then runs; any other shortcut is swallowed;
  - what the pointer does with a key held: `cmd`-click opens a link, `shift`-click extends
    the selection, `ctrl`-click is a right-click;
  - `escape` while the pointer resizes or moves an image: it ends the drag and nothing
    changed (the editor intercepts the key for the length of the drag and no longer);
  - `escape` in the list of keyboard shortcuts itself when its `Close` has no key (so the
    list can always be left), and the two keys of the message window that stands in when
    the library cannot be opened (the changes live in the library);
  - what macOS keeps (`cmd-tab` and the like never reach the app).
- **Vim mode sees the keys of the text first.** With Vim mode on, every key without `cmd`
  (and that is no function key and no option chord) goes to the Vim engine before any
  binding, in the text and for commands that work anywhere while the keyboard is in the
  text; where Vim has a command on it (in Normal and Visual mode: `ctrl-b`, `ctrl-y`, the
  arrows and many more) the shortcut never runs. The keymap as data does not know this, so
  the app tells the list (`Listing::takes_keys_first`, `workspace::VIM_TAKES_KEYS_FIRST`,
  `vim_leaves`) and the question about such a key says so while Vim mode is on. The other
  way round nothing is eaten: a bare character is refused as a shortcut where text is
  typed, and the one key that is let through to the editor although Vim has a command on
  it is the key of the text's context menu.
- **What the user is told counts commands.** A command is a row of the list, never a
  single key: Italic with two keys of the user's is one, Underline left without a key is
  one, and a command bound in several key contexts of one place is one
  (`Listing::changed_commands`, `Listing::commands_among`,
  `workspace::changed_command_count`). One wording, with "1 command" and "4 commands"
  (`workspace::commands_counted`): the header of the list ("No key changed", "4 commands
  changed", and "1 more not shown now" for a changed command of a part that is hidden),
  the row of the settings ("4 commands have keys of your own."), both questions of "reset
  all" (the list's and the system dialog's say the same sentence: "4 commands go back to
  the default keys. Every change you made is removed."), the preview of an import and its
  toast ("Imported the keys of 5 commands"). The state line of a scripted run keeps
  `changed=N`. `keymap::changed_count` counts key contexts and is for tests.
- **One toast about the shortcuts at a time** (`Workspace::say_about_shortcuts`): the list
  stays open under the toast that offers Undo, the list leaves room for one toast under
  itself (`ROOM_BELOW` in `writer-keymap-ui/src/render.rs`), and a second one would lie
  over the choices of a question. A toast that names a file names it with
  `export::short_path` (the home directory as `~`, a long path shortened in the middle,
  the file name whole), and no toast draws its message past its edge: at most three lines,
  then an ellipsis (`writer_ui::toast`, `MESSAGE_LINES`; `docs/gpui-notes.md` says why
  GPUI's own line clamp does not do that).
- **Guard rails** (`writer_keymap::check`, asked by the list before anything changes): a
  bare character or shift with one is refused where text is typed; a key macOS keeps
  (`cmd-tab`, `cmd-alt-escape`, ...) is refused, one it uses unless switched off
  (`cmd-space`, the screenshots) is warned about. A key another command has in the same
  place is "taken": the user takes it from that command or cancels. The same key in
  another place is allowed, and the list says who keeps it where both apply ("Keep Both").
  What a place is, the app tells: in `keymap::places` (which key context can lie inside
  which, which is only around another, where text is typed) and with the rows of the list
  and their place names ("A place name is a promise"). A new key context of a crate goes
  into both. The one spot where `keymap::places` is not exact is written there ("What is
  not exact": the library's keys "wherever the keyboard is in it" against a key of the
  window, in the filter field).
- **What the list says about a key** (`writer-keymap-ui/src/words.rs`, `verdict_words`):
  the question has a heading that says the most important thing about the key, with a mark
  for how it is to be read (`mark=` in the state line), in this order: it cannot be used;
  it is taken; macOS uses it; Vim mode sees it first; it would not work everywhere
  (another command keeps it where both apply); it is used for typing; it types a special
  character; it is also the key of another command; it is free. A heading never says
  "free" above a sentence that warns, and under a warning the marked choice is "Press
  Another Key", so `enter` does not take a key the user was just warned about. A command
  bound in several key contexts of one place is named once. The field above the list
  searches by name, place or key: words that stand together come first, one typing mistake
  is forgiven when nothing else matches, and a whole key (`cmd-b`, `⌘B`) finds that key.
- **Nobody is locked out.** The list works with the pointer alone, and "Reset All Keyboard
  Shortcuts…" is in the menu bar and the palette, where it asks in a system dialog that no
  key of the app can break.
- **Storage.** One setting of the library, `keyboard.shortcuts`
  (`{"version": 1, "changes": [{"command", "context", "default", "keys"}, ...]}`): only what
  differs from the defaults, with the defaults of the moment beside it, so later defaults
  arrive by themselves. Read first of all in `Workspace::new`, before a key hint is drawn;
  removed again when every key is the default. An entry for a command this version does
  not have, fields it does not know and entries it cannot read are kept and written back.
  `writer_app::keymap::set_changes` installs (and builds the menu bar again);
  `Workspace::set_key_changes` installs and stores. Tests use the same two.
- **The file.** Export (the list's "Export…", the palette, the menu bar; the save panel
  suggests "The Writer Shortcuts.json") writes every command in every place with its
  title, where it applies, its default keys and its keys: a list of all keys a person can
  read and edit. **The file holds every command, the assistant's too while the assistant
  is not shown**, with the same entries in the same order whatever is shown
  (`workspace::file_catalogue`): a file means the same on every Mac, a change to a key of
  the assistant is not lost because the assistant was off when the file was written, and
  two files can be compared. Import reads such a file and shows a preview in the list: how
  many commands change, what is skipped and why (a command this version does not have, a
  key that cannot be read, a change a guard rail refuses), what would collide (by the rule
  the question about a single key judges with: a test holds the two against each other).
  A key that cannot be read says what is wrong with it, for whoever edited the file by
  hand (`KeySequence::parse_written`: a word that is no modifier, a name no key has, `+`
  between the parts, an empty text, modifiers without a key, two keys in one keystroke),
  and a key that was read is named in a reason as the list shows keys
  (`KeySequence::glyphs`). A change to a command that is not shown now is taken like any
  other and named with its place marked ("New chat (in the assistant's chats, not shown
  now)", `workspace::list_catalogue`). Only what
  differs from the `default` written in the same entry counts as a change. The user
  replaces their own changes with the file's, adds the file's to their own, or cancels.
  After an import and after "reset all" a toast offers Undo ("Undo keyboard shortcuts
  import or reset" in the palette does the same).
- **Scripted runs.** `keys:cmd-/` opens the list; `type:bold; keys:enter` waits for a key
  on the row, the next `keys:` is the key, `keys:enter` takes the marked choice. `state`
  has `keys=[changed=N]` and, while the list is open, `list=[mode=list|recording|question
  rows=N changed=N selected="..." keys=[...] default=[...] clash="..." question="..."
  mark=... says="..." choices=[...]]` (`clash`: the warning on the selected row; `mark`:
  how the heading of a question is to be read).
  `shortcut:editor::ToggleBold=cmd-alt-b` gives a command its keys without the list
  (several: `cmd-alt-b|f5`; `none`; `default`; `@In the find bar` for a command listed in
  several places; no guard rail is asked). `shortcuts:import:<path>` reads a file and shows
  the preview (the open panel cannot be driven), `shortcuts:press:<label>` presses a
  control of the open list by its label (a choice of the preview: `Import`, or `Add to My
  Changes` and `Replace My Changes` when the user has changes of their own, `Cancel`; a
  control of the list: `Import…`, `Export…`, `Reset All…`, `Changed Only`),
  `shortcuts:reset` and `shortcuts:undo` do what they say. `menukey:Bold` prints the key
  the real menu bar shows for an item, before and after a change. An export goes to
  `THE_WRITER_EXPORT_DIR`. A test names the file an "Import…" takes with
  `workspace::choose_import_file_beforehand`.

## Zoom

The view of the text is drawn larger or smaller, in flow view and in page view: type,
spacing, indents, the sheets with their margins, shadows and page numbers, all together.
Nothing of the document changes: text sizes stay what they are in points, nothing is marked
as edited, undo knows nothing of it, an export is the same. The interface around the text
does not zoom. Where it lives: `crates/writer-editor/src/zoom.rs` (what a zoom is, setting
it, keeping the place), `crates/writer-app/src/zoom.rs` (the commands and their keys, the
stored form, how large the display is), `workspace/zoom.rs` (the handlers, the status bar
item and its menu, storing), `pinch.rs` (the trackpad).

- **What a zoom is** (`writer_editor::Zoom`): a percentage from 50 to 400, "fit width" (the
  sheet, or in flow view the column, as wide as its room), "fit page" (one whole sheet), and
  "fit width up to N %". The three fits follow the window. Zoom in and out step through 50,
  67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400 (`ZOOM_STEPS`), from whatever
  the view shows (from a fit of 163 %: in is 175, out is 150).
- **What 100 % is**: one point of the document is one pixel of the window (a logical pixel,
  two device pixels on a Retina display), in both views. Pages and TextEdit draw their 100 %
  so, and by everything that could be found Word for Mac does too (Word for Windows draws
  96 to the inch, a third larger). On today's displays that is much smaller than paper: at
  127 pixels to the inch an A4 sheet at 100 % is 12 cm wide. That, and not the layout, is
  why a page looked small.
- **The layout never sees the zoom.** It is computed in the document's own points
  (`crate::layout`: no scale in `LayoutStyle`, `PageGeometry` is the page setup and nothing
  else), and one transform (`layout::View`) puts layout coordinates into the window when a
  frame is planned, a geometry query is answered or a pointer position comes in. Glyphs are
  painted one by one at their shaped place times the zoom, in their font at its size times
  the zoom (`paint.rs`). So in page view the same words stand on the same lines and the
  same lines on the same pages at every zoom, by construction, and a zoom shapes nothing
  again (`tests/zoom.rs`: `the_same_words_on_the_same_lines_and_pages_at_every_zoom`).
- **Page view**: a true zoom of the sheets. A sheet wider than its room does not shrink
  (the clamp of 0.5.0 that only ever made a page smaller is gone): the view scrolls
  sideways, with two fingers or with shift and the wheel, a move that goes mostly up and
  down leaves the sheet where it is, and the caret is kept in view while typing. When the
  room of such a sheet changes at the same zoom (the comments or the side chat open or
  close, the sidebar, the window), the caret is brought back into view sideways, or a
  selection on one row whole (its start where it does not fit), and nothing moves up or
  down; a caret that is not on screen pulls nothing (`PendingScroll::Sideways`, asked for
  in `Editor::install_layout_env`).
  **Flow view**: the column is 700 points wide and grows with the zoom until it fills the
  width there is; beyond that it holds fewer points and the text reflows. Flow view never
  scrolls sideways. Compact editors (a comment being written) do not zoom.
- **Everything the editor says in window coordinates is at the zoom**: `caret_bounds`,
  `position_bounds`, `range_bounds`, `selection_bounds`, `position_at_point`, the rectangle
  macOS gets for its input windows, `scroll_offset` and `scroll_by_pixels`. So whatever is
  anchored through them follows by itself (the tools, the context menus, the format bar,
  the quick chat, the comments, Vim's caret and its motions by screen row, spelling and
  find marks). **Never work a position out from points and a line height: ask the editor.**
- **Zooming keeps the place.** From the keyboard and the menus the line the caret is on
  stays where it is on screen (`Editor::set_zoom`); with `cmd` and the wheel and with a
  pinch what is under the pointer stays under it (`Editor::zoom_by`; sideways only where the
  sheet is wider than its room, otherwise it stays in the middle).
- **Each view has its own zoom**, kept in the library's settings: `view.zoom.flow` and
  `view.zoom.page`, as text (`"125"`, `"fit-width"`, `"fit-page"`). A fit is kept as a fit
  and follows the window again after a relaunch. Only the zoom the writer changed is
  written, 300 ms after the last change (the wheel makes many), and when the window closes
  or the app quits. Flow view starts at 100 %.
- **What a page opens at before a zoom was ever chosen for it**: as large as the paper it
  stands for, or as wide as the window where that is narrower ("fit width up to life size",
  `zoom::first_page_zoom`; it follows the window and is not stored). Life size is the
  display's: `CGDisplayScreenSize` and the display's width in pixels give pixels to the
  inch, over 72 (177 % on this Mac's built-in display at its default scaling, 210 % at
  "More Space"). `main` asks this Mac (`zoom::use_this_mac`); a test names a density
  (`zoom::set_display_density`) or has none, and with none a page opens at 100 % (still
  as wide as its room where the window is narrower, which is all a page did before 0.6.0)
  and there is no "Life Size" to choose.
- **That first zoom is called "Automatic"** (`zoom::Automatic`,
  `Workspace::zoom_to_automatic`): the item of the menus is checked while page view has it
  (`Zoom::FitWidthUpTo`), and choosing it brings it back and removes `view.zoom.page` from
  the library (also a zoom of page view that was still waiting to be written), so the next
  launch opens a page the same way. It is for page view only: the menu of the status bar
  does not have it in flow view, and from the palette or the menu bar it says there what it
  is for and changes nothing (as "Fit Page" does). No key by default. In the palette and
  the list of shortcuts it is "Zoom to automatic (page width, not larger than paper)": a
  command has no line for an explanation, so the title carries it
  (`zoom::AUTOMATIC_TITLE`).
- **How it is reached.** Keys: zoom in `cmd-+` (and `cmd-=`), zoom out `cmd--`, actual
  size `cmd-shift-0` (bound in both spellings: `cmd-)` is what a real keyboard reports).
  Palette: "Zoom in", "Zoom out", "Zoom to actual size" (the menus say "Actual Size"; the
  title has "zoom" in it so that the list of shortcuts, which searches titles and not
  keywords, finds it with the others), "Zoom to fit width", "Zoom to fit page",
  "Zoom to life size", "Zoom to automatic (page width, not larger than paper)" and "Zoom
  to 50 %" ... "Zoom to 400 %" (no keys by default; the user can give them one). Menu bar:
  View > Zoom In, Zoom Out, Actual Size, Fit Width, and Zoom To > Automatic, Fit Page,
  Life Size and the steps. Pointer: the zoom in the status bar ("125 %"), a
  click or a right-click opens the same choices with the one in force checked
  (`MenuKind::Zoom`); `cmd` with the wheel; a pinch on the trackpad. The commands are only
  there while a document is open.
- **The pinch** is AppKit's, not GPUI's: gpui 0.2.2 has no magnify event, so `pinch.rs`
  installs a local monitor for magnify events (and no others: scroll events never reach
  it), which hands each event on untouched after reading how much and where, through a
  channel, to `Workspace::pinch`. The monitor is installed in every run of the real
  binary; `THE_WRITER_PINCH=off` in the environment keeps it from being installed. **No
  script can pinch, so no real magnify event has been read by anything but fingers.** What
  stands in for that: a test compares the return types of the seven methods the monitor
  calls (`type`, `window`, `magnification`, `locationInWindow` of `NSEvent`, `windowNumber`
  and `contentView` of `NSWindow`, `frame` of `NSView`), as the Objective-C runtime of the
  Mac has them, with the types `pinch.rs` reads; `pinch::stand_in::Event` is an object of a
  class declared with the runtime that answers the four messages of an event with those
  types, and tests hand it to the block the way AppKit calls a block (`Monitor::hand`) and
  see what is read and the zoom follow (`pinch::follow`); `zoom:pinch:<magnification>` in a
  scripted run hands one whose window is the real `NSWindow` to the very block AppKit
  holds. The stand-in ends the process when `magnification` is asked of an event that is
  not a magnify event, as AppKit's exception would. The x86_64 half is read, not run
  (`objc` 0.2 takes `objc_msgSend_stret` there for the `NSRect` and not for the `NSPoint`).
- **Scripted runs**: `zoom:in`, `zoom:out`, `zoom:actual`, `zoom:fit`, `zoom:page`,
  `zoom:life`, `zoom:auto` ("Automatic"; `state` then has `fit-width-up-to-<life size>`
  again), `zoom:125`, `zoom:by:1.25` (as a pinch at the middle of the text),
  `zoom:pinch:0.25` (the same through AppKit's monitor and a stand-in for the event; the
  zoom follows a moment later, so `wait:` before `state`), `zoom:sideways:300` (scrolls a sheet that is wider than its room); `menu:zoom` opens the
  status bar's menu; `state` has `zoom=[page 200% sideways=144/470 life=210%]`.
- **What it costs** (release build, the generated book of 10,000 paragraphs, measured for
  0.6.0 with `keycost:cmd-= ...` and `typecost:` after `zoom:150`): a change of zoom from
  the keyboard reaches the picture in 8 ms in page view (33 ms at worst) and in 14 ms in
  flow view where the column has to reflow (42 ms at worst); the main thread kept a 4 ms
  timer waiting 3.4 ms at most. A typed key reaches the picture at 150 % in the time it
  takes at 100 %: page view 10.0 and 9.1 ms at 100 %, 9.9 and 9.9 ms at 150 %; flow view
  9.4, 9.9 and 6.5 ms at 100 %, 8.0, 8.7 and 6.7 ms at 150 % (medians of 20 keys, the two
  zooms in turns in one run; the frame itself wanders between 2 and 10 ms from run to run).
- **How much a page holds** does not depend on any of this; it is paper, margins, font,
  size and line spacing. `pagefit:<font>|<size>|<line spacing>|<a4 or letter>|<file>` lays
  a text out and counts. Per line the page holds what CoreText puts on it, character for
  character, in Times New Roman 12, Arial 11, Georgia 12 and Charter 12 on A4 and Letter
  (91.8, 90.0, 83.1 and 83.1 characters on A4). Per page it differs from Word only through
  what "line spacing" means: here a multiple of the text size, rounded to whole points
  (`LayoutStyle::line_points`), in Word a multiple of the font's own line (1.15 to 1.22
  times the size).

## Printing

"Print…" puts the open document on paper as page view shows it, through the macOS print
panel. Where it lives: `crates/writer-editor/src/print.rs` (the print pass: the pages as
data), `crates/writer-print` (the PDF writer, the PDF reader for tests, the print
operation), `crates/writer-app/src/printing.rs` (the job and where it goes),
`workspace/print.rs` (the command, the slices, the note, Cancel).

- **What is printed**: the document as it is in the editor at that moment, unsaved text
  included, in page layout: its paper, orientation, margins, font, size, spacing, page
  breaks, page numbers in the footer. Text in its fonts with bold, italic, underline,
  strikethrough, highlights, text colours, the chips of inline code, links as coloured
  underlined text (not clickable), bullets, list numbers, to-do boxes and ticks, the quote
  bar, the code panel with its language, dividers, page numbers. Real size (a point is
  1/72 inch), no scaling, Paper's colours.
- **What is not**: the caret, the selection, spelling marks, find marks, the tints of
  comments, the desk, the shadows and borders of the sheets, the "Page break" marker, the
  placeholder of an empty document, the scroll bar. A test prints the same document plain
  and then in flow view at 200 % in Night with a selection, a comment, an open find bar
  and a marked misspelling, and holds the two PDFs equal to the pixel
  (`tests/suite/print.rs`, `what_the_window_shows_is_not_on_paper`).
- **A right-to-left paragraph that wraps** is not laid out right in page view: its rows
  are cut by logical bytes and each paints a stretch of the visual line, so a row can run
  past the edge of the sheet. Paper has the same rows as page view, not other ones
  (`crates/writer-editor/src/print.rs`, the module docs). Text that runs one way is not
  affected.
- **The layout never comes from the window's view.** `Editor::print_pass` takes a copy of
  the document and lays it out with a layout of its own, as page view's branch does, at
  zoom 1 in Paper's colours; the only thing it takes from the window is the text system
  (GPUI has none without a window). So flow view, a zoom and Night print the same pages,
  and what is typed after "Print…" was chosen is not in them. The contract the tests hold:
  the PDF has `Editor::page_count()` pages of `PageSetup::page_size()`, and on each the
  text page view has on it. **Compare only once the editor's layout is settled**
  (`!editor.layout_pending()`): before that the editor's own page count is an estimate
  for blocks that were never on screen, and the pass estimates nothing.
- **How it is reached**: File > Print…, "Print…" in the menu of the title, "Print" in the
  palette, `:print` in Vim. **No default key** (`cmd-p` is Quick open, `cmd-shift-p` the
  palette, `cmd-opt-p` page view); the user gives it one in "Keyboard shortcuts", where it
  and "Cancel printing" are rows "Anywhere". Only while a document is open.
- **The seam** (`printing::PrintTarget`, an app global like the spell service):
  `SystemPanel` (the print panel as a sheet on the window; `main` installs it in a launch
  that is not scripted), `Recorder` (tests: `printing::record(cx)` before the workspace
  opens, every job is kept), `Directory` (a scripted run: "<title>.pdf" into
  `THE_WRITER_EXPORT_DIR`, or a toast that a scripted run has no panel). **Without a
  target a workspace has no Print**: no handler, the menu bar's items greyed out, no item
  in the title's menu. **No test and no scripted run opens the print panel**, and nothing
  but `printing::show_panel` can: only a person can answer it.
- **A job** (`Workspace::print`): the pass is made, the job is "preparing", the pictures of
  the document are read in the background, and a task of
  the main thread makes the pages in slices of 8 ms (`printing::set_slice`; the laying out
  and the drawing of pages into the PDF together, `layout_share` in
  `workspace/print.rs`; pages that are laid out wait in a queue, a slice draws at least one
  and more while it lasts, and no more blocks are laid out while four pages wait)
  with a wait of a millisecond on a timer in between, which is a turn of the main thread.
  The PDF writer is not `Send`: the PDF is made on the main thread. After 0.4 s a note says
  "Preparing to print …" with the button Cancel. **Cancel is the command "Cancel
  printing"** (File > Cancel Printing, the palette, no key by default; it has a handler
  only while pages are prepared), because a toast's button has no way in from the
  keyboard. **The note does not go while the pages are made**, or the panel would come up
  unannounced: "Dismiss notifications" and a click on the note (which takes any note away)
  cancel too, and a note pushed out by four newer ones (`writer_ui::toast::MAX_VISIBLE`)
  is shown again as the newest (`Workspace::print_note_went`, which watches the layer: a
  note pushed out leaves it full, one clicked away leaves room).
  While a job is preparing or in the panel a second "Print…" does nothing. A test holds a
  job in "preparing" with `printing::set_slice(Duration::ZERO)` (a slice is then one
  block, and the slices are a millisecond of the test clock apart) and in the panel with
  `printing::hold_the_panel(cx)`, a stand-in the test ends.
- **The sheet is started outside an update**, from the job's task between two awaits
  (`printing::show_panel`): AppKit calls the window's delegate while a sheet comes up, and
  that re-enters GPUI. Its end arrives inside AppKit and only notes the answer and wakes
  the task (`printing::panel_end`). `show_panel` asks `writer_print::check_selectors`
  first: a message this macOS does not understand would end the app.
- **What bites in the print operation** (`writer-print/src/system.rs`; details in
  `docs/gpui-notes.md`, "Printing"): the selector is
  `printOperationForPrintInfo:scalingMode:autoRotate:` (`scalingMode:`, not `scaleMode:`);
  the scaling is "none" and `NSPrintInfo`'s centering must stay on, only then is the page
  on the paper at its real size; the print info carries the document's paper.
- **Pictures** are on paper: before the first slice the job's task reads the stored bytes
  of every image of the document in the background
  (`images::LibraryImages::bytes_of_all`; the job is "preparing" meanwhile and Cancel
  works) and hands them to the pass (`PrintPass::set_images`; `PrintPass::images` says
  which). No picture is read on the main thread inside a slice. `Workspace::print_job`
  reads them at once. The PDF holds each picture once, as the file it is stored as (a JPEG
  stays a JPEG), however often the document uses it. An image the library does not have is
  printed as the hairline of its box with its description. **A page of photographs is laid
  out in no time and takes long to draw**: a slice that drew every page it had laid out
  held the main thread for two seconds on a document of fifty photographs, which is why
  drawn pages are counted against the slice too.
- **Scripted runs**: `print:pdf:<path>` makes the job at once (`Workspace::print_job`),
  writes the PDF and logs `print: 12 pages, 595.28 x 841.89 pt, 5214 rows, 0 rows off, 3
  images, fonts: .SFNS-Regular, Charter-Bold, Charter-Roman, made in 0.04 s` (and `print:
  missing fonts: …`). **`0 rows off` is what QA asserts**: a row is "off" when CoreText
  spaces its glyphs otherwise than the layout did, which is how a face that is not the
  layout's shows. It is judged pair of clusters by pair inside the row, not by the row's
  width: the layout shaped the whole hard line and the PDF shapes the row alone, so a tab
  on a wrapped row (its stop is counted from the start of the line) and a script face's
  contextual forms at the row's ends come out at another width in the right face; those,
  spaces (justification) and the edges of inline code are left out of the comparison. `print:job:<path>` runs the same PDF through the real print operation
  as a save job (`writer_print::run_save_job`): everything of "Print…" but the panel.
  `action:workspace::Print` is the command itself (it writes into the export directory),
  `action:workspace::CancelPrint` right after it on a book stops it; `state` has
  `print=[idle]`, `print=[preparing 40%]`, `print=[panel]`.

  ```bash
  THE_WRITER_DATA_DIR=/some/tmp THE_WRITER_SEED_DEMO=1 \
  THE_WRITER_AUTOMATION="wait:1200; keys:cmd-p; type:chap 2; keys:enter; wait:400; zoom:200; theme:dark; print:pdf:/some/tmp/a.pdf; print:job:/some/tmp/b.pdf; state; quit" \
  ./target/debug/the-writer
  # a book, in the release build:
  THE_WRITER_AUTOMATION="wait:1500; book:10000; wait:3000; print:pdf:/some/tmp/book.pdf; action:workspace::Print; stall:1500; state; quit"
  ```
- **What it costs** (release build, the generated book of 10,000 blocks and 660,000 words,
  measured when printing was added): 2,856 pages and 67,103 rows, 0 rows off, a PDF of
  7.8 MB, made in 2.4 s in one go (`print:pdf:`). In slices, as "Print…" makes it, the
  window stays live: the main thread kept a 4 ms timer waiting 11.8 ms at the median and
  13.6 ms at most while the pages were made and the spell checker ran its own slices
  beside them (5.1 ms with the spell checker alone), and a typed key reached the picture
  in 2 ms. Every 100 ms the window is asked for a frame, so that GPUI's line cache lets
  go of what the pass shaped (it holds a line until the window has drawn twice).
  A document of fifty photographs (twelve megapixels and 5.6 MB each as JPEG, 279 MB
  together; measured when images were added): 26 pages, a PDF of 293 MB, made in 1.7 s in
  one go and in 2 s in slices, in which `state` was answered every third of a second
  (a page of two such photographs is about 65 ms of drawing and cannot be divided). The
  peak memory footprint was 1.0 GB against 0.36 GB with the document only open: the bytes
  of every picture and the PDF are in memory once each until the job is handed over.
  Hundreds of large photographs in one document are beyond what this was made for.
- **Never verified by anything but a person**: the print panel itself (that the sheet
  comes up on the document's paper, its preview, Cancel giving the keyboard back to the
  text, Save as PDF from its PDF menu, a real printer, quitting with the sheet up).
  `run_sheet` was never called by a test or a script; what stands in for it:
  `check_selectors` in a test of the app's own build, the delegate called the way AppKit
  calls it (`writer-print`), and the same operation run as a save job from a test and
  from `print:job:`.

## Images

A document can hold pictures. Where it lives: `writer-core` (the block),
`writer-store/src/images` (the bytes, the collector, the copy among the backups),
`writer-image` (a file made into what is stored), `crates/writer-editor/src/images.rs`
and `mouse.rs` (drawing, the pointer, the ways in), `writer-editor-tools/src/text_menu.rs`
(the image's menu), `src/image_description.rs` (the description's popover) and
`src/format_menu.rs` (the image's rows of the format menu),
`crates/writer-app/src/images.rs` (the provider over the library, the exporters'
pictures, the collector's timers and what it says), `pasteboard.rs`,
`workspace/images.rs`; for agents "Agents", for the assistant "The assistant", on paper
"Printing".

- **The block, and what "selected" is.** An image is a block of its own, a void block like
  a divider ("Document model"): no text wraps around it. **The caret on an image's block is
  "the image is selected"**; there is no second kind of selection anywhere. The editor
  draws an outline and four handles in place of a caret (whatever the caret style: Vim's
  block caret is the outline too), `Editor::selected_image` and `caret_bounds` (the
  picture's box) answer for it, and everything that acts "on the selection" acts on the
  image: copy and cut take the picture, the alignment keys and "larger" / "smaller" work on
  it, `backspace` deletes it, typing and `enter` start a paragraph below it, `shift-enter`
  one above. The four widths, "Original size" and "Description" exist only for a selected
  image: the editor and the tools take them only then and let them travel on otherwise, and
  at the workspace root the writer is told what they need
  (`Workspace::image_command_came_this_far`, `workspace::SELECT_AN_IMAGE_FIRST`), because
  the menu bar cannot grey an item out by what is selected.
- **Width is a share of the text column, the same share in every view** (`ImageWidth`,
  thousandths): "50 %" is half the width of the text in flow view, on a page, in print and
  in a DOCX. Flow view's column is wider than a page's (700 points against 451 on A4), so
  the same picture is drawn larger there. "Original size" is the picture's own size **on
  the page** (pixels at the density its file declares; a Retina screenshot at half its
  pixels), never wider than the column and never taller than a page's text area
  (`Image::original_width`, `Image::max_width`: one definition for the app, the importer
  and agents). A new picture is centred at its original size; one that Markdown brings in
  stands at the left.
- **Where the bytes are.** A document names stored bytes by the first 16 bytes of their
  SHA-256 (`ImageId`, 32 hex digits) and holds what layout needs without them (pixel size,
  density, width): about 130 bytes of JSON per image. The bytes are stored once in
  `images.db` beside `library.db` (the row of `writer-store` above says why a second file
  and what a backup holds). The store never parses a document; it scans stored content for
  one token, `"image":"<32 hex>"` (`Store::referenced_images`): **do not rename that JSON
  key**. `library.db` is at schema 3, so a version that does not know images refuses the
  library instead of reading an image as an empty paragraph and saving that.
- **Bytes first** (product rule 4). On every way in the bytes are committed before a block
  names them: the provider's `import` answers with the image only when the write is back,
  an agent's request stores what it brought before it changes the document.
- **Nothing ever deletes the bytes of an image but the collector**
  (`writer_store::collect_unused_images`): not an undo, not a purge, not emptying the
  trash, not the pruning of versions, not the `writer` tool. It runs in the app only
  (`images::collect_in_background`, started by `main`), in the background, in two halves:
  - **Once per launch at most it deletes**: 20 s after the launch (`images::COLLECT_DELAY`),
    when the last run is a day old (`images.collected_at` in the library's settings). It
    stamps every image that no stored content names (documents live and in the trash,
    versions) and deletes one whose stamp is older than 30 days **and** older than the
    oldest kept backup of the library. At a launch nothing but stored content can hold an
    id: the undo history, a Vim register and a cut that waits to be pasted are empty. A
    launch that does not run it brings the copy of the pictures among the backups up to
    date instead.
  - **Once a day while the app stays open it only looks** (`mark_unused_images`,
    `images::LOOK_EVERY`): it clears the stamp of every image that is named and stamps
    what is not. It deletes nothing. Without it a stamp could grow old on an image that is
    named again without the store being asked: an undo brings back an image that was cut
    and saved away, and nothing tells the store.
  - Who else can hold an id and why each is safe is the table at the top of
    `writer-store/src/images/collect.rs` (the clipboard and an old answer of the assistant
    come in through `touch_image`, which clears the stamp; a backup is safe by the second
    condition). The timers are the app's (`cx.background_executor().timer`): a test drives
    them with its clock.
  - Deleting documents therefore frees picture space only after 30 days, and only at a
    launch.
  - **After each run the writer is told** what the store reported, in a note that stays
    twelve seconds: a copy of the pictures among the backups that could not be brought up
    to date (once a launch; stderr every time), and each damaged file of pictures the store
    set aside (`writer_store::images_moved_aside`; once for each file, by its path, with
    "Show in Finder"; the setting `images.moved_aside_told` keeps the ones told of that are
    still there). One set aside by a read in between is told of at the next run.
- **A missing picture.** A document can name an image the library does not have. The block
  stays everywhere: the editor shows a placeholder of the picture's size that says "Missing
  image" (the size is in the block, so nothing moves when the picture is there), a save
  writes the block as it was, exports write the description, a print its box, agents read
  the line. A picture that is only in `backups/images.db` is found there and written back
  (`ImageFile`): a library whose `images.db` was lost shows its pictures again.
- **The provider seam** (`writer_editor::ImageProvider`, set per editor;
  `Editor::has_image_provider`). The editor does not know the library. Its provider hands
  out the stored bytes of an image (`bytes`), stores what the user chose, pasted or
  dropped (`import`), asks for files (`choose`), looks at the pasteboard (`pasteboard`) and
  is told when ids came in from outside, with a pasted fragment (`touch`). Without one
  every image is a placeholder and nothing can be inserted: compact editors (a comment
  being written) and the editors of the assistant's answers have none.
  `writer_editor::FakeImages` is the provider for the tests of every crate that shows an
  editor (pictures in memory, `hold` / `release` for one that is on its way, the "panel"
  and the "pasteboard" answered beforehand). The app's is `images::LibraryImages`, and
  the app's own tests use it, not the fake.
- **Nothing large on the main thread** (`images::LibraryImages`). For a library on disk the
  provider has a connection of its own to `images.db` (`writer_store::ImageFile`, behind a
  mutex that only background tasks lock): the bytes of a picture are read there, and a new
  picture is read from its file, converted (`writer_image::prepare`), hashed and written
  there (up to 16 MB with a sync), and only then does the block appear. A library in
  memory (tests) has one connection and does the same at once. A print reads its pictures
  the same way before its first slice. **What still runs on the main thread**: an agent's
  picture (it is part of the request; the cost is in "Build and test") and an export, which
  reads every picture of what it exports through the library's store, as it reads the
  documents. The editor asks again for images it found missing after anything that can
  have brought their bytes: an agent's change, a restored version, a picture stored
  through the provider (`Editor::images_changed`; `LibraryImages::on_stored`).
- **A file that cannot be used** says so in a toast, in the words of
  `writer_image::PrepareError::sentence` about the file's name, or about "The picture on
  the clipboard": three files of one gesture by name, the rest counted ("2 more files
  could not be used."). A gesture is told from the next by the app's clock
  (`cx.background_executor().now()`, so a test decides).
- **Formats and limits** are `writer_image::prepare`'s: a PNG, a JPEG or a GIF that is
  upright, at most 4096 pixels on its long edge and 16 MB and carries no GPS position is
  stored as it came, byte for byte, so the same file twice is stored once. HEIC, TIFF,
  BMP, WebP and AVIF are converted (JPEG, or PNG when transparent), a larger picture is
  made smaller, a file over 100 MB, an SVG and a PDF are refused. A stored picture keeps
  what its file says about itself except the GPS position (camera, date, author, a place
  name somebody typed), and that travels into a DOCX and a Markdown picture folder; the
  read-me says so. An animated GIF is kept whole and shows its first frame, on screen and
  on paper.
- **Decoded pictures** (the editor's `images/cache.rs`): a picture is decoded in the
  background at a *level*, its natural size halved as often as it can be while it stays at
  least as wide as the device pixels it is drawn on, because GPUI's atlas samples without
  mip maps and a twelve megapixel photograph is 48 MB of pixels. Levels are keyed by
  `ImageId`, so two blocks with one picture share them; they are bounded at 128 MB of
  pixels per editor, and a level that leaves the cache is handed to `Window::drop_image`,
  or its texture would stay in the atlas for the life of the window
  (`docs/gpui-notes.md`, "Images").
- **A resize with the pointer is a preview, not an edit**: while a handle is dragged the
  layout shows the new width (`DocumentLayout::set_image_preview`) and the text below
  follows, the document is not touched, and the release is one transaction. `escape`
  during the drag ends it with nothing changed.
- **Copy, paste, drop.** A clipboard item is text or one picture, never both. Copy or cut
  with the caret on an image puts **the picture** on the clipboard (it pastes into other
  apps) and remembers the block; pasted back here it comes with its size, alignment and
  description. A selection of text and pictures is copied as its text with the blocks
  beside it, as before. A paste takes, in this order: what was copied here, a picture
  GPUI's clipboard shows (a screenshot, Copy in Preview), a picture only the pasteboard
  shows, text. The third is the app's own read of `NSPasteboard` (`pasteboard.rs`): GPUI
  hands over the text of a pasteboard whenever there is text, which leaves out an image
  file copied in the Finder (its name is the text), "Copy Image" in a browser (its address
  is the text) and a HEIC. What counts as a picture is a pure function
  (`pasteboard::decide`): a file address whose file has an image's extension, else picture
  data when there is no plain text beside it or the text is one line that is an address;
  cells copied in a spreadsheet carry a picture of themselves beside their text and stay
  text. "Paste" in the text's menu asks `Editor::can_paste`, which asks the pasteboard
  too. A drop is `Editor::drop_files`, at the boundary between blocks nearest to the
  pointer.
- **What GPUI cannot do**: a picture dragged out of a browser or out of Photos does not
  arrive (the window registers for file drops only; a file from the Finder works), the
  open panel has no file type filter (any file can be chosen, and one that is no picture
  is refused with its sentence), a custom element cannot paint a picture half transparent
  (a quad of the surface's colour over it dims a dragged picture), and there is no
  accessibility interface: the description of an image is not read out by VoiceOver in the
  app. It serves where it can: the alternative text of a picture in a DOCX, the alt text
  in Markdown, what agents and the assistant are told, the preview of a version.
- **Exports** (`export.rs`, `images::StoreImages`): a DOCX embeds the pictures at their
  size on the page; a Markdown **file** links them and they are written into a folder
  beside it, `<name> images/image-<8 hex>.<ext>` (the folder is named after the file as it
  was suggested and keeps that name when the file is renamed in the save panel: the links
  are relative; the toast says where the pictures are); Markdown for the clipboard, for
  agents and for the assistant names each image `![description](image:<id>)`; plain text
  has the description on a line of its own. An export whose pictures cannot be read from
  the library fails and writes nothing ("Could not export: its pictures could not be read
  from the library (...), so nothing was written"; `export::pictures_not_read`). Counts,
  the character limit, find, spelling
  and the library search see an image as an empty block.
- **The version history** shows an image as its description, or "Image", in the muted
  colour: its preview is text made of the blocks, not an editor, and has no picture.
- **Scripted runs**: `image:<path>` inserts a file (the open panel is never opened),
  `image:select:<n>` selects, `state` says `images=N` and what the selected image is; the
  rest is keys (`keys:cmd-shift-.`, `keys:alt-up`, `keys:backspace`,
  `keys:ctrl-enter; choose:Description…`), an export into `THE_WRITER_EXPORT_DIR` and
  `print:pdf:`. There are no pointer steps in the app (the editor's demo has them:
  `cargo run -p writer-editor --example demo`).
- **Not in this version**: captions, cropping, text that wraps around a picture, a change
  of an image's size by an agent, removing a description through an agent's Markdown (an
  empty alt text keeps the old one). A selection of text and pictures pasted into another
  library arrives with "Missing image" where the pictures were (one picture copied by
  itself travels as a picture and arrives whole).
- **Checked by hand only, or by nothing**: a file dragged in from the Finder (the two drop
  listeners of the editor were compiled and never run by a test: nothing can drag from the
  Finder but a hand), what real apps put on the general pasteboard (the read itself is run
  against a pasteboard of the test's own, `pasteboard::private::Pasteboard`, and the
  decision is tested), resize and move with a real pointer in the app (the editor's tests
  call what the listeners call), the open panel, a HEIC from Photos, and everything on an
  Intel Mac.

## Pages

A page is a web page (an HTML file or a folder, with its styles, scripts, fonts, pictures
and sounds; a claude.ai artifact) kept in the library beside the documents, shown rendered
in a WebKit view in the window and working **offline for good**: everything it needs is
downloaded once, when it is added, and stored; showing it never touches the network. A page
that cannot be kept whole is not kept at all, and the reason names what is missing. The
design, with its reasons and its review, is `docs/pages-design.md`. Where it lives:
`writer-store` (the row and the files), `writer-page` (the bundler), `writer-web` (WebKit),
`writer-agent` / `writer-cli` / `writer-mcp` (agents, "Agents"), `crates/writer-app/src/pages.rs`
(the page engine, the page source, the seams of the import), `workspace/page.rs` (the open
page), `workspace/page_import.rs` and `page_report.rs` (adding pages in the app),
`page_export.rs` (its export).

- **In the library**: a row of `documents` with `kind = 'page'` (schema 4), in the tree among
  the documents with a globe on its row, in the trash, the versions, the search (its text)
  and quick open like a document. Its content is the manifest (`writer_page::Manifest`: the
  served files and the original ones by path, file id and media type, the pieces fetched
  from the web, the source address, a fingerprint of the input, when it was taken, what the
  check found, and the text the page showed). Its files are in `images.db`, table `files`,
  stored once each by the hash of their bytes (`FileId`; a font that ten pages use is one
  row), kept by the copy among the backups and deleted by nothing but the collector, as
  pictures are. **The store names no file it does not have**: `create_page` and
  `save_page` refuse a manifest that names a missing file (`FilesMissing`), and the two
  kinds never save over each other (`WrongKind`). The title is the page's `document.title`
  after its scripts ran, else its first `<h1>`, else the name of the file or folder; a
  rename sets the flag of a chosen title, which an update keeps.
- **Whole or nothing** (`writer_page::bundle`, then the check, then `prepare`): every
  reference of the HTML, SVG, CSS and JS is found and every piece on the web is fetched once
  (`curl`, public addresses only, redirects followed and checked hop by hop, 30 s an
  address, 180 s a page) and stored under `_external/<host>/…`, the references rewritten to
  relative paths. Then the served files are loaded offscreen with the network blocked: a
  `Content-Security-Policy` on every answer names every request that would leave, in a
  content world of the check's own that the page's scripts cannot reach. A page that asks
  for anything it does not have (a piece that could not be fetched, a path it lacks, data
  from a service, an embedded site, a frame that cannot be watched) is refused, the first
  five problems named, "Nothing was saved." The check never scrolls, so a picture or a
  frame under `loading="lazy"` far down the page would never be asked for: the probe makes
  each one eager as soon as it is in the document, and the bundler names a local one that
  is not among the page's files before the check ("the page asks for pics/late.png when it
  is scrolled to (loading=lazy), which is not among its files"), for a folder and a single
  file alike. **A server's Content-Type is not believed on its own word** (Google Fonts
  answers some `/l/font?kit=…` addresses with `text/html` for WOFF2 bytes, and 0.8.0 stored
  those fonts as HTML files of the page, whose bytes the check read as text):
  `bundle.rs`, `what_a_piece_is`, takes the type from the bytes' signature first
  (`writer_page::sniffed_type`: fonts, pictures, sound, film), then from where the page names
  the piece (a `url()` of `@font-face` and a preload as a font are never text; a script, a
  style sheet) and its ending, and the header only where those say nothing against it; bytes
  that are no text are never HTML, also for a file of the page (`served_type`) and when
  writer-web serves a stored one. **A piece from the web is never a document of the page**:
  only the entry and the page's own HTML files are loaded by the check and read for text
  (`Bundle::html_paths`, `write_served_dir`). What is not fetched and is no reason (an
  address in a string, `window.claude`) is a warning. Limits: 32 MB a file, 64 MB a page,
  2,000 files, 500 pieces from the web, 101 HTML files, 120 s of check. The text of a page
  is what it shows (`innerText`) and then what it holds unshown (closed `<details>`, hidden
  chapters), for search, agents and the assistant; what layout shows apart stays apart (a
  tag after a word, a chip, the gutter of a listing, the labels of an axis: each junction
  is measured and marked before `innerText` is read, only white space is added). The text
  is that of every HTML file of the page the check loads (the entry first, the others under
  their titles; never a piece from the web), so the words a page counts (status bar, `writer list`) can be many more than
  the entry shows (Nimbus Landing Variants: 54,000 words, of which 6,900 in its
  `index.html`), and a search can find a page by a word only another of its HTML files
  has. `page:make` stores the static check's text (tags stripped, every file), whose
  counts differ from an import's.
- **A single file is never a walk of its folder**: an `.html` file is taken with the local
  files its references name, never a hidden file nor a document it only links to, and a
  literal path to a file beside it (a narration a button plays, also in an event handler,
  which is read as a script) is named in a warning, "add the folder". A folder is taken
  whole; opened with another entry it is another input. A page is exported as a folder
  whole or not at all.
- **Adding pages in the app** (`workspace/page_import.rs`): File > Import Page… (the open
  panel, files and folders, several; with a page engine only), "Import Page…" in the menu
  of the library and of a folder row, and `.html` / `.htm` files or folders dropped on the
  sidebar (the folder under the pointer), on an open page (the web view takes no drops, so
  GPUI's view and the page element get them) or on the text (the provider of pictures hands
  web pages to the import, `LibraryImages::set_page_import`). What is no page is left out
  and counted. A gesture is read and fetched on a thread of its own, checked by the page
  engine (WebKit's check on the main thread, from the task between two awaits), judged in
  the background, its files stored in the background through the pictures' connection
  (`LibraryImages::put_files`) and its row written on the main thread
  (`Library::create_page`). The first page of a gesture opens unless another item was
  opened meanwhile. After 0.4 s a note says what is going on with Cancel, which is the
  command "Cancel adding pages"; like the note of a print it comes back when newer notes
  push it out, and a click that takes it away cancels. **One summary per gesture** ("2
  pages added, 1 not added", "Added “Orbitarium”"), with **Show Why** when a page was
  refused, has warnings or files were left out: the command "Show the report of the pages
  added last" (also File > Show Report of Pages Added Last…, for whoever missed the toast)
  opens a dialog of the modal layer with every page and its whole sentence, and Copy (`tab`
  goes from page to page, `escape` closes). The app's import gives no source address:
  importing a file again is a new page.
- **Isolation** (`writer-web`, docs/pages-design.md section 5): the scheme `writer-page`,
  each page its own host, **its id and the moment its row was made**
  (`page-12-1731600000000`, `pages::page_host`): the views of a run share one store of
  website data, kept per origin, and SQLite gives the id of a page erased for good to the
  next row, so with the id alone a new page read the local storage, IndexedDB and cookies
  of the erased one (an update keeps the row and its origin). The CSP and a content rule
  list block every request to `http(s)`, `ws(s)`, `ftp` and `file`; WebRTC and media
  devices are off (`_setPeerConnectionEnabled:` and `_setMediaDevicesEnabled:`), and so is
  the data store's preconnect (`_WKWebsiteDataStoreConfiguration`
  `initNonPersistentConfiguration`, `setAllowsServerPreconnect: NO`, then
  `WKWebsiteDataStore _initWithConfiguration:`; without it WebKit looks up and connects to
  the host of a link a script clicks, DNS, TCP and TLS, before the policy refuses the
  navigation: a channel out of an offline page). All of these are private and required: a
  macOS without them has no pages (`writer_web::check_selectors`, `selectors.rs`; trimming
  that list drops the guard). Navigation away, forms, downloads, new windows,
  autoplay, AirPlay, camera, microphone, link previews and the inspector are off; storage
  lasts while the app runs (one non-persistent data store); no bridge to the app. **A link
  to another site opens in the default browser only when the reader opened that link**:
  the navigation's address is the address of the link under the pointer at a real
  `mouseDown:` or of the focused link at a real `enter` (asked of the page's document in
  the client's content world before the event reaches the page, through frames of the
  page's own origin, shadow trees and SVG links), within a second of the reader's click or
  key, once per act; or "Open Link in Browser" of its menu. A script's click opens nothing,
  also not one in a listener of the reader's click or key that clicks another link
  (`writer_web::policy::reader_opened`; before the review's fix, ISO-4, any click or key
  opened the gate for a second for whatever link a script clicked). An `<area>` of an image map is not found that way (the
  point is the picture's) and does not open. `alert` is a toast "The page says: …". The rule list is compiled into
  `<data directory>/WebKit/` (the tool's children: `$TMPDIR/the-writer-<uid>/WebKit/`) under
  one fixed identifier, never into WebKit's default store.
- **macOS 12.3 for pages.** `writer_web::available()` asks the version and every selector it
  sends; when it fails there is no page engine: a page shows its text with the sentence, and
  nothing can be added (the app keeps its floor of 10.15.7 for documents).
- **The open page** (`workspace/page.rs`): `Workspace::session` is `None` and
  `Workspace::page` holds it (`open_item_id` is either). The `WKWebView` is a native subview
  above GPUI's view, kept on the bounds of the page element; GPUI's focus on the element and
  AppKit's first responder on the view are kept in step every frame; every callback of
  AppKit only posts to the page's inbox, and a task of the workspace acts on it
  (docs/gpui-notes.md, "A native view in the window"). Keys: command keys reach GPUI first;
  everything else goes to the page, except a key the keymap binds where a page has the
  keyboard (`ctrl-enter`, `shift-f10`: the page's own menu as a GPUI context menu), which
  the view hands to GPUI's view. `PageView` is a place where text is typed (a page has
  fields), so no page command can be given a bare letter. A modal or a context menu over the
  page is drawn over a snapshot of it (the view hidden; the picture handed to
  `Window::drop_image` when the overlay closes, and when the page closes under it, as an
  item opened from quick open, the library search, a row's menu or the version history
  closes it: `Workspace::discard_page`, the one way a page session ends; one left behind
  is a texture the size of the page in GPUI's atlas for the life of the window); a toast gets a hole in the view (drawn and
  hit-tested: a click on its button reaches GPUI). The title and the status bar have the
  page's items (its words, its zoom 50 to 300 %, the sparkle); find is a strip in the
  find bar's key context, with Aa, previous, next and close for the pointer (none takes
  the keyboard; the press that gives the page the keyboard is on the page's area, not on
  the element that holds the strip, or a press into the strip's field would end in the
  page); the theme reaches the view; a crash of the WebContent process
  reloads once and shows "This page stopped working." the second time within 10 s. The
  commands of a text are greyed out and left out of the palette; those of a document that
  mean nothing on a page (document settings, the size and spacing of body text, take title
  from text, spelling language, save now, comments, the fits of the zoom, the quick chat)
  say what a page is in a toast from their keys and the menu bar, and the palette leaves
  them out too, with the zoom steps past 300 % (`commands_not_for_the_open_page`); it names
  "Rename document" and "Go to text" "Rename page" and "Go to page" while a page is open
  (`Workspace::page_title_of`, the old title kept among the keywords). Print is WebKit's print
  of the view through the same print target; Export writes the served files as a folder
  that opens offline, or a PDF; the version history shows a version's text and restores it.
- **Agents' pages** (`writer page add`, `add_page`, "Agents"): prepared and checked by the
  tool, never by the app. While the app is open the request carries the files as base64
  (protocol 3, one page a request, 64 MB at most) and **the thread that accepts it stores
  them before the request is queued** (`writer_agent::live::store_page_files`, through an
  `ImageFile` of its own), so the main thread writes only the row (about a millisecond for
  16 MB, "Build and test"). That thread knows the owner's switch (`live::set_switch`, from
  the setting whenever it is read or changed): with it off a request with page files is
  refused there at once and nothing is stored. The note: "Claude added “X”" (Open), "Claude
  updated “X”" (the open page reloads; no button).
- **The page engine** (`pages::PageEngine`, an app global like the print target):
  `WebKit` in every run of the real binary unless `THE_WRITER_PAGES=off` or
  `available()` fails; `Fake` in tests (`pages::use_fake`: a GPUI element draws the page's
  text, the check answers with `static_check` or what `FakePages::set_check` left, and the
  fake records what the view was asked: opened, finds, zoom, snapshots made and dropped,
  the first responder, edits, reloads, holes; `hold_checks` / `release_checks` hold an
  import in "checking"); none: a page shows its text and nothing can be added. A scripted
  run switches WebKit's occlusion detection off for its view (`pages::draw_when_covered`),
  or a covered background window draws the page blank.
- **Scripted runs**: `page:make:<folder>[|<entry>]` stores a folder that is offline already
  as it is (QA: the archive's `offline/index.html`), `page:import:<path>` adds one as the
  import does (with the real check), `page:wait`, `page:text`, `page:find:<q>`,
  `page:js:<script>`, `page:menu`; `state` has `web=[…]` and `adding=[…]`. Copy Page Address
  puts text on the clipboard: do not script it.
- **Not in this version**: a page added from an address (fetching a live site), a page's
  storage kept across launches, its downloads, `<input type=file>`, `window.print()`,
  editing or commenting a page, the quick chat on a page, the pinch on a page, a zoom kept
  per page, more than one live view, single-file HTML export, re-importing a file in the app
  to update its page, finding across the other HTML files of a page.
- **Checked by hand only, or by nothing**: keys, the first responder and the cursor in a
  real active key window, the print panel on a page, a drop from the Finder, the sound that
  does not autoplay, a link opening Safari, a page with Wi-Fi off, scrolling, the theme
  (`docs/pages-design.md`, section 13: checks 1 to 10, for a person). A picture under
  `loading="lazy"` far down a page printed offscreen (`__page-pdf`): the probe that makes
  it eager is the check's only.
- **The tests that reach the internet** are ignored and run on purpose only:
  `CARGO_TARGET_DIR=target-pure cargo test -p writer-page -- --ignored archive` bundles
  every original of the archive with the real fetcher and finds every piece the archive
  has (read only). The font files of Google Fonts are not compared by address (its style
  sheets name other files as time goes on, even moving a family between `/s/<family>/`
  and a kit): enough that the page has some where the archive has some and that no style
  sheet still names one on the web.

## Mouse

Every feature is reachable with the mouse alone (product rule 1). How it fits together:

- **The menu bar** (`crates/writer-app/src/menus.rs`) is the complete map: The Writer (with
  the keyboard shortcuts: the list, import, export, reset all), File, Edit (with
  Spelling), Format (with Turn Into, Align and Image), Insert (with Image…), View (with
  Theme), Help. While the
  assistant is shown it has its items too (Edit > Ask the Assistant…; View > Show or Hide
  Assistant, Go to Assistant, New Chat, Clear Chat), and none of them otherwise. Every item
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
  menu, and Ask the Assistant while the assistant is shown; Export and Version History on
  library rows, Copy Page Address and Open Page Address in Browser on the row of a page with
  an address, Import Page… on the library and on a folder row). The text menu has a second hook for
  what goes first (`EditorTools::set_text_menu_leading_extension`): the spell checker's
  suggestions on a marked word.
- **An image** is everything with the pointer (the editor's `mouse.rs`, the tools'
  `text_menu.rs` and `image_description.rs`). A click on the picture selects it: an
  outline and four corner handles. A
  drag of a handle resizes it (a chip says the percentage, it snaps to 25, 50, 75 and
  100 % and to its original size, the text below follows; one undo step on release); a
  drag of the picture moves it between blocks, with a line where it would go; an image
  file dragged in from the Finder shows the same line and lands there. A right-click on
  the picture selects it and opens its menu: Cut, Copy, Paste, Paste as Plain Text; Size
  (Larger, Smaller, the four widths, Original Size), Align (Left, Centre, Right),
  Description…; Insert, which on an image begins with Paragraph Above and Paragraph Below;
  Find in Document, Select All; the app's Add Comment, dimmed, because no thread can be
  anchored to an image; Delete Image. "Description…" opens a small popover under the
  picture with one field and the buttons Done, Remove (only when there is a description)
  and Cancel; a click anywhere else keeps what was typed, and so do closing the window and
  quitting with the popover open (`Workspace::keep_what_the_tools_hold`, before the last
  save: neither moves the keyboard, which is what closes the popover otherwise). The menu
  bar has the same under
  Format > Image, and Insert > Image… opens the macOS open panel (several files at once).
- **A page** is everything with the pointer: a right-click in it opens WebKit's own menu,
  cut down (no Reload, Download, Inspect, new windows) and extended with the app's items
  (Find in Page…, Reload Page, the address, Print…, Export…, Version History…; "Open Link in
  Browser" on a link); the title's chevron and the status bar's zoom have the page's menus;
  `.html` files and folders dropped on the sidebar, a page or the text are added
  ("Pages"); File > Import Page… opens the open panel.
- **The format bar** (`crates/writer-editor-tools/src/format_bar.rs`) comes up 150 ms after
  the left button is released on a selection the pointer made: bold, italic, underline,
  strikethrough, inline code, highlight and text colour (menus), link, turn into (a menu),
  the app's "Add Comment" and, while the assistant is shown, "Ask the Assistant"
  (`EditorTools::set_format_bar_extension`, set beside the text menu's hook) and "more"
  (the format menu). Each button runs what its key runs and keeps
  the selection and the bar. `editor.format_bar` (the "Format bar" switch in the settings)
  turns it off through `EditorTools::set_format_bar_enabled`. It is not a `ToolKind`.
- **The app's own menus** (`crates/writer-app/src/workspace/menus.rs`): the title above the
  text (right-click, or the chevron beside it) and the items of the status bar (the save
  state, the counts, the page indicator, the character limit, the zoom; a click or a
  right-click).
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
