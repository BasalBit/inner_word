# Whiteboards: design

Status: **built** (a383097..bc997a4, 2026-11-23), and this text now says what was built.
Written 2026-11-23 against `main` at 0c0193d (0.10.1) as the spec the builders worked from;
every name of existing code below was read in that commit, or comes from the four scout
reports of this design and their experiments on macOS 27.0.1 (that session's scratchpad,
`board/scout-app.md`, `scout-web.md`, `scout-agents.md`, `scout-excalidraw.md`; the
prototype of the bundle and its WebKit harness were in `board/proto/`). Where a fact was
only read and not run, it says so. Revised after two reviews (safety, feasibility): where a
reviewer's point was taken, the text below says what holds now; where it was not, a note
"Not taken" says why. Revised again after the build: where the build departed from the
spec, the text says the built thing, and a short note marked **Built:** says why (the
builders' reports, the reviews of each package and the final review). CLAUDE.md
("Whiteboards") is the short form of the same facts.

The feature in one paragraph. A **whiteboard** is a third kind of item in the library,
beside documents and pages: a canvas the writer draws on with the real Excalidraw
(`@excalidraw/excalidraw` 0.18.1, MIT), bundled into the app and run in the WebKit view that
pages already have, offline for good. The writer makes one with "New Whiteboard" and draws
shapes, arrows, text and pictures, turns a Mermaid diagram into a drawing, and keeps shapes
to reuse in Excalidraw's shape library, which the library of Inner Word stores; the drawing
is saved as it is drawn, kept in versions, found by its words in the library search, and
exported **as a PDF only**: one page exactly the size of the drawing and a margin, its lines
and its text kept as vectors, on its own or after the documents of its folder in the
folder's PDF. Agents make whiteboards and draw on them with the `innerword` tool and its MCP
server (shapes, arrows, text, frames, and pictures from files of this Mac; change and remove
elements by id), read them as a list of elements, export them as a PDF and trash them; the
app takes their changes into an open whiteboard without losing a stroke of the writer's.

## 0. Words used here

| Word | Meaning |
|---|---|
| whiteboard, board | A library item of kind `board`: a row of `documents` with `kind = 'board'`. "Whiteboard" is the word of the interface; "board" is the word of the code (`DocumentKind::Board`, `board:12`, `innerword board`). |
| Excalidraw | The drawing editor: npm `@excalidraw/excalidraw` 0.18.1 with `react` and `react-dom` 19.2.9, bundled. |
| the bundle | The built folder of the whiteboard's web page: `index.html`, `export.html`, `assets/`, `fonts/` (crate `innerword-board-web`, folder `dist/`). |
| the glue | Our JavaScript in the bundle around Excalidraw: `window.innerwordBoard`, the bridge, the autosave, the export. |
| scene | Excalidraw's drawing: its elements, the canvas settings that belong to the drawing, its pictures by id. |
| stored scene | The scene as the library keeps it in `documents.content` (section 2). |
| element | One object of a scene: a rectangle, an arrow, a text, a frame, a picture. It has an `id` and a `version` that grows with every change. |
| label | A text element bound to a shape or an arrow (`containerId`), drawn inside it. |
| skeleton | What an agent writes for a new element: a subset of Excalidraw's `ExcalidrawElementSkeleton` (section 7). |
| the view | The live `WKWebView` of the open whiteboard in the window. |
| the bridge | The one script message handler of a whiteboard's view: the page posts to the app. |
| a call | A function of the glue the app runs in the page (`callAsyncJavaScript`) and whose answer it waits for. |
| export page | `export.html` of the bundle, loaded offscreen in WebKit to make a PDF. |
| `FileId` | `innerword_store::FileId`: the first 16 bytes of the SHA-256 of stored bytes, 32 lowercase hex digits; the key of a row of `images.db`'s `files`. |

## Decisions at a glance

1. A whiteboard is a row of `documents` with `kind = 'board'`. `library.db` goes to schema 5,
   a step that changes nothing, so that 0.10 refuses a library that holds one; the step is
   taken by the first whiteboard made (in the transaction of `create_board`), not at the
   first open, so a library in which nobody made a whiteboard stays readable by 0.10. Tree,
   folders, trash, versions, search, quick open and `innerword list` treat it as they treat a
   page.
2. Its content is the scene as Excalidraw serialises it for a database, minified, deleted
   elements left out, every picture named by the id of its stored bytes: each entry of
   `files` has `"file":"<32 hex digits>"`, the token the store already reads for pages. The
   bytes are in `images.db`'s `files` table, so the copy among the backups, the
   damaged-file handling and the collector keep them as they keep a page's files. No new
   file, no `images.db` schema step.
3. The editor is the real Excalidraw, never a rewrite. Its JavaScript, its build and the
   built folder live in a new crate, `innerword-board-web`: sources in `web/`, the build
   `node build.mjs` (esbuild 0.28.2, exact versions, the lockfile committed), the output in
   `dist/`, **committed**, so that no cargo build and no test needs node or the network.
   The CJK hand-drawn font (Xiaolai, 12.7 MB) is left out (answer 1); Mermaid to Excalidraw
   ships whole (answer 4: 100 chunks, 2.66 MB, loaded only when it is used): the bundle is
   7.44 MB in 159 files (2.67 MB as tar.gz; the estimate was about 9 MB and 3.1 MB), and
   the arm64 disk image grew from 12.5 MB (0.10.1) to 15.6 MB.
4. The app and the tool read the bundle as files: `Contents/Resources/Whiteboard/` in the
   app bundle (one copy for both executables), the crate's `dist/` when the executable is not
   in an app bundle. Not `include_bytes!`: a universal build would carry four copies.
5. The view is the pages' view (`innerword-web`) with four things only a whiteboard's view
   has: one script message handler (`innerwordBoard`, from the main frame of its own origin
   only, under a size cap), calls into the page with `callAsyncJavaScript`, no navigation
   after the first load, and a stricter Content-Security-Policy (no script but the
   bundle's files, no eval). A view with the bridge loads no subframe at all (every subframe
   navigation is cancelled: the policy alone lets a `srcdoc` frame run a script of the
   view's own origin, 4.2). A page's view keeps no bridge and its policy.
6. **Never lose a stroke.** The page posts the scene at the end of every gesture (and 150 ms
   after a change made with the keyboard); the app always holds the newest scene it was
   given and saves it in the background as it saves a text, **compare-and-swap** against
   the stored scene the view's scene descends from, so a late post never writes over an
   agent's change or a restored version; the synchronous flush writes what the app holds;
   every deliberate end (another item opened, the item closed, `cmd-w`, the window's close
   button, Quit from the keyboard, the menu or a signal, an agent's change, an export, a
   duplicate, a restored version) first asks the page for what it has not posted yet, at
   most 1 s, keeping the view alive and out of sight meanwhile; and a scene the app cannot
   parse or store is written to a recovery file, never dropped.
7. Keys: a whiteboard has its own key context, `BoardView`. Keys without `cmd` go to
   Excalidraw (tool letters, digits, escape, arrows, typing). Of the `cmd` chords the
   window keeps its own (palette, quick open, library, settings, save, export, zoom, the
   list of shortcuts, find, close, quit); every other chord Excalidraw acts on is the
   whiteboard's on a whiteboard, held there by fixed `NoAction` bindings in `BoardView`
   and by a menu bar rebuilt without those key equivalents while a whiteboard is open; the
   list of shortcuts shows them as fixed rows "On a whiteboard". The Edit menu's Undo, Redo,
   Cut, Copy, Paste and Select All keep their keys on a whiteboard and reach Excalidraw
   (4.7). Six commands without default keys
   (Add text, rectangle, ellipse, diamond, arrow, frame) make an element without the
   pointer, so a whiteboard can be drawn from the keyboard alone (product rule 1).
8. The zoom commands zoom Excalidraw's canvas, not the web view; Fit Width and Fit Page
   show the whole drawing. Find opens Excalidraw's own search. The theme follows the app.
   **Mermaid to drawing** (Excalidraw's dialog, and Mermaid text pasted on the canvas)
   works offline under the whiteboard's strict policy, which it needs nothing more of
   (4.13). **Excalidraw's shape library** is shown and kept: one library for every
   whiteboard, stored in the library's settings (`board.shape_library`), posted by the
   glue when it changes and served to the page when a whiteboard loads; "Browse
   libraries", "Publish" and the file actions stay out of reach (4.15).
9. Pictures come in by paste (Excalidraw's own), by a drop from the Finder, by
   Insert > Image… (the app's) and from agents (a skeleton of type `image` naming a file
   of this Mac, read by the tool, section 7). Every picture but an SVG goes through
   `innerword_image::prepare` (upright, no GPS position), and its bytes are stored before
   a save names them.
10. **PDF only, one code path.** The bundle's export page, loaded offscreen in WebKit,
    restores the stored scene, runs Excalidraw's `exportToSvg` (scale 1, light colours, a
    32 px margin, its fonts inlined), shows the SVG alone, and
    `createPDFWithConfiguration:` makes one page of exactly that size, scaled down only
    when a side would pass 14,400 pt (and `innerword_web::MAX_PDF_SIDE` caps whatever the
    page answers). The app runs it in-process; the tool runs it in a
    child, `innerword __board-pdf <dir> <out.pdf>`. An empty whiteboard has no PDF.
11. Print… prints that PDF on the user's paper, scaled down to fit (`innerword-print` gains
    a scaling). The export picker of a whiteboard has one row, "PDF file". **A folder's PDF
    holds its whiteboards** (answer 2): after its documents, one page per whiteboard, the
    whiteboard's own PDF page of its own size drawn into the folder's PDF by
    `innerword-print` (`PdfWriter::add_pdf_page`, `CGContextDrawPDFPage`: vectors and text
    kept); an empty whiteboard is left out and the toast says so. The folder's Markdown,
    DOCX and plain-text exports leave whiteboards out and say so.
12. Agents: `innerword board new|add|update|remove`, `innerword boards`, and the board
    branches of `get`, `info`, `find`, `history`, `rename`, `move`, `duplicate`, `trash`,
    `restore`, `delete` and `export` (PDF only, else exit code 15). MCP: four new listed
    tools (`create_board`, `add_to_board`, `update_board`, `remove_from_board`) and the
    hidden `list_boards`: 41 tools, still 12 of them read-only, so the assistant's tools do
    not change.
13. **Skeletons become Excalidraw elements in pure Rust** (a new crate, `innerword-board`),
    not in a WebKit child: an agent's operation runs on the app's main thread and in the
    test stand-ins, and must take milliseconds, not seconds. Every value it makes (ids,
    seeds, nonces) is derived from its input and the store's clock, so a dry run says the
    ids the real run gives and the two runs of the agents' suite store the same bytes. Its
    text sizes are close, not exact; Excalidraw measures them again whenever it loads a
    scene (the view, the export page), and a case of the WebKit test program holds the Rust
    output against Excalidraw's own `convertToExcalidrawElements`.
14. The live channel goes to protocol 4 (an app of version 3 would answer a whiteboard's
    operation "the request cannot be read", exit 2). A request carries skeletons and, for
    a skeleton of type `image`, the picture the tool read from the file it names
    (`innerword_board::sniff_svg` or `innerword_image::prepare`, base64 on the channel, 16 MB
    a picture and 32 MB a request at most), which the app's listener stores in `images.db`
    `files` before it queues the request, as it stores a page's files (answer 3).
15. Every change by an agent keeps the scene as it was as a version first; `--dry-run` says
    what would be added, changed and removed; a change that sets what an element already
    has is left out, and a request that changes nothing keeps no version and says so. No
    new exit code: 15 for a text operation on
    a whiteboard and the reverse (and for an export that is not a PDF), 3 for an element id
    the whiteboard does not have, 2 for a skeleton that cannot be read, 9 for the PDF of an
    empty whiteboard.
16. An agent's change to the whiteboard that is open is merged into the live view instead
    of reloading it, **against a base**: each element the agent changed carries its version
    in the stored scene the agent worked on, and the canvas takes the agent's copy only
    where its own copy is still at that version; where the writer changed the element
    since, the writer's copy stays and the note says which of the agent's changes were not
    taken. One `updateScene` with `captureUpdate: IMMEDIATELY`, one step of Excalidraw's
    undo; the note's Undo takes it out while the writer has not drawn since, and is offered
    only when every change of the agent's run is a step of that undo. The same rule
    in Rust (`innerword_board::merge`) settles the cases the view cannot answer.
17. The side chat is sent the whiteboard's listing of elements in place of Markdown (a
    message waits up to 1 s for what the page has not posted yet); the
    quick chat does not open on one; comments, spelling, the format bar and Vim mode are not
    there.
18. Tests never make a `WKWebView` in the app's suite: the fake page engine learns
    whiteboards. Real WebKit and the real bundle are tested in `innerword-web`'s own test
    program, and in one case of `innerword-cli` that runs the built tool.
19. Whiteboards need what pages need (macOS 12.3 and WebKit's private switches,
    `innerword_web::available`). Without a page engine a whiteboard shows its listing of
    elements, and "New Whiteboard" is not there; agents still read and change whiteboards.
20. The release is 0.11.0.

---

## 1. What a whiteboard is to the user

### Its name and its signs

- The interface says **Whiteboard** ("New Whiteboard", "Rename whiteboard", "A whiteboard
  has no comments."). The code says board.
- Its row in the library has its own icon, `IconName::Whiteboard` (new, drawn like the set:
  1.5 px round strokes, no fill; a board on an easel, or a square, a circle and an arrow).
  The icon is also on its hit in the library search, in quick open (detail "Whiteboard")
  and on its row in the trash. Rows, the trash and the drag ghost carry the item's
  `DocumentKind`, not a `page: bool`.
- Agents refer to it as `board:12`. `doc:12`, `page:12` and `board:12` are read alike, as
  `doc:` and `page:` are today; an operation that does not fit the kind says so (exit 15).

### Making one

- **New Whiteboard**: the palette ("New whiteboard", keywords: drawing, canvas, diagram,
  sketch, excalidraw, board), File > New Whiteboard (after New Folder), the library's
  menu and its + menu ("New Whiteboard"), a folder row's menu ("New Whiteboard Here"), and
  a hint in the empty state of the window ("New whiteboard", with its key only when one is
  given). **No default key** (`cmd-n` is New document; `cmd-shift-n` New folder); the
  user can give it one.
- It is made at the end of the folder selected in the library (or of the folder whose row
  was used), with an empty scene, and **named in place in the sidebar** as a new document
  of the library's own `n` is (the sidebar is shown for it when hidden). `enter` takes the
  name and opens the whiteboard with the keyboard on the canvas; `escape` leaves it
  "Untitled" and opens it all the same; when the keyboard goes elsewhere while it is named
  (a click in the window, the sidebar hidden), the name typed so far is kept and it opens
  without taking the keyboard; a whiteboard trashed while it is named does not open. Its
  title is the name it was given and never follows its text. **Built:** the naming is the
  library panel's (`LibraryPanel::new_board`, available once the app gave the panel the
  scene of an empty whiteboard, `set_new_board_content`, which it does only with a page
  engine); the spec named only `enter` and `escape`, the third end keeps the panel from
  taking the keyboard back from where it went. `Workspace::new_board(title)` (no naming)
  remains for `board:new` and the tests.
- Without a page engine (macOS below 12.3, `INNER_WORD_PAGES=off`) New Whiteboard has no
  handler: greyed out in the menu bar, left out of the palette and the menus.

### What the window shows

- The canvas fills the editor area, under the title. Excalidraw's own toolbar, its style
  panel, its zoom and undo buttons and its main menu (trimmed, section 4.13) are inside it.
- **Title**: the whiteboard's title. The chevron and a right-click open its menu:
  Rename…, Add (a submenu: Text, Rectangle, Ellipse, Diamond, Arrow, Frame, From Mermaid…,
  From Shape Library…), Shape Library, Add Selection to Shape Library, Find on
  Whiteboard…, Insert Picture…, Zoom to Fit,
  Print…, Export as PDF…, Version History…, Reveal in Library, and Assistant while the
  assistant is shown.
- **Drawing without the pointer** (product rule 1): "Add text", "Add rectangle", "Add
  ellipse", "Add diamond", "Add arrow" and "Add frame" (the palette, Insert > Whiteboard in
  the menu bar, the title menu's Add, the `ctrl-enter` menu; no default keys, the user can
  give them some). Each puts a new element at the middle of the view, selected, through
  the same converter agents use (a text element opens for typing at once; an arrow with
  two shapes selected binds them, from the first selected to the second; a frame with
  elements selected holds them). From there the keyboard does the rest in Excalidraw: the
  arrows nudge (with `shift`, by the grid), `enter` edits the label, `cmd-up` and the other
  `cmd` arrows add the next node of a flowchart, and the statistics panel (`alt-/`) edits X,
  Y, W, H and the angle in fields that `tab` reaches. "Mermaid to drawing…" opens
  Excalidraw's Mermaid dialog (its text field has the keyboard; `cmd-enter` inserts,
  `escape` closes); "Show or hide the shape library", "Add selection to shape library" and
  "Insert from shape library…" (a picker of the app over the stored items, section 4.15)
  do with keys what the library's sidebar does with the pointer. Product rule 1 holds with
  no exception.
- **Status bar**, from the left: the save state ("Saved", "Saving…", "Edited", "Not saved:
  <the reason>", the item a text has, with its menu: Save Now), the elements ("14
  elements", counted as the listing counts them: a label is part of its shape or arrow;
  its menu says how many of each kind in plain plural words, "2 rectangles, 1 arrow", the
  pictures and the words), the zoom ("100 %"; its menu: Zoom In, Zoom Out, Actual Size,
  Zoom to Fit and the steps), the assistant's sparkle while the assistant is shown. No Vim
  chip, no character limit, no page indicator. A whiteboard that is not drawn (no engine,
  a page that failed to start) shows the listing's count and no zoom.
- **`ctrl-enter` and `shift-f10`** (`board::OpenContextMenu`) open the app's menu of the
  whiteboard over a picture of it: Undo, Redo, Select All, the six Add items, then the
  title menu's items. A right-click on the canvas opens Excalidraw's own menu (drawn by the
  page).
- **Where the writer looked** (scroll and zoom) is kept per whiteboard and comes back when it
  is opened again. A whiteboard that changed since it was last looked at (an agent, a
  restored version) opens zoomed to fit its drawing.

### What a whiteboard is not

No comments, no spelling, no Vim mode, no format bar, no quick chat, no document settings,
no page view, no export as Markdown, DOCX or plain text, no import or export of
`.excalidraw` or `.excalidrawlib` files in v1. Section 6.3 says what each of those commands
says on a whiteboard.

---

## 2. In the library

### The row

| Column | For a whiteboard |
|---|---|
| `kind` | `'board'` |
| `title` | The name it was given; `""` shows "Untitled". |
| `folder_id`, `position` | As for a document: whiteboards sit among the documents and pages of a folder, in one order. |
| `content` | The stored scene (below). Never parsed as an `innerword-core` document or a page manifest. |
| `plain_text` | The text of its elements (below), indexed by `documents_fts` through the existing triggers. |
| `word_count`, `char_count` | Counted from that text by the rule of a text, `innerword_core::Fragment::from_plain_text(text).to_document().stats()`, as for a page. |
| `source_url` | `NULL`. |
| `view_state` | Where the writer last looked: `{"scrollX":-120.5,"scrollY":40,"zoom":1.25,"at":"<the content stamp>"}` (`at`: the `ContentStamp` of the stored scene when it was written, below; a time stamp would not tell a save of the same millisecond apart). Written by the app only, with `Store::save_view_state`. |
| `created_at`, `updated_at`, `trashed_at`, `trashed_with` | As for a document. |

### Schema 5 of `library.db`

Appended to `MIGRATIONS` in `crates/innerword-store/src/schema.rs`:

```rust
/// Version 5: library items may be whiteboards (kind 'board'; their pictures are in
/// images.db `files`, named by "file" in the scene as a page's files are in its manifest).
///
/// The step changes nothing in this file: `kind` has no CHECK (version 4). It exists so
/// that a version of the app or of the `innerword` tool that does not know whiteboards
/// refuses the library ([`StoreError::SchemaTooNew`]) instead of failing on a row of an
/// unknown kind, or treating a whiteboard as a text and saving over it.
const V5: &str = r#"
SELECT 1;
"#;
```

- `SCHEMA_VERSION` is 5, the newest the store knows; `OPEN_SCHEMA_VERSION` is 4, the version
  an open migrates to. **The step to 5 is taken lazily**: `Store::create_board` runs V5 and
  sets `user_version` to 5 inside its own transaction when the file is at 4, before it
  inserts the row (a failure leaves both undone). So a library becomes schema 5 exactly
  when it first holds a whiteboard, made by the app or by a 0.11 tool; until then 0.10.1
  and its tool open it as before (reviewer's point: an eager step at the first open, or at
  the first read of a 0.11 tool that can take the lock, would lock 0.10 out of a library
  that never had a whiteboard). After it, 0.10.1 refuses the library with `SchemaTooNew`,
  as 0.7.0 refused schema 4. A store opens a file at 4 or 5 alike; `duplicate_document` and
  `restore` of a whiteboard only happen in a file at 5 already. A 0.11 tool never makes a
  whiteboard under a running 0.10 app: its request reaches that app on the channel, where
  the protocol refuses it (section 7).
- Tests: `a_library_stays_at_4_until_its_first_whiteboard`,
  `the_first_whiteboard_takes_the_library_to_5_and_its_documents_are_unchanged`,
  `a_failed_first_whiteboard_leaves_the_library_at_4`,
  `a_build_that_knows_schema_4_refuses_a_library_of_schema_5`; the tests that pin
  `SCHEMA_VERSION == 4` keep 4 as `OPEN_SCHEMA_VERSION` and 5 as `SCHEMA_VERSION`.
- **Built:** `open_read_only` (the tool's read of a library the app has open) compares
  against `OPEN_SCHEMA_VERSION`, not `SCHEMA_VERSION`, and `NeedsUpgrade.supported` is 4:
  otherwise a 0.11 tool would have taken every library at 4 for an outdated one.
  `take_steps_above_open(&Transaction)` is the lazy step; `migrate` goes up to 4 only.

### The stored scene

```json
{"type":"excalidraw","version":2,"source":"inner-word",
 "elements":[
  {"id":"api","type":"rectangle","x":100,"y":80,"width":200,"height":90,"angle":0,
   "strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","fillStyle":"solid","strokeWidth":2,
   "strokeStyle":"solid","roughness":1,"opacity":100,"groupIds":[],"frameId":null,
   "index":"a0","roundness":{"type":3},"seed":1640187542,"version":3,
   "versionNonce":1207911042,"isDeleted":false,"updated":1794306980000,"link":null,
   "locked":false,"boundElements":[{"type":"text","id":"api-label"}]},
  {"id":"pic","type":"image","fileId":"6c2b…(Excalidraw's id)","status":"saved","scale":[1,1],
   "crop":null, "(the fields every element has)":"…"}],
 "appState":{"viewBackgroundColor":"#ffffff","gridSize":20,"gridStep":5,"gridModeEnabled":false},
 "files":{"6c2b…(Excalidraw's id)":{"mimeType":"image/jpeg",
          "file":"3f2a9c1b5d6e7f80a1b2c3d4e5f60718","created":1794306980000}}}
```

- **The envelope** is Excalidraw's own (`type`, `version`, `source`), so the JSON that
  `innerword get board:12 --format json` prints is recognisably an Excalidraw scene.
  `source` is `"inner-word"` (Excalidraw writes the page's origin there).
- **`elements`**: every element as Excalidraw wrote it, every field kept, unknown fields
  included (a later Excalidraw adds some), in z-order, each with its fractional `index`.
  **Deleted elements (`isDeleted: true`) are left out**: they matter to Excalidraw's undo in
  the view and to nothing that is stored.
- **`appState`**: exactly `viewBackgroundColor`, `gridSize`, `gridStep`,
  `gridModeEnabled`, what `serializeAsJSON(…, "database")` keeps. Not the zoom, the scroll,
  the selection, the theme, the current tool or the current style: the zoom and the scroll
  are the row's `view_state`, the rest is Excalidraw's session.
- **`files`**: one entry per picture, under Excalidraw's own file id, with `mimeType`,
  `file` (the `FileId` of the stored bytes, 32 lowercase hex digits) and `created`. **No
  `dataURL`**: the bytes are in `images.db`. Excalidraw's file id is not the hash of what is
  stored (it hashes a pasted file before it resizes it), so the entry names both. For a
  picture the app inserts itself (Insert > Image…, a drop) the two are the same hex.
  `mimeType` is the type of the bytes as stored, read from the bytes (section "Pictures"),
  never the type the page or a pasted scene claimed.
- **Kinds of element that are never kept**: `iframe`, `embeddable` and `magicframe` (web
  embeds and AI frames; Excalidraw renders an `iframe` element's `customData` as a frame's
  `srcdoc`, section 4.13). The glue deletes them from the canvas the moment they appear;
  wherever a scene is read (`innerword_board::Scene::from_stored`, `from_view`) they are
  dropped with a warning; an agent's request that names one is refused.
- **What every read of a scene mends** (built in the final review): an element's `link`
  whose scheme is not `http`, `https`, `mailto` or the view's own origin is left out (a
  pasted scene can carry any); a value nested deeper than `serde_json` reads (a pasted
  `customData` 130 deep) is read as `null` with a warning, so it never makes a drawing
  unsaveable.
- Minified, **every object's keys sorted**, and every number the crate computes rounded to
  9 decimals. **Built:** the spec left the order to `serde_json`, but gpui turns on its
  `preserve_order` in `target/`, so the app and the tool (`target-pure`) would have written
  different bytes for one scene; and a summed width such as 116.86000000000001 changed
  bytes on a rewrite. Sorted keys and exact numbers make `to_stored` the same bytes in
  every build. The order means nothing to Excalidraw.
- **Only `innerword_board::Scene::to_stored` writes this JSON** (the app's save, the agent's
  operations); every reader parses it through `Scene::from_stored`. The collector's scan
  reads in it the one token it reads in a page's manifest (`"file"`, optional white space,
  `:`, optional white space, `"`, 32 hex digits, `"`; `images/scan.rs`), so every picture a
  whiteboard names is kept. A hit elsewhere in the scene (a `customData` of pasted
  Excalidraw JSON can hold `{"file":"<32 hex>"}`) only keeps some bytes longer, which is
  harmless. **The store's check of what a save names does not use the scan** (a false hit
  of a missing id would refuse every save of that whiteboard): the caller passes the ids,
  `Scene::stored_files()` (section "The store's half").
- **Limits** (`innerword_board::limits`) are of two kinds, because a writer's drawing must
  never become unsaveable:
  - **for agents** (a request is refused, exit 2, nothing written): 16 MB of stored scene
    after the change (`MAX_SCENE_BYTES`; about 25,000 elements at the 620 bytes Excalidraw
    spends on one), 20,000 elements (`MAX_ELEMENTS`), 20,000 characters in one text
    (`MAX_TEXT_CHARS`), 2,000 elements in one request (`MAX_REQUEST_ELEMENTS`);
  - **for the writer's own scene**: no limit on a text, on the elements or on the size
    below a hard cap of 256 MB of stored scene (`MAX_VIEW_SCENE_BYTES`, which no drawing a
    person makes reaches: ten times the largest freedraw-heavy board the scout measured).
    Over 16 MB the save state says "Large: saving this whiteboard takes longer" once; it
    saves all the same. Over the hard cap the scene goes to a recovery file (section 4.6)
    and the save state says "Not saved: this whiteboard is too large to keep. Remove
    something from it." The transport is never the limit: a scene larger than one message
    of the bridge comes in parts (section 4.4).

### The store's half

`innerword-store` gets, beside the page functions and in their shape (the two page
functions become private helpers that take a kind, since their bodies differ only in the
kind and the source address):

- `DocumentKind::Board` (`"board"`), and on `DocumentKind` `reference_prefix()` (`"doc"`,
  `"page"`, `"board"`) and `noun()` (`"document"`, `"page"`, `"whiteboard"`), so that no crate
  writes its own `if is_page { "page" } else { "doc" }` again.
- `BoardContent<'a> { scene, files, plain_text, word_count, char_count }`: `files` is the
  list of `FileId`s the scene names (`Scene::stored_files()`), which is what the store
  checks. **No title**: an autosave never writes the title, so it cannot put back a title
  that a rename (the writer's, or an agent's `rename board:12` while the whiteboard is open)
  changed while the save was in flight. Only `create_board` (its own `title` argument) and
  rename set a whiteboard's title.
- `ContentStamp`: the first 16 bytes of the SHA-256 of a row's `content`, as 32 hex digits
  (`ContentStamp::of(content)`). It is what a compare-and-swap compares: two writes of the
  same millisecond have different stamps where `updated_at` would be equal.
- `Store::create_board(folder, title, board)`, `Store::save_board(id, board, expect)`: the
  same checks as `create_page` and `save_page`, and:
  - **The store names no new file it does not have.** Of `board.files`, an id that neither
    the row's current content nor any snapshot of the row names is new; each new id is
    looked up in `images.db` (a row read, no bytes) and a missing one refuses the save with
    `StoreError::FilesMissing`, writing nothing. An id the row or a version already named is
    kept even when its bytes are missing (a `library.db` backup brought back without the
    copy of its pictures): the whiteboard shows Excalidraw's missing picture and goes on
    saving. Each id that is there has its collector's stamp cleared.
  - **Compare-and-swap.** `expect: Option<ContentStamp>`: with `Some(stamp)`, the save
    writes only when the row's content still has that stamp, read in the same transaction;
    otherwise `StoreError::Stale { id, stored: ContentStamp }`, nothing written. The app
    always passes the stamp its view's scene descends from (section 4.6); agents, which read
    and write under one lock or one turn of the main thread, pass `None`.
  - `save_board` refuses a text and a page (`WrongKind`), `save_document` refuses a page
    and a whiteboard, `save_page` refuses a text and a whiteboard. Today `save_document`
    refuses only `Page` and `save_page` only `Text`: both become "anything but my kind".
  - **The order of refusals** (built; the spec fixed none): `WrongKind`, then `Stale`, then
    `FilesMissing`. The kind is checked first, so a save of another kind touches nothing;
    then the files are looked up (`board_files_missing`, which clears the collector's stamp
    of each present id and writes back one only the backups' copy has); then the IMMEDIATE
    transaction checks the kind again and compares the stamp, the one place a save that
    goes through reads and hashes the stored scene. When files are missing the stored
    stamp is looked at first, so a late post is told `Stale` (which the app merges on)
    before `FilesMissing`. So a save refused as `Stale` or `FilesMissing` writes nothing to
    `library.db` but may have touched `images.db` as said (harmless: the caller's scene
    names those files). The ids of `FilesMissing` come in the order of `board.files`.
  - The answer carries the new stamp (`BoardSaved { meta, stamp }`); the text is not
    indexed again when it did not change.
- `Store::boards()`: every whiteboard, live and trashed, most recently updated first, from
  the covering index (`boards_query`, held by the plan test
  `the_list_of_whiteboards_does_not_read_the_scenes` at schema 4 and 5).
- `StoreError::WrongKind { id, kind, saving }` (`saving`: the kind the caller tried to
  write), with a sentence for each pair that names the item by its id, as today's does (the
  store's error carries no title: "item 12 of the library is a whiteboard: the text of a
  document cannot be saved over it", in the crate's lowercase style and its existing words
  for a text); the agents' and the app's sentences, which have the title, are their own
  (section 7.7). `FilesMissing`'s sentence says "the item" instead of "the page".
- `images/collect.rs`, the table of who can hold the id of stored bytes, gets a row: "a
  whiteboard's scene, live, trashed or in a version: the scan finds it (`"file"`). The open
  whiteboard in the app's view holds file ids in Excalidraw's undo history and its own map
  of pictures, as an editor's undo history holds image ids: only the run at launch deletes,
  and at launch no view exists."

### Pictures

- Stored in `images.db`'s **`files`** table (`Store::put_file`, `ImageFile::put_file`), under
  the `FileId` of the bytes as stored. Not in `images`: a whiteboard's picture is not an
  `innerword-core` image and has no `ImageId`; `files` takes any media type (an SVG) and
  needs no new token. What `images.db` and the store already do for a page's files holds:
  stored once by hash, kept by the copy among the backups, set aside when damaged and taken
  from the copy, deleted only by the collector 30 days after nothing names it.
- **What is stored is decided by the bytes, never by the type the page names** (the `file`
  message's `mimeType` comes from the page, or from pasted JSON). `innerword_board::sniff_svg`
  reads the bytes: an optional byte-order mark, an XML declaration, comments and a doctype,
  then a root element `svg` in the SVG namespace, all within 4 MB and valid UTF-8. Bytes that
  are such an SVG are stored as they came (Excalidraw allows 4 MB; an SVG is only ever drawn
  as a picture through an `<img>`, never as a document, so its scripts never run). Every
  other byte goes through `innerword_image::prepare`, as every picture of the library does:
  upright, at most 4096 pixels and 16 MB, no GPS position, HEIC, TIFF, BMP, WebP and AVIF
  converted; whatever `prepare` refuses (an ICO, a broken file, an HTML file claiming to be
  `image/svg+xml`) is not stored: a toast names it with `PrepareError::sentence`, and its
  element stays on the canvas and shows Excalidraw's placeholder after the next load.
- **The type a picture is served and recorded with** is the type of the stored bytes:
  `image/svg+xml` for what `sniff_svg` took, otherwise what `innerword_page::sniffed_type`
  reads (PNG, JPEG, GIF), and never `text/html`; the scene's `files` entry gets that type at
  the save (`Scene::from_view` writes the map's type, not the page's).
- **Bytes first** (product rule 4): no save names a picture whose bytes are not stored and
  committed (section 4.6 says how the app orders the two; the store refuses otherwise).
- **An agent's picture** (a skeleton of type `image`, section 7.2) follows the same rule
  from the other end: the file is read in the process that was asked and made by the same
  two functions (`sniff_svg`, else `prepare`; one routine, `innerword_agent`'s
  `board_picture`, beside the app's own in `boards.rs`), its bytes travel with the request
  and are stored in `files` before the operation's scene names them (section 7.6). Its
  element names it as a picture the app inserted does: Excalidraw's file id is the
  `FileId` hex.
- **A picture the library does not have** (in neither `images.db` nor the copy): the view
  is answered 404 for it, Excalidraw draws the element as a missing picture, the PDF leaves
  it out as Excalidraw's export does, and agents read the element with `"picture":"missing"`.
  The scene keeps it.

### Its text and counts

`innerword_board::Scene::text()`: the text of every text element (free text, the labels of
shapes and arrows) and the name of every frame, in reading order (by the top edge of the
element, a label by its container's, then by the left edge), one per line, empty texts
left out (a text as it was typed, its `originalText`, not as it wraps). It is
`plain_text`; the counts are of it. So the library search finds a
whiteboard by the words on it, `innerword list` shows its words, and quick open lists it by
title. A whiteboard without text has none.

### Versions, trash, duplicate

- **The app keeps versions of a whiteboard as of a text**: after an autosave,
  `Store::snapshot_if_stale` with the interval a text uses (`SaveJob::snapshot_interval`).
  A snapshot's `content` is the stored scene; its kind is its row's.
- **The shape library is not part of a whiteboard**: it is a setting of the library
  (section 4.15), so a version, a duplicate, the trash and an export of a whiteboard neither
  hold nor change it.
- **Agents keep a version before every change** (`content::keep_version`), labelled:
  "Before a drawing by Claude" (`add_to_board`, new `Reason::Drawing`), "Before an edit by
  Claude" (`update_board`, `Reason::Edit`), "Before a removal by Claude"
  (`remove_from_board`, new `Reason::Removal`), "Before a version was restored by Claude"
  (`restore_version`, `Reason::Restore`). `create_board` keeps none. A change that sets
  what an element already has is left out of the request (no version raised); a request
  left with nothing to do keeps no version and says "Nothing changed" ("would change
  nothing" in a dry run, `Written::would_change`). **Built:** at first such a request still
  raised versions and kept one (the final review).
- **The version history** of a whiteboard shows each version with its date and label, and
  as its preview the listing of its elements (section 7), in the muted colour. Restore
  keeps the current scene as a version first and brings the old one into the open view as
  one step of its undo (section 6.7).
- Trash, restore, move, delete for good: as for any item. Deleting frees the pictures'
  space 30 days later, at a launch, through the collector.
- **Duplicate** copies the kind and the scene (the store copies `kind`); the copy names the
  same pictures, which are stored once. Duplicating the open whiteboard in the app first
  collects and saves (section 4.6), so the copy has the last gesture.
- **Rename** sets the title. `innerword rename board:12 --follow-text` is refused (exit 2,
  "a whiteboard has no text to take its title from").

---

## 3. The bundle: `innerword-board-web`

### What the crate is

A new crate that holds everything of Excalidraw: its JavaScript sources and build, the
built folder, the licenses of what is in it, and a small Rust library that finds the
built folder and knows its contract (the files, their media types, the bridge's messages,
the calls, the keys Excalidraw acts on). It depends on nothing of ours (like `innerword-page`
and `innerword-web`), builds and tests in `target-pure`, and is linked by `innerword-app` and
`innerword-cli`.

```
crates/innerword-board-web/
  Cargo.toml            serde, serde_json, sha2 (all in Cargo.lock already); dev: innerword-board,
                        innerword-keymap, sha2, tempfile
  build.rs              INNER_WORD_BUNDLED: compile no dist/ path in
  src/lib.rs            Assets, bridge, calls, keys (section 9, B3)
  tests/main.rs         one integration binary (suite/: assets, bridge, bundle, calls, keys):
                        dist against MANIFEST and BUILD-INFO, notices, no eval, ...
  web/
    package.json        "private": true, exact versions, no ranges
    package-lock.json   lockfile version 3, with integrity hashes; committed
    .npmrc              ignore-scripts=true, audit=false, fund=false
    build.mjs           the build
    bundled-inside.json packages carried inside others, and the license texts that are ours
    src/index.html      the editor's page
    src/export.html     the export page
    src/main.jsx        mounts Excalidraw (index.html) or runs the export (export.html)
    src/glue/           asset-path.js, bridge.js, scene.js, fonts.js, pictures.js, autosave.js,
                        edit.js, calls.js, keys.js, links.js, library.js, mermaid.js,
                        board.jsx, export.js
    src/board.css       what is hidden of Excalidraw's interface
    fonts-licenses/     the OFL 1.1 text once, Comic Shanns' MIT text, fonts.json (each font's
                        copyright lines, which the build checks against every file)
    licenses/           license texts of packages that ship none (Excalidraw itself, 35 Radix
                        packages, fastdom, ...), named under textsOfOurs in bundled-inside.json
    UPGRADING.md        what to do for a new Excalidraw (below)
  dist/                 the built folder, committed (git: -diff -text linguist-generated)
    index.html  export.html  assets/  fonts/  font-metrics.json
    MANIFEST.json       every file of dist/: path, bytes, sha256, media type ("type")
    BUILD-INFO.json     the versions, the sha256 of every input, the patches applied
    NOTICES.txt         the licenses of everything in the bundle
```

Root files, in B3's first commit: `.gitignore` gets `crates/innerword-board-web/web/node_modules/`;
`.gitattributes` gets `crates/innerword-board-web/dist/** -diff -text linguist-generated` and
`*.woff2 binary`; the root `Cargo.toml` gets `innerword-board-web` in
`[workspace.dependencies]`. **Built:** `-text` was added after review: with only `-diff`, the
repository's `* text=auto eol=lf` would let git convert a CR a later build writes, and a fresh
clone would hold files whose size and SHA-256 differ from `MANIFEST.json`, which
`Assets::read` refuses.

### Pinned versions

`package.json`, exactly (`npm ci` refuses a lockfile that does not match it):

```json
{
  "name": "innerword-board-web",
  "private": true,
  "type": "module",
  "engines": { "node": ">=22" },
  "dependencies": {
    "@excalidraw/excalidraw": "0.18.1",
    "react": "19.2.9",
    "react-dom": "19.2.9"
  },
  "devDependencies": { "esbuild": "0.28.2", "fontkit": "2.0.4" }
}
```

0.18.1 is npm's `latest` (2026-04-20); the `next` tag's daily snapshots are not used. React
19.2 is the line it was released against (peer range `^17 || ^18 || ^19`).

### The build

```bash
cd crates/innerword-board-web/web
npm ci --ignore-scripts --no-audit --no-fund   # node 22 or later; the only step that reaches the network
node build.mjs                                 # writes ../dist from nothing, in about half a second
node build.mjs --check                         # builds into a temporary folder and compares it with ../dist, byte for byte
```

`--ignore-scripts`: no package's install script runs (esbuild needs none; its binary comes
in its platform package). `build.mjs` is the prototype's (`board/proto/build.mjs`) grown
up:

- esbuild: `format: "esm"`, `splitting: true`, `minify`, `target: ["safari15"]`,
  `conditions: ["production"]`, `jsx: "automatic"`, `define NODE_ENV "production"`,
  `legalComments: "eof"`, two entry pages that load the same `assets/main.js`.
- **Patches, each applied exactly once or the build fails** (so that a new Excalidraw cannot
  drop one quietly):
  1. *No font CDN.* Excalidraw appends `https://esm.sh/@excalidraw/excalidraw@0.18.1/dist/prod/`
     to the `src` of every `FontFace` whatever `EXCALIDRAW_ASSET_PATH` says; WebKit checks
     each against the CSP at once (234 violation reports at a load in the scout's run, no
     request left). The prototype's patch (`r.length||r.push(…ASSETS_FALLBACK_URL…)`) makes
     it a fallback only when no local path was given.
  2. *No Xiaolai* (answer 1): the list of Xiaolai's faces in the package is
     emptied, so no `FontFace` names a file the bundle does not have. When 0.18.1's code
     does not allow that patch cleanly, the builder leaves the faces registered instead, and
     their requests are answered 404 by the board's source (CJK text then falls back to the
     system's font on screen and in the PDF); the WebKit test then allows 404s under
     `fonts/Xiaolai/` and nowhere else.
  3. *The locales* (55 lazy chunks, 1.65 MB, loaded only for a language other than `en`):
     left out by a resolve rule that fails the build when it stops applying, if 0.18.1's
     dynamic import allows one; otherwise kept (never loaded).
  4. *No eval in the font subsetter* (built, not in the spec): its WebAssembly glue
     (emscripten's embind) makes three kinds of function with `Function`, which the export
     page's policy refuses; subsetting then failed and every font went into the PDF whole
     (about 1 s an export). The build turns the three into closures, each applied exactly
     once; the export page keeps its strict policy and an export takes about 90 ms.

  **Built:** patches 2 and 3 applied cleanly: Xiaolai's face list is emptied and its paths
  blanked, so no 404 is allowed anywhere (the WebKit test fails on a request under
  `fonts/Xiaolai/` unless `BUILD-INFO.json` records the patch as not applied); the 53
  locales other than `en` and `percentages` resolve to one empty module. `BUILD-INFO.json`
  records the count of each patch (font CDN 1, Xiaolai 1, locales 53, no eval 3).

  **Mermaid is not patched** (answer 4): `@excalidraw/mermaid-to-excalidraw` 2.2.2 and what
  it brings (`mermaid` 11.17.2 with its `@mermaid-js/parser`, d3, dagre-d3-es, cytoscape,
  KaTeX's JavaScript, DOMPurify and the rest; built: 100 lazy chunks, 2.66 MB) are bundled as
  Excalidraw imports them, behind its two dynamic `import()`s (the dialog and a paste of
  Mermaid text), so none of it loads with the page. The build's `--no-mermaid` switch of the
  prototype is not carried over. What Mermaid needs at run time, read in the prototype's
  `node_modules` and its built chunks, is in section 4.13.
- Copies `node_modules/@excalidraw/excalidraw/dist/prod/fonts` to `dist/fonts/` without
  `Xiaolai/`, and rewrites `url(./fonts/` in `main.css` to `url(../fonts/` (the UI font,
  Assistant).
- **Writes the font metrics** `dist/font-metrics.json`: for every font file of `dist/fonts/`
  that a scene can name, its family number, units per em, and the advance width of every
  character from U+0020 to U+024F, read from the font files by a font-parsing
  devDependency with WOFF2 support (`fontkit` 2.0.x, MIT, pinned exactly in the lockfile
  like the rest; B3 confirms the version and its license before it pins it). So no step of
  the build needs Python or a package outside the lockfile (reviewer's point: fontTools is
  not on this Mac and installing it reaches the network). `innerword-board` keeps a copy of
  that file and a test holds the two equal (section 7.1).
- Writes `MANIFEST.json` (every file: path, bytes, sha256, media type by the table below,
  under the key `"type"`), `BUILD-INFO.json` (`excalidraw`, `react`, `esbuild`, `fontkit`,
  `mermaid` versions; the sha256 of `package-lock.json`, `build.mjs` and every file under
  `src/` and `fonts-licenses/`; the patches applied; `packages`, `mermaidChunks`, and the
  two lists `metafilePackages` (`name@version` of every package in esbuild's metafile,
  before any license handling: 127) and `carriedInside` (`{carrier: [names]}`), against
  which a test holds `NOTICES.txt`), and `NOTICES.txt` (below). No time stamps: the same inputs give the same
  bytes, which `--check` holds.
- Fails on a license outside the accepted list.

What is in it, without Xiaolai and with Mermaid, **as built**: 159 files and the manifest,
7.44 MB on disk, 2.67 MB as tar.gz; JavaScript 6.66 MB in 128 files (about 1.4 MB of it
loaded at start, the font subsetter at the first export, Mermaid's 100 chunks, 2.66 MB, at
the first Mermaid dialog or paste; the locales are gone), fonts about 440 KB (Excalifont,
Virgil, Nunito, Lilita One, Comic Shanns, Liberation Sans, Cascadia Code and the interface
font Assistant). The arm64 disk image grew by 3.1 MB (12.5 MB for 0.10.1, 15.6 MB), the
editor is 7.5 MB in the app. (The design's estimate from the prototype's build was 8.4 MB of
JavaScript in 180 files, about 9.0 MB on disk and 3.1 MB as tar.gz.)

### Licenses

- `NOTICES.txt` is made by `build.mjs` from esbuild's metafile: every package that has a
  file in the bundle, with its version, its license (from its `package.json`) and its
  license text (its `LICENSE` file), each text once with the packages under it; then the
  fonts, from `web/fonts-licenses/` (OFL 1.1 for Excalifont, Virgil, Nunito, Lilita One,
  Liberation Sans, Cascadia Code, Assistant and, if it ever ships, Xiaolai; MIT for Comic
  Shanns; each font's copyright line as its `name` table says, read once by the builder).
  Accepted: MIT, ISC, Apache-2.0, BSD-2-Clause, BSD-3-Clause, 0BSD, CC0-1.0, Zlib,
  Unlicense, OFL-1.1, the same list as `about.toml` plus OFL. An SPDX choice is accepted
  when one of its alternatives is (DOMPurify 3.5.0 is `(MPL-2.0 OR Apache-2.0)`: taken
  under Apache-2.0, and `NOTICES.txt` says so); a package whose `package.json` names no
  license (`khroma` 2.1.0) is read from its `license` file, and the build fails when that
  says nothing it knows. **Built:** many bundled packages ship no license file at all
  (Excalidraw itself, 35 Radix packages, fastdom, react-remove-scroll-bar); their texts were
  taken from their repositories into `web/licenses/`, named under `textsOfOurs` in
  `bundled-inside.json`, and a new package without a text fails the build. The subsetter
  carries fonteditor-core and harfbuzzjs as WebAssembly: their texts come from the chunk's
  license comments.
- **Packages inside another package's files.** A package's own `dist/` can carry others
  that the metafile never shows as packages (`@mermaid-js/parser` carries langium and
  chevrotain, Apache-2.0 and MIT). The metafile's path is read from its innermost
  `node_modules/` (`mermaid/node_modules/@mermaid-js/parser/…` is the parser, not
  `mermaid`), and `web/bundled-inside.json` names, for each package that carries others,
  those it carries; their texts come from their own folders in `node_modules`. The build
  fails when a package it lists is not in the lockfile, and `UPGRADING.md` says to read the
  carriers' `dist/` again. **Built:** the carriers are found from their source maps and
  keyed by carrier name and version: `@mermaid-js/parser@1.2.1` carries langium, chevrotain
  and ten others, `mermaid@11.17.2` carries js-yaml 4.3.0 (path-browserify and js-yaml are
  not in the lockfile, so their texts are in `licenses/`); 13 carried in all.
- `dist/NOTICES.txt` ships inside the bundle (`Contents/Resources/Whiteboard/NOTICES.txt`).
  `about.hbs` gets a paragraph so that `THIRD-PARTY-LICENSES.txt` says: "The whiteboard's
  editor, Excalidraw, and its fonts are JavaScript and font files, not crates: their
  licenses are in Contents/Resources/Whiteboard/NOTICES.txt." `scripts/bundle.sh` copies the
  file onto the disk image as `THIRD-PARTY-LICENSES-WHITEBOARD.txt` and checks both places.

### How the app and the tool find it

```rust
let assets = innerword_board_web::Assets::find()?;   // Err: "The whiteboard's files are missing from Inner Word (…)."
```

1. When the running executable is in `<something>.app/Contents/MacOS/`: only
   `Contents/Resources/Whiteboard/`. The bundled app and the bundled tool find the same
   folder.
2. Otherwise (a `cargo build`, debug or release, scripted runs and measurements, the tests of
   `innerword-cli` that run the built tool): the crate's own `dist/`, by the path cargo gave at
   compile time (`env!("CARGO_MANIFEST_DIR")`). `scripts/bundle.sh` builds with
   `INNER_WORD_BUNDLED=1` in the environment, and a build with that variable compiles no
   such path in (`option_env!`), so the released binaries hold no path of the build machine
   and look nowhere but in their bundle.
3. There is **no variable that points the app at another folder**: what runs in a
   whiteboard's view is what was signed into the app.

`Assets::find` reads `MANIFEST.json` once and serves only the files it lists, with its media
types. Reading a file is `Assets::read(path)`, on whatever thread asks (the scheme handler
asks on a background queue). The view's source and the tool's child wrap it as an
`innerword_web::PageSource` (section 9). Media types: `text/html; charset=utf-8`,
`text/javascript; charset=utf-8`, `text/css; charset=utf-8`, `font/woff2`, `application/wasm`
(WebAssembly's streaming compile refuses another), `application/json`, `image/svg+xml`,
`image/png`, and `text/plain; charset=utf-8` (built: so that `NOTICES.txt` is listed); a file
of another ending fails the build. `Assets::read` refuses a file whose size is not the
manifest's. The Rust half has `Assets::{find, find_for(executable), at, root, get, read,
paths}` and `AssetInfo { media_type, bytes, sha256 }`.

**Why files and not `include_bytes!`**: the app and the tool are two executables; a
universal build holds each architecture's copy of embedded data; four copies of 9 MB is
36 MB in the bundle where one copy is 9 MB (and with Xiaolai, 87 MB against 22 MB). Files
in `Resources` are sealed by the bundle's signature as the executables are.

### `scripts/bundle.sh`

- Copies `crates/innerword-board-web/dist/` to `Contents/Resources/Whiteboard/` before
  signing (`ditto`, then `diff -rq`); checks `index.html`, `export.html`, `MANIFEST.json` and
  `NOTICES.txt` are there and that the folder has as many files as its `MANIFEST.json` lists,
  in `dist/`, in the app and in the app on the disk image; builds both executables with
  `INNER_WORD_BUNDLED=1`.
- When `node` is on the `PATH` **and the packages are installed** (`web/node_modules`), runs
  `node build.mjs --check` and refuses to bundle a `dist/` that is not what its sources
  build; otherwise it says which of the two was missing and that the check was skipped.
- The smoke test, while the bundled app is open on its temporary library: the bundled tool
  makes a whiteboard and draws on it (the app takes both changes), reads it back, and
  exports it as a PDF (the real child in the bundle, which must find
  `Resources/Whiteboard`), read back with PDFKit: one page, its text has the label.

### Upgrading Excalidraw (`web/UPGRADING.md`)

A new version is a change of its own, never part of another: change the three versions,
`npm install`, `node build.mjs` (each patch must still apply once), then read again in the new
package what this design took from 0.18.1 and change what moved: the keys of section 4.7
(`actions/*` key tests), the switches of 4.13, the element fields and defaults
`innerword-board` writes (section 7), the font metrics (copy `dist/font-metrics.json` into
`innerword-board` again), `restoreElements` (it raises the version of an element whose
local copy is newer: the glue's merge does not pass it local elements, section 4.5),
`exportToSvg`'s options, the kinds of element the glue deletes (4.13), the tooltips whose
keys the glue removes (4.7); run
`innerword-web`'s WebKit program (its cross-check of the converter fails when Excalidraw's
own output moved) and the person checks of section 10. `innerword_board::EXCALIDRAW_VERSION`
and `innerword_board_web::EXCALIDRAW_VERSION` must equal the package's (a test of B3).
**Built:** the test that nothing in the bundle evaluates a string counts what is allowed
exactly (six of lodash's `Function("return this")`, one dead `(0,eval)(t)` of chevrotain,
and no `new Function`, no string timer, no `.constructor("…")`); `UPGRADING.md` says what to
do when a new version changes a count.

---

## 4. The view

### 4.1 What a whiteboard's view shares with a page's, and what it does not

| Part | A page | A whiteboard |
|---|---|---|
| Engine seam (`pages::PageEngine::{WebKit, Fake}`), `use_webkit`, `use_fake`, `not_shown_sentence` | yes | the same engine |
| The native view on the bounds of a `canvas` element, made at the first prepaint, `try_borrow_mut` while lent | yes | the same |
| One focus, first responder kept in step every frame (`sync_page_view`) | yes | the same |
| Every AppKit callback only posts to the inbox; a task acts (`pages::post`, `next_events`, `answer()`) | yes | the same, with the bridge's messages among the events |
| Holes for toasts; a modal or a context menu covers the live view (built: a snapshot under the hidden view at first, replaced after 0.11.0, 4.14) | yes | the same |
| Theme through `NSAppearance` | yes | the same; the glue turns Excalidraw's own theme with it |
| Crash: reload once, then "stopped working" | yes | the same, after saving what the app holds |
| Origin | `page-<id>-<created_at>` | `board-<id>-<created_at>` (`pages::board_host`): never a page's origin |
| Rule list, private switches, one non-persistent data store, the reader-only link gate | yes | unchanged |
| Content-Security-Policy on every answer | `CONTENT_SECURITY_POLICY` (inline scripts and eval allowed: a saved page has them) | `BOARD_CONTENT_SECURITY_POLICY`, stricter (4.2): the bundle is our own code and needs neither |
| `alert` | a toast "The page says: …" | nothing (a scene cannot show a message dressed as the app's) |
| Its files | the manifest's, from `images.db` | the bundle's, `board.json`, `library.json` and `_files/<FileId>` (4.3) |
| A bridge to the app | none | one script message handler, `innerwordBoard` (4.4) |
| Calls into the page | `evaluate` (scripted runs, a link's address) | `call_async` (4.5) |
| Navigation | to the page's own files | **none after the first load** (`ViewConfig::one_document`) |
| Key context | `PageView` | `BoardView` (4.7) |
| Zoom | the web view's `pageZoom` | Excalidraw's canvas zoom through the glue |
| Find | the find strip, WebKit's find | Excalidraw's own search |
| Edit menu | `copy:`, `cut:`, `paste:`, `selectAll:`, the view's `NSUndoManager` | copy, cut and paste the same; Undo, Redo, Select All through the glue |
| Drops | the view takes none; GPUI's view adds pages | the same; picture files go onto the canvas (4.12) |
| WebKit's own right-click menu | cut down, with the app's items | Excalidraw draws its own and stops WebKit's; the app's items are in the title menu and `ctrl-enter` |
| The open panel of `<input type=file>` | answered with nothing | the same (Excalidraw's image tool is hidden, 4.13) |

The app does not get a third `Option` beside `session` and `page`: the page session becomes
the session of an open **web item** with a kind (section 6.1).

### 4.2 What `innerword-web` gains

All of it only for a view whose configuration asks; a page's view is made exactly as today.
**`ViewConfig` does not change**: what only a whiteboard's view has is a second struct,
`ViewExtras { bridge, one_document, csp }` with a `Default` that is a page's view, passed to
a new constructor `WebPage::new_with(gpui_view, config, extras, events)`; `WebPage::new`
calls it with `ViewExtras::default()`. So the app's literal of `ViewConfig`
(`workspace/page.rs`) is not touched in wave 1 (reviewer's point: two wave-1 packages would
otherwise edit `innerword-app`).

- `ViewExtras::csp: Option<&'static str>`: the policy of every answer of the view's scheme
  handler, in place of `CONTENT_SECURITY_POLICY`. A whiteboard's view and its export page
  use `BOARD_CONTENT_SECURITY_POLICY`:
  `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'
  data: blob:; font-src 'self' data:; connect-src 'self' data: blob:; worker-src 'self'
  blob:; frame-src 'none'; child-src 'none'; object-src 'none'; form-action 'none';
  base-uri 'self'`. The policy runs no inline script, no eval, no frame of a file and no
  `data:` frame, but it does not stop a `srcdoc` frame: a `srcdoc` document inherits the
  policy, and `script-src 'self'` lets it run any script of the view's own origin, while
  `frame-src 'none'` does not apply to `about:srcdoc`. What keeps every frame away from the
  bridge is that **a view with the bridge loads no subframe at all**: every subframe
  navigation is cancelled, whether to a file, a `srcdoc`, a `data:` URL or `about:blank`
  with a source (`ViewHost::subframe_may_load`, `ViewShared::bridged`). The main-frame check
  alone cannot do it, because WebKit reports a message that a same-origin frame posts
  through `top.webkit.messageHandlers` as the main frame's. **Built:** the spec said the
  policy alone kept frames off; B11 found in WebKit (`board::frames_never_reach_the_bridge`)
  that a `srcdoc` frame naming one of the view's own scripts runs under it (a static one,
  one a script makes after the load, one sandboxed `allow-scripts allow-same-origin`, a file
  frame turned to `srcdoc` later), and added the cancellation. The export page needs
  WebAssembly for the font subsetter: its policy adds `'wasm-unsafe-eval'` to `script-src`
  (`BOARD_EXPORT_CONTENT_SECURITY_POLICY`), and it only ever loads a stored scene that
  `innerword_board` read and filtered. **Built:** the export policy is chosen at run time
  (`innerword_web::board_export_policy()`): `'wasm-unsafe-eval'` exists only from Safari 16's
  WebKit (614, `WASM_KEYWORD_SINCE_WEBKIT`), and macOS 12.3's WebKit (613) ignores it and
  refuses WebAssembly, so below 614 the export page takes `'unsafe-eval'` in its place
  (`BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD`), and only the export page (the
  review proposed `'wasm-unsafe-eval'` for the live view too; it does no export and loads no
  WebAssembly, so it gets none). Read from WebKit's release notes, not run on 12.3; every
  caller (the app, the tool's child, the tests) passes `board_export_policy()`, never the
  constant. The bundle's HTML
  has no inline script and no inline
  event handler (a test of B3). This also guards the calls: `collect` and `adopt` live in
  the page world, where a foreign script could replace them and hand the app a forged
  scene; no foreign script can run there. **Mermaid to drawing needs nothing more of the
  live view's policy** (read in its chunks, section 4.13: no eval, no `new Function` that
  runs, no worker, no WebAssembly, no request; its inline `<style>` elements are allowed by
  `style-src 'unsafe-inline'` already, and its preview is drawn on a canvas, not through
  the font subsetter), so the live view's policy stays as above.
- `ViewExtras::bridge: Option<Bridge>` (`Bridge { name: &'static str, max_bytes: usize }`):
  before the web view is made, its configuration's user content controller gets one
  `WKScriptMessageHandler` (a new class, `InnerWordWebBoardMessages`, sibling of the check's
  `InnerWordWebCheckMessages`) under that name, **in the page world**
  (`addScriptMessageHandler:contentWorld:name:` with `[WKContentWorld pageWorld]`: Excalidraw
  and the glue run there, and a named world would add nothing, since a page-world script
  can reach a named world's listener anyway, as the scout showed). The handler holds a
  `Weak` of the view's shared state (the controller retains its handlers: a strong one would
  be a cycle). For each message it checks, in this order, and drops and counts
  (`ViewStats::messages_dropped`) whatever fails: the name; `frameInfo.isMainFrame`;
  `frameInfo.securityOrigin` is `innerword-page` with the view's own host; the body is a
  string of at most `max_bytes` UTF-8 bytes. Then `PageEvents::message(body)`, which only
  posts. **Built:** the body is converted **lossily** (`bridge::utf8_of`: strict UTF-8 first,
  else the UTF-16 units through `getCharacters:range:`, a lone surrogate becoming U+FFFD),
  and the byte cap applies after the conversion; a strict conversion would have dropped
  every post of a rev whose text the glue failed to mend, against 4.4's promise that the
  transport never refuses a drawing. `call_async`'s answers go through the same function.
  The handler's ivar is `innerwordBridge`. `WebPage`'s `Drop` sends `removeAllScriptMessageHandlers` (and the check's
  `tear_down` keeps doing it for the check).
- `PageEvents::message(&self, body: String)`, a default method that drops the body.
- `ViewExtras::one_document: bool`: after the entry's first load finished, every
  main-frame navigation is cancelled, also to the view's own files (a whiteboard never
  navigates; a navigation would throw away what is not saved). `WebPage::reload` still
  works (it is the app's), and the view does not call `alert`'s toast for such a view
  (`PageEvents::alert` is not called when `one_document` is set). **Built:** a little
  stricter: in the main frame only the view's own loads of its entry go through (the first
  load and `WebPage::reload`), also before the first load finished; a link the reader opens
  still goes to the browser through the existing gate.
- **A view out of sight, still drawn**: `WebPage::park(&self)` moves the view's frame out
  of the window's visible content (its origin at x = -100,000, its size kept) and turns its
  window occlusion detection off; it is not hidden (a hidden `WKWebView` stops animation
  frames and throttles timers, and fires `visibilitychange`). The closing whiteboard's view
  is parked while its last scene is collected (4.6). **Built:** `park` hides and shows the
  view again in the same turn of the run loop after turning occlusion detection off
  (otherwise WebKit never checks visibility again in a window already covered, and
  animation frames stay stopped), hands the keyboard back to GPUI's view, and records the
  hidden state and the occlusion detection it found (`_windowOcclusionDetectionEnabled`,
  read only where the view answers it, an optional selector); `set_frame` brings a parked
  view back and restores both, and `set_hidden` on a parked view becomes the state it comes
  back with (`WebPage::is_parked`). The app never calls `set_frame` on a closing view.
- `WebPage::call_async(&self, body: &str, argument_json: &str, done)`:
  `callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:` in the page world,
  with one argument `arg`, the JSON text as an `NSString` (WebKit passes values, so nothing
  is pasted into the script's source), and WebKit awaiting the function's promise itself. The
  body returns a string (the glue's calls `return JSON.stringify(…)`); `done` gets
  `Ok(String)`, or `Err` with the exception's message (`WKJavaScriptExceptionMessage`). It
  is called on the main thread inside WebKit's block, guarded, as `evaluate`'s is.
- **One script, offscreen, and its PDF**: `run_script(input, done)` (main thread, the
  caller's run loop) and `run_script_blocking(input)` (a program: the tool's child). They
  generalise what `check.rs` and `print_offscreen` already do: an offscreen window at
  (-20000, -20000), a web view with occlusion detection off (Excalidraw draws in animation
  frames), a fresh data store, the rule list, the CSP, a host of its own (`board-export`),
  the source's entry loaded, `didFinish` **and the page's `load` event** waited for (built:
  WebKit reported `didFinish` while `document.readyState` was still `interactive`, and in
  about half the runs `window.innerwordBoard` did not exist yet), then **one** `callAsyncJavaScript` in the
  page world with `input.script` and `input.argument_json`; every answer of the source
  carries `input.csp`. With `input.pdf`, the script's
  answer must be JSON `{"width": w, "height": h}` (CSS pixels, which become points 1:1), and
  after `document.fonts.ready` and two animation frames (a second call) the helper calls
  `createPDFWithConfiguration:` with a `WKPDFConfiguration` whose `rect` is
  `(0, 0, ceil(w), ceil(h))` (WebKit drops a fractional size: 413.5 became 413 in the
  scout's run), checks the result is one page (`pixels::pdf_pages`) and tears everything
  down. Fails on a crash, a navigation error, a script error, an answer that is no size,
  the time limit (`input.limit`), or a missing entry. Each side of the answer is capped at
  `innerword_web::MAX_PDF_SIDE` (14,400 pt) whatever the page says (built, after review: a
  forged answer of 1e7 would have made WebKit render a huge rect). `ScriptInput { source,
  entry, script, argument_json, pdf, csp, limit, rules_dir }`, `ScriptOutput { value, pdf:
  Option<PrintedPdf> }`. The check of a page under another policy is
  `check_with_policy` / `check_blocking_with_policy` (built: `CheckInput` stays as it was,
  since other crates build it with literals).
- `ViewStats` gains `messages_received` and `messages_dropped`.
- The guard (`selectors.rs`, `USED`) gains what the new code sends: `+[WKContentWorld
  pageWorld]`, `-[WKWebView callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:]`,
  `-[WKWebView createPDFWithConfiguration:completionHandler:]`, `-[WKPDFConfiguration setRect:]`,
  `-[WKScriptMessage name]`, `-[WKFrameInfo securityOrigin]`, `-[WKSecurityOrigin protocol]`,
  `-[WKSecurityOrigin host]`, `-[NSData length]`, `-[NSData bytes]`, and (built)
  `-[NSString length]`, `-[NSString dataUsingEncoding:allowLossyConversion:]`,
  `-[NSString getCharacters:range:]`. All are macOS 11 or older,
  under the 12.3 floor of pages, and required: a macOS without one has no whiteboards and
  no pages, rather than a whiteboard that cannot save.
- The module's notes (`lib.rs`) say "no bridge to the app in a page's view; a whiteboard's
  view has one handler, checked as above". (`docs/pages-design.md`, section 5, "A bridge to
  the app: none", gets the same sentence; B10.)

### 4.3 Loading a whiteboard

- The view is made with `WebPage::new_with`, `ViewConfig { host: board_host(id,
  created_at), entry: "index.html", source: LibraryBoardSource, .. }` and `ViewExtras {
  bridge: Some(Bridge { name: "innerwordBoard", max_bytes: bridge::MAX_MESSAGE_BYTES }),
  one_document: true, csp: Some(BOARD_CONTENT_SECURITY_POLICY) }`.
- `LibraryBoardSource` (`crates/innerword-app/src/boards.rs`, the app's half of the bundle)
  is an `innerword_web::PageSource` that answers:
  - the bundle's files (`Assets::read`, cached in memory once read: about 1.5 MB at start);
  - `board.json`, one answer so that its parts belong together: `{"stamp": "<ContentStamp>",
    "scene": <the stored scene>, "view": <the row's view_state, or {}>}`, **as they are
    now**, from a cell the source shares with the session (`Arc<Mutex<StoredScene>>`: the
    content and its `ContentStamp`), which the app replaces after every save of the
    whiteboard and every outside write it takes over. So a load at any time (the first
    one, a reload after a crash, Reload whiteboard) starts from what the library holds,
    never from what it held when the view was made. (The export page reads `scene.json`,
    the stored scene alone, section 5.)
  - `library.json`: the stored shape library as it is now, from a cell of its own (4.15),
    **always** (`{}` when there is none: a page that finds no `library.json` starts with an
    empty shape library, which the writer's next "Add to library" would save over the
    stored one).
  - `_files/<FileId>`: the stored bytes of a picture the whiteboard names, through a
    read-only `ImageFile` of its own (as `LibraryPageSource` reads a page's files), with the
    type of the stored bytes (section 2, "Pictures"). Only the `FileId`s the source was told
    about (the stored scene's, and those the app stores or adopts later) are answered; any
    other path is a 404.
- The glue at start (`index.html`): fetches `board.json` and `library.json` (**a
  `board.json` that is missing or is no stored scene** — no string `stamp`, no object
  `scene` with an `elements` list — **is a failed start**: the page posts an `error` whose
  message begins with `bridge::START_FAILED` and never posts `ready`, and the app shows that
  the whiteboard stopped working; built after review, where an empty canvas had been
  offered as the loaded scene); fetches each
  picture (`_files/<FileId>`) and makes it a `dataURL`; waits for every font family the
  scene uses (`document.fonts.load("20px <family>", <its text>)`, a few milliseconds:
  Excalidraw measures text with whatever font is loaded at that moment, and the same text
  measured 26 % wider once Excalifont was there; built: the fonts are loaded through
  `exportToCanvas` onto one pixel first, because Excalidraw adds its faces to
  `document.fonts` only when it exports or mounts, and `document.fonts.load` alone measured
  with the fallback font); then
  `restore(scene, null, null, { refreshDimensions: true, repairBindings: true })` (the two
  only act together), which measures every text again, mends bindings and fractional
  indices, and fills fields a Rust-made element lacks; mounts `<Excalidraw>` with that as
  `initialData` (with the view's scroll and zoom, or `scrollToContent` fitting the drawing
  when the view is `{}` or its `at` is not the stamp it loaded); clears Excalidraw's undo
  history; deletes the kinds of element never kept (4.13); and after two animation frames
  posts `ready` with the stamp it loaded, which becomes the session's base (4.6).
- **Built after the assistant's drawings: the layout follows the width.** Excalidraw
  chooses between its full layout and its compact one (below 730 pixels wide, the class
  `excalidraw--mobile`) in its ResizeObserver (`refreshEditorBreakpoints`, kept in
  `App.device`), which renders only when the container's rectangle differs from its state.
  Its listener of the window's `resize` (`onResize`) has stored that rectangle first, and
  React 19 renders an update made in a `resize` listener at once (a discrete event, in the
  microtask after the listener), with the layout of the old width; the observer then
  changed the choice and rendered nothing. So showing the side chat over a whiteboard left
  the full layout on a canvas 560 pixels wide (the tool bar cut off, the style panel over
  the drawing), hiding it the compact one on 940, until something else made Excalidraw
  render: logged in the page, the `resize`, the render with the old choice, the observer
  with the new one, and no render after it, on every change of width, in a scripted run's
  window and in the WebKit program's offscreen one alike; nothing in it waits for the
  window to be active, so a person sees it too, except where a render something else
  happened to cause (the keyboard leaving the canvas, for one) came after. The
  glue observes the container too (`followTheWidth`, `layout.js`, from an observer made
  after Excalidraw's) and calls `refresh`, a default update rendered in a task after every
  observer: one render more for each change of the container's size. The app's frames
  were never the cause: each change of width came to the page as one `resize` with the
  final size. The WebKit case `board_width` changes the view's width six times and holds
  the layout to it (it failed five of six before).
- **Built:** `PageEvents::loaded` can arrive before the module scripts ran, so the app waits
  for `ready`, never for `loaded`; a scripted run turns occlusion detection off for a
  whiteboard's view too (`pages::draw_when_covered`, which survives a `park`), since
  Excalidraw mounts in animation frames and a view in a covered window never posts `ready`.
- **Opening saves nothing.** The glue takes the restored scene as its starting point (its
  change detector starts there), so the text widths Excalidraw measured again reach the
  library with the writer's next change, not at the open. A whiteboard that was only looked
  at keeps its `updated_at`.
- Measured by the scout (the prototype, macOS 27, arm64): the first view of a run 183 to
  388 ms from load to ready (the WebContent process started), a later one 109 to 214 ms.
  As built: 90 to 440 ms in the debug app (`board:wait` prints "loaded in N ms",
  `BoardState::ready_in`), 129 to 345 ms in the WebKit program; a whiteboard of 3,000
  elements (5,250 with labels, 3.0 MB) 477 ms in the release app.

### 4.4 The bridge: what the page posts

Every message is one JSON string, `postMessage(JSON.stringify(m))` on
`window.webkit.messageHandlers.innerwordBoard`, with `"v": 1` and a `"kind"`. The app reads
it with `innerword_board_web::bridge::parse` (a message it cannot read is dropped and logged
to stderr; nothing it says is ever run).

| kind | fields | when |
|---|---|---|
| `ready` | `excalidraw` (its version), `elements`, `stamp` (the `ContentStamp` of the scene it loaded) | the scene is on the canvas (4.3), and after a reload |
| `scene` | `rev`, `scene` (`elements`, `appState`, `files` without `file` and without `dataURL`) | the drawing changed (4.6) |
| `scenePart` | `rev`, `index`, `count`, `text` (a slice of the scene's JSON, at most 16 MB of UTF-8, cut between characters) | in place of `scene` when the scene's JSON is longer than 16 MB; the app joins the `count` parts of one `rev` and drops an incomplete set when a newer `rev` begins |
| `file` | `id` (Excalidraw's file id), `mimeType` (a hint only: the bytes decide, section 2), `dataURL` | a picture the app has not had yet (pasted), once per id, **before** the first `scene` that names it, and again whenever the app asks for it (`files`, 4.5) |
| `status` | `rev`, `posted` (the last rev posted), `editing` (a text element is being edited), `composing` (an input method is composing in it), `pointer` (`"down"`/`"up"`), `zoom`, `scrollX`, `scrollY`, `elements`, `selected`, `tooLarge` (4.4, size), `libraryWaiting` (a change of the shape library not given to the app yet, 4.15) | any of them changed; zoom and scroll at most every 250 ms |
| `library` | `lrev`, `items` (Excalidraw's `LibraryItems`: `[{id, status, elements, created, name?}]`), `tooLarge` | the shape library changed (`onLibraryChange`), 300 ms after the last change; at once on `pagehide` (4.15). A library longer than one message is posted as `{"lrev": N, "items": null, "tooLarge": true}` and not marked posted; `collect` brings it whole (built after review: the view had dropped it without a word) |
| `help` | none | Excalidraw's `?` key or the main menu's Help: the app opens its list of shortcuts at the place "On a whiteboard" (it opens nothing else; the bridge can ask for no other window) |
| `error` | `message` | an uncaught error, a rejected promise or a refusal of the policy (which says where), at most one a second; the app writes it to stderr. A message beginning with `START_FAILED` (built) is a failed start, with no `ready` |

- **Built:** the posted scene has the `inner-word` envelope, leaves out deleted elements and
  the kinds never kept, and its `files` entries are `{mimeType, created}`, only for pictures
  the canvas holds. A `status` is posted at once on each new rev while the pointer is up;
  while it is down, rev, zoom and scroll go at most every 250 ms. `window.innerwordBoard` is
  frozen and holds `call`, `excalidraw`, `keysTheWindowTakes`, and an `api` getter for the
  tests and `board:js:` only.
- `rev` counts the states of the drawing in this view: the glue's change detector
  (Excalidraw's `hashElementsVersion` over the elements including deleted ones, the four
  `appState` fields, the set of file ids) gives a new `rev` for each state it sees.
- Size: `MAX_MESSAGE_BYTES` = 24 MB (a part of 16 MB with room; a picture of 4 MB,
  Excalidraw's most, is 5.4 MB of base64). The glue never posts a longer message: a scene
  is cut into `scenePart`s instead, so the transport never refuses a writer's drawing. Over
  `MAX_VIEW_SCENE_BYTES` (256 MB) the glue posts a `status` with `tooLarge: true` and keeps
  the scene in the page; the app then asks for it in parts at the deliberate end and writes
  it to the recovery file (4.6). A picture whose `file` message would be longer (Excalidraw
  JSON pasted from elsewhere, where Excalidraw's own 4 MB limit does not hold) is never
  posted (`postFile`, `filesTooLarge` in the glue): the view would drop it without a word
  and the app would wait for it with every save; the `files` call answers it as `missing`,
  so the autosave goes on and the element shows Excalidraw's missing picture, and `collect`,
  an answer and not a message, still brings it whole.
- The text of every message is made well formed before it is posted: the glue replaces a
  lone surrogate (a half of a character pair, which `JSON.stringify` writes as `"\ud800"`
  and `serde_json` refuses in a string) with U+FFFD, by a regular expression, since
  `String.prototype.toWellFormed` is newer than macOS 12.3's WebKit. Rust does the same to
  an escape it still meets (`innerword_board::well_formed_json`) before it parses.
- Measured by the scouts: a post 0.4 to 3 ms; a 20 MB string through the bridge and back
  143 ms.
- **Built (final review), what the app bounds on its side**: the parts of one scene are
  bounded in number and size before they are joined; a `file` message's `dataURL` is
  percent-decoded as bytes (a `%` before a multi-byte character no longer panics); a
  picture file of a drop or of Insert > Image… is read only when it is a plain file, and
  only up to the size `prepare` takes.

### 4.5 What the app asks the page

Calls are `WebPage::call_async` with a body from `innerword_board_web::calls` and one JSON
argument; each answers JSON. They are started from the workspace's tasks (never inside an
AppKit callback), and their answers only post. The glue runs its calls one after another
(a promise chain), in the order the app made them, and each waits for `ready`, **except
`state`, which is answered at once outside the chain** (built after review: behind a
`collect` waiting on a failed start, the scripted `state` line hung).

| Call | Argument | Answer | Used by |
|---|---|---|---|
| `collect` | `{}` | `{"rev", "scene": {…} or null or "parts", "files": [{"id","mimeType","dataURL"}…], "library": {"lrev", "items"} or null, "status": {…}}`: everything the app has not been posted yet (the shape library too, when it changed since its last post); `"parts"` when the scene is longer than one message, in which case its `scenePart`s were posted before the answer and the app waits for all of them | every deliberate end (4.6), an agent's request (6.7), Save Now, Reload whiteboard |
| `files` | `{"ids": [Excalidraw file ids]}` | `{"posted": [...], "missing": [...]}`: a `file` message was posted for each id the canvas has the bytes of | a posted scene names a picture the app does not know (4.6) |
| `adopt` | a delta (below) | `{"rev", "same": bool, "conflicts": [ids]}` | an agent's change, a restored version, a save the store found stale (4.6, 6.7) |
| `undo`, `redo` | `{"steps": n}` | `{"rev"}` | the Edit menu, the agents' note's Undo |
| `selectAll` | `{}` | `{}` | the Edit menu |
| `zoom` | `{"in": true}`, `{"out": true}`, `{"to": 1.25}`, `{"fit": true}` | `{"zoom"}` | the zoom commands |
| `search` | `{"query": "…"}` or `{}` | `{}` | Find, a hit of the library search |
| `insertPictures` | `{"pictures": [{"file", "mimeType", "dataURL", "width", "height"}], "at": {"x","y"} or null}` | `{"ids": [...]}` | Insert > Image…, a drop (4.12) |
| `insertElements` | `{"elements": [...], "select": true, "editText": "<id>" or null}` | `{"ids": [...]}` | the six Add commands (section 1), the scripted step `board:draw:`. An element whose id the canvas has is replaced when the incoming version is higher; new ones go on top. `restoreElements` drops a 0×0 element, so an Add text element has a size. **Built:** the app does not use `editText` (below) |
| `center` | `{}` | `{"x","y"}`: the middle of the view in scene coordinates, and the selected elements' ids | the Add commands, before they convert |
| `mermaid` | `{}`, or `{"insert": "<definition>"}` | `{}`, or `{"ids": [...]}` | "Mermaid to drawing…": opens Excalidraw's dialog (`openDialog: {name: "ttd", tab: "mermaid"}`); with `insert` (the scripted step `board:mermaid:` and WebKit's cases only) converts the definition as the dialog does (`parseMermaidToExcalidraw`, then Excalidraw's `convertToExcalidrawElements` with new ids) and puts it at the middle of the view, selected |
| `library` | `{"show": true}`, `{"hide": true}`, `{"toggle": true}`, `{"addSelection": true}`, `{"insert": "<item id>"}` | `{"lrev", "added": <item id> or null, "refused": "<sentence>" or null}` | the three shape library commands (4.15) |
| `state` | `{}` | the `status` fields | the scripted `state` line |

- **Add Text opens its text for typing without a race** (built: the glue's `editText`
  pressed `enter` before React had drawn the new selection and opened the previously
  selected shape, and an 80 ms timer of the app's lost the first keys to Excalidraw's
  shortcuts, `shift-h` flipping the text). Two call bodies of the app's own, run in the
  page world (`workspace::board::HOLD_KEYS`, `OPEN_TEXT`): `HOLD_KEYS`, sent before
  `center`, holds the writer's key presses without `cmd` or `ctrl` for at most 2 s (events a
  script makes pass); `OPEN_TEXT`, sent after `insertElements` answers, waits two animation
  frames, dispatches `Enter`, waits for Excalidraw's text editor and puts the held keys into
  it. Moving this into the glue is left for later.
- `undo`, `redo` and `selectAll` act on the text being edited when there is one
  (`document.execCommand` in Excalidraw's text area), and on the drawing otherwise, by a
  keyboard event the glue dispatches to Excalidraw's container (`cmd-z`, `cmd-shift-z`,
  `cmd-a`): 0.18.1 has no public undo, and the scout's run took a merged change out and put
  it back that way. The webkit test holds it for the pinned version.
- **`collect` never waits on an animation frame or a timer**: the closing view is parked
  out of sight but drawn (4.2), and still a view that does not wait cannot be stalled by a
  throttled one. It reads Excalidraw's elements as they are
  (`getSceneElementsIncludingDeleted`, which holds what is typed into a text element being
  edited: Excalidraw writes the text into the element at every input), and answers. A
  WebKit case collects from a parked view and from a hidden one. Over 16 MB its scene comes
  in parts, also above 256 MB, so that the app can write the recovery file.

**The delta and the merge** (reviewer's point, taken: 0.18.1's
`restoreElements(incoming, current, …)` raises every incoming element to the canvas's
version + 1 whenever the canvas's copy is newer, so `reconcileElements` would then always
take the agent's copy and drop a move the writer made after the collect). The delta is
`innerword_board::Delta::to_json`:

```json
{"baseRev": 31,
 "upserted": [{"element": {…}, "baseVersion": 3}, {"element": {…}, "baseVersion": null}],
 "removed":  [{"id": "db", "baseVersion": 5}],
 "files":    {"<Excalidraw file id>": {"mimeType": "image/png", "file": "<32 hex>"}},
 "undoable": true}
```

`baseVersion` is the element's version in the stored scene the change was made on (S1),
`null` for an element S1 did not have. `baseRev` is the rev of the view's scene that S1
was saved from (`BoardState::base_rev`: the rev of the post the base was saved from, the
rev an adopt answered, 0 for the scene loaded). **The glue judges "untouched" by its own
record, not by `baseVersion`**: Excalidraw can raise an element's version when it loads a
scene (measuring a Rust-made text again, mending a binding), so the canvas's version of an
element nobody touched may differ from the stored one. The glue keeps, for the scene it
loaded and for each scene it posted or adopted, every element's version on the canvas at
that moment (`versionsAt[rev]`, the last eight revs kept; a `baseRev` it no longer has
counts every element as touched, which can only make conflicts, never lose a stroke).
`adopt` merges by **the base rule**, the same rule `innerword_board::merge` applies in Rust
(on two stored scenes, where `baseVersion` is the right record):

| The canvas's copy (deleted ones included) | An upserted element | A removed element |
|---|---|---|
| at its version of `versionsAt[baseRev]` (the writer did not touch it since S1) | the incoming copy, at S2's version or the canvas's + 1, whichever is higher (Excalidraw's undo wants versions that only grow) | deleted, version raised by one |
| at another version (the writer changed or deleted it since) | **the writer's copy stays**; the id is a conflict | the writer's copy stays; a conflict |
| none, `baseVersion` null | the incoming copy | (cannot happen) |
| none, `baseVersion` set (the writer deleted it and a reload forgot the tombstone) | nothing; a conflict | nothing |
| present, `baseVersion` null (an id the writer gave a new element meanwhile) | the writer's copy stays; a conflict | (cannot happen) |

- **Every wait comes first**: the pictures in the delta are fetched from `_files/<file>`
  (the app told the source about them), then the fonts of every incoming element are
  loaded. Then, in one synchronous stretch with no await, the glue reads the rev and the
  canvas, merges, restores, calls `updateScene` and records the versions (built after
  review: reading the canvas before an await let a drag released meanwhile be replaced by
  the old copy).
- The incoming elements go through `restoreElements(…, null, { refreshDimensions: true,
  repairBindings: true })`, **without the canvas's elements as local elements** (so nothing
  is raised), together with **copies** of the canvas elements they name (a container, a
  frame, the ends of a binding, what is bound to them), which are restored beside them for
  the binding repair and then thrown away; their versions are then set by the glue as the
  table says. **Every other canvas element stays the very object Excalidraw has**, and the
  merge never drops or deletes an element the delta does not name (built after review:
  restoring the whole canvas made a new object of each element and froze a freehand stroke
  in progress, whose rest Excalidraw writes into `this.state.newElement`, and dropped a
  stroke just begun as too small to see; restoring the incoming elements alone would have
  cut a binding to an element the delta does not name). The merged list is ordered by
  fractional `index` (where an incoming and a canvas element share one, the canvas's comes
  first), and its indices are repaired by `updateScene` itself (`syncInvalidIndices` in
  `replaceAllElements`; 0.18.1 exports no function for it). Then
  `updateScene({ elements, captureUpdate: IMMEDIATELY })`: one step of the undo; `NEVER`
  when `undoable` is false. The kinds of element never kept are deleted from the result. An
  incoming element the restore drops as too small to see counts as removed. A canvas
  tombstone that was there at the base, met by an incoming element with `baseVersion` null,
  takes the incoming element (the table's "present" row would have called it a conflict).
- `same` is true when there was no conflict, every version the glue set is S2's (read again
  after `updateScene`: Excalidraw's index repair may raise one), and **the canvas was at
  `baseRev` itself when merged** (built: the spec's rule, "nothing the app had not taken in",
  would answer `same` after a change the glue posted during the adopt's waits, a post the app
  holds aside while it adopts, and that change would have been lost). The canvas then holds
  exactly S2. Otherwise the page posts the merge (taking a new rev when the answered one was
  posted already, through `sceneChanged`, so it waits for a gesture or text edit under way to
  end), and the app saves it (an extra save at worst). The same rule judges the stale-save
  path of 4.6. `conflicts` names the agent's elements that were
  not taken (6.7 says what the note makes of them).
- The export page (`export.html`) answers two calls: `exportLayout`, with
  `{"padding": 32, "maxSide": 14400}` (section 5), and `convertSkeletons`, which runs
  Excalidraw's own `convertToExcalidrawElements` then `restoreElements(…, null,
  {refreshDimensions, repairBindings})` on the skeletons it is given and answers the
  elements: used by the converter's cross-check (section 9, B11) and by nothing in use.

### 4.6 Saving: never lose a stroke

**What the page posts.** The glue's change detector runs on Excalidraw's `onChange` (which
comes for every pointer move, scroll and selection; only a new `rev` counts) and posts the
scene:

- at once at the **end of a gesture**: the pointer released, a text element's editing
  ended, a paste, an undo or redo, a deletion, any change while no pointer is down and no
  text is edited that came from a key (then 150 ms after the last such change, so that
  holding an arrow key to nudge a shape posts once);
- while a text element is being edited, at most once a second (what is typed is in the
  element, and a crash then loses at most a second of it);
- never while the pointer is down (a drag's middle is worth nothing to keep);
- at once on `visibilitychange` to hidden and on `pagehide`.

A picture the app has not had is posted as a `file` message first.

**What the app holds** (`BoardState`, in the open item's session, section 6.1):

- `base`: the stored scene the view's scene descends from, as `(ContentStamp, content)`,
  and `base_rev`, the view's rev it came from: what the view loaded (`ready`'s stamp, rev
  0), then the result of each of the app's own saves (the rev saved) and of each outside
  write the view adopted (the rev `adopt` answered);
- `held`: the newest `scene` the page posted (its raw JSON with its `rev`), `saved`: the
  `rev` last saved;
- `adopting`: while an outside write is being taken into the view, the rev from which
  posts count again (below);
- the view's last `status`;
- `pictures`: Excalidraw's file id to `Stored(FileId, type)`, `Pending`, `Missing` or
  `Refused(sentence)`. **Seeded from `base`'s `files`** when the view is made, and again
  from every scene the view adopts or is restored to, so the first save after an open
  keeps every picture the whiteboard had (reviewer's point, taken: a map filled only by
  `file` messages would drop the entry of every picture already stored).

**The autosave.** A new `scene` message makes the whiteboard dirty. It is saved when no
save is in flight, no picture it names is `Pending`, and no outside write is being
adopted:

1. **Pictures it names that the map does not know** (a `file` message that was lost, or
   pasted JSON naming a file of another whiteboard) become `Pending`, and the app calls
   `files` with their ids. The page posts the bytes it has (stored as below); an id it
   answers `missing` becomes `Missing`, and the save goes on with that element and without
   its `files` entry (Excalidraw shows its missing picture). An id that `base` named is
   never unknown: it is `Stored` from the seed, so a picture is never dropped by a save.
2. In the background, through the saver's second connection, as a text's: `Scene::from_view`
   with the map, `to_stored`, its text and counts, then `Store::save_board(id, content,
   expect: Some(base.stamp))`, then the row's `view_state` with `at` set to the stamp just
   written (`saver::view_state_at`; built after review: the job had written the stamp it
   expected, so after a crash the whiteboard opened fitted), then `snapshot_if_stale`. A
   library in memory (tests) saves at once. **Built:** the second kind is a job of its own,
   `BoardSaveJob` (`Saver::work_board`, and `work_shape_library` for 4.15), sharing the
   saver's sequence and its cancel; a job writes what it saved (`WrittenBoard`) into a shared
   slot (`WrittenSlot`) before it lets the second connection go, and a background save that
   a newer one superseded is retried (the final review).
3. On success, `base` becomes the saved content and its stamp, the source's cell takes it
   (4.3), `saved` is the rev. On `StoreError::Stale` (the row changed under the view by a
   write the app did not take over; every outside write the app knows of is taken over
   first, so this is a guard, not a path): the app reads the stored scene S, sends the view
   `adopt` with the delta from `base` to S (the base rule keeps the writer's changes), sets
   `base` to S, and drops `held`: the page posts the merge, which is saved on S. The
   library never loses the outside write and the canvas never loses the writer's. **Built:**
   when the delta is empty (the stamp changed, no element did) `held` is kept and saved on
   the new stamp (the first build dropped it, and the page had no reason to post again).
   **On the synchronous path a `Stale` never drops `held`** (built after review): a flush
   first takes a background save that already wrote as its base (`settle_board_saves`,
   from the `WrittenSlot`: `settle` had cut such a job off and left the old stamp, so the
   flush's own save was refused), and on a `Stale` it merges in Rust
   (`merge_and_save_now`: `innerword_board::merge(base, held, stored)`, saved on the stored
   stamp, tried again up to three times when the row keeps changing), the view then taking
   the merge in as an outside write; when that save fails too, the scene goes to the
   recovery file. A flush while an adopt is still waiting (`flush_board_while_adopting`)
   merges the newest of the posted scene and the one held when the adopt began. The
   library's own flush settles the saver first (final review).
4. **`Scene::from_view` is lenient**: an element of a type it does not know is kept as it
   came (`ElementType::Other`), so are fields it does not know, and a value of an
   unexpected type (`null` where Excalidraw wrote `NaN`); a lone surrogate is mended
   (`well_formed_json`). It fails only on what is not JSON, or not an object with an
   `elements` array. A save that fails is retried as a text's is. **When the held scene
   cannot be saved** (it fails to parse, it is over `MAX_VIEW_SCENE_BYTES`, or a save failed
   three times in a row), its raw JSON is written at once to a recovery file,
   `<data directory>/Recovered/whiteboard-<id>-<unix ms>.json` (rewritten for each newer
   rev; a library in memory keeps it in `BoardState::recovered`, which tests read), and the
   save state says "Not saved: <the reason>. A copy of the drawing is in <short path>."
   with Show in Finder. At every end of the session a held scene that is not saved is
   written there before the view goes, so a drawing is never dropped (reviewer's point,
   taken): another item opened, the whiteboard closed, a crash, and the flush of closing
   the window and quitting (`Workspace::flush_at_an_end`, also the quit hook). A scene
   already written is not written again, a newer one always is (`recovered_scene`, the
   final review: an earlier copy no longer stops a later one). Reload whiteboard does
   not load the view over a scene it could not save first ("Not reloaded: …"). The
   recovery file is never read by the app: it is for the writer, and for us.

A `file` message goes to the background too: base64 decoded, `sniff_svg` or `prepare`
(section 2, "Pictures"), stored with `LibraryImages::put_files` (the pictures' own
connection), and the map says `Stored` (and the source learns the `FileId`) or `Refused` (a
toast with the sentence). A store that fails (the library busy, the disk full) is no
refusal: the picture stays on its way and is stored again after `RETRY_DELAY`, said once.

**After an outside write** (an agent's change, a restored version, 6.7): from the moment
the app wrote it until `adopt` answers, posts of the view are held and not saved (a post
that left the page before the merge is the scene without the outside write, and would
write over it); `adopting` is the rev `adopt` answers, and only posts at or above it are
saved, on the new `base`. A post below it is dropped: the merged canvas already holds what
it held. **Built:** a post that arrives during the adopt is held aside, not dropped, because
the merge the page posts can arrive before the adopt's answer; it is taken when its rev is
at or above the answered one. When `adopt` fails (a script error, a crash of the WebContent
process, no answer in 3 s, `ADOPT_PATIENCE`): the app collects what it can (1 s), merges in
Rust (`innerword_board::merge(base: S1, writer: the newest of what was collected, what was
posted meanwhile and what was held when the adopt began, theirs: S2)`, the base rule),
saves the result with `expect: Some(S2's stamp)`, and reloads the view from the stored
scene (4.3), whose source now serves it; while it reloads, the old page's posts are dropped
(its `pagehide` post would erase the outside write) until `ready`. When that save fails,
the merge goes to the recovery file and the view loads the merge on the stored scene's
stamp, so the next gesture saves everything. A late answer never acts on a later session
(`adopt_seq` and a session number). **Built:** the first build merged the scene held when
the adopt started and threw away what the page posted meanwhile, losing a stroke the writer
finished during the wait. The writer loses Excalidraw's undo history, never a stroke and
never the agent's change (reviewer's point, taken: a failed adopt used to leave the
canvas without the change, and the next gesture saved it away).

**The synchronous flush** (`Workspace::flush`, called on switching items, before a version
history, by `on_app_quit`): `Saver::settle`, then, when the whiteboard is dirty, the held
scene is written on the main thread with the same compare-and-swap. The deliberate ends
below are asynchronous and wait for a `Pending` picture's background store before they
save. Only the system's quit (`on_app_quit`, below) can meet a picture still `Pending`;
then its bytes are made ready and stored through the library's own connection
(`Store::put_file` of the main `Store`, not `LibraryImages`, whose mutex only background
tasks lock and which may be behind a write of 16 MB), so the save names no bytes that are
not there.

**Getting what the page has not posted yet.** What the app holds can lag the canvas by a
gesture that has not ended, or the 150 ms after a key. The view's last `status` says so
(`rev > posted`, `editing`, `pointer: down`; for the shape library, `libraryWaiting`,
4.15). So every **deliberate end** of the whiteboard's
session first asks the page:

1. **Another item opened** (the library, quick open, the library search, a hit, the
   history of another item, the welcome document), **the whiteboard closed, trashed or
   deleted** by the writer: when the status says the page holds more, the session's view
   is **parked** (4.2) and kept in `Workspace::closing_boards` while the new item opens at
   once; `collect` is called; its answer (or after `VIEW_COLLECT_PATIENCE`, 1 s, without
   it) is saved like a `scene` message, and only then is the view dropped (its snapshot
   pictures handed to `drop_image` first, as `discard_page` does). Opening the same
   whiteboard again while it is closing waits for that save (a deferred reopen is forgotten
   when another item opens meanwhile). When the status says nothing is outstanding, the
   session ends at once, as a page's. **Built:** closing whiteboards are a list
   (`closing_boards`, `Workspace::boards_closing()`), not one slot: with one slot, opening a
   second whiteboard while the first was closing ended the first early and dropped the
   gesture its parked view was about to hand over.
2. **Closing the window and quitting go through one routine**,
   `Workspace::end_then(End::CloseWindow | End::Quit)`: it collects from the open
   whiteboard when its status says the page holds more and finishes the whiteboards in
   `closing_boards` (at most 1 s for all together), waits for pictures still being stored,
   saves (`flush`), and then removes the window or quits. Reviewer's point, taken: today
   none of the paths below reaches an asynchronous step. Who calls it:
   - `CloseWindow` (`cmd-w`, File > Close Window): `on_close_window` calls `end_then`
     instead of flushing and removing the window itself;
   - the window's close button: `on_window_should_close` answers "not now" when a
     whiteboard holds more or one is closing, and calls `end_then`;
   - `Quit` (`cmd-q`, Inner Word > Quit): the app-wide listener of `innerword_app::init`,
     which is `cx.quit()` today, becomes `quit::request(cx)`: with a workspace window it
     dispatches to it and `end_then(End::Quit)` runs; without one (the window that says
     the library is in use) it quits. **Built:** `quit::request` runs after the update that
     asked (`cx.defer`): `cmd-q` pressed in the window reaches the app-wide listener inside
     that window's update, so updating the workspace from it failed and the app quit at
     once without asking the whiteboard (found by the tests of the fix round);
   - `SIGTERM`, `SIGHUP`, `ctrl-c` in the terminal: `signals.rs` calls `quit::request(cx)`
     instead of `cx.quit()`;
   - the last window going (`on_window_closed`) quits after `end_then` has already saved.
   A second Quit while the routine runs quits at once, after the synchronous flush.
3. **A quit the system starts** (log out, restart, the Dock's Quit, an AppleScript `quit`)
   reaches only `on_app_quit`, whose 100 ms cannot wait for WebKit's answer: the open and
   the closing whiteboard are saved as the app holds them, which is the scene of the last
   gesture that ended. That is the one case in which the last unfinished gesture is lost;
   a person check (section 10, check 3) looks at it.
4. **An agent's request** (6.7), **Save Now**, **Export as PDF**, **Print**, **Version
   History**, **restoring a version**, **Duplicate** of the open whiteboard, **Reload
   whiteboard**, **a folder's PDF export** whose folder holds the open whiteboard (it reads
   the stored scene, section 5), **an Add command or a message of the side chat** while the
   page holds more (built): `collect`, save, then go on. Duplicate saves first whenever the
   app holds more than is stored, whether or not the page does (final review). No collect before a move or a
   folder's Markdown, DOCX or plain-text export: a move writes no content (`save_board`
   never touches the folder or the title), and those exports leave whiteboards out, so
   neither reads the scene.

**A crash** of the WebContent process: the held scene is saved at once (the save does not
wait for the debounce), the source's cell takes it, and the view reloads once from the
stored scene; a second crash within 10 s shows "This whiteboard stopped working." with
Reload, as a page shows. Excalidraw's undo history is gone with the process; the versions
are the undo that lasts.

**Reload whiteboard** (the command "Reload page", titled "Reload whiteboard" in the palette
while one is open, its old title among the keywords; and the Reload button of "stopped
working"): collect when the view answers (1 s), save, then reload from the stored scene.
It is there for a view that misbehaves; it costs Excalidraw's undo history and says so in
its toast ("Reloaded. Undo starts again from here; Version History has the earlier
states.").

**The app's own save is not an outside change.** Its `DocumentSaved` reaches the workspace
like every save; for the open whiteboard it is ignored (today a page's reloads the view,
`page_changed_in_library`). Outside changes reach an open whiteboard only through an
agent's request or a restored version, and both adopt (6.7). A rename by an agent changes
the title only; the autosave never writes a title (section 2).

### 4.7 Keys

**Where a key goes**, with the board's view focused (GPUI's focus on the board element,
the web view first responder, kept in step as for a page):

- **Keys without `cmd`** (letters and digits for Excalidraw's tools, `escape`, `enter`,
  the arrows, `delete`, `backspace`, `?`, typing into a text element, `shift`, `alt` and
  `ctrl` chords) reach the web view's `keyDown:`, which asks `takes_key`: the keymap's
  bindings for `[Workspace, BoardView]`. Only what is bound there goes to GPUI's view; by
  default that is `ctrl-enter` and `shift-f10` (the whiteboard's menu). `BoardView` is **a
  place where text is typed** (`keymap::places`, `.typing(BOARD_VIEW)`), so no bare
  character, nor `shift` with one, can ever be bound there.
- **Key equivalents** (`cmd` chords) reach GPUI's view first (the view is a sibling above
  GPUI's; read in AppKit's order and in gpui 0.2.2, seen in background runs, never by a
  person in an active window: section 10, check 1). GPUI takes a chord when a binding on the
  path `Workspace > BoardView` matches **and** its action has a handler there; otherwise
  AppKit goes on to the web view (Excalidraw) and then to the menu bar.

**The window keeps its keys** (their actions have handlers on a whiteboard): `cmd-shift-p`
palette (Excalidraw's own palette never opens), `cmd-p` quick open, `cmd-n`, `cmd-shift-n`,
`cmd-0` library, `cmd-1` the whiteboard, `cmd-2` (a toast, 6.3), `cmd-3` side chat, `cmd-,`
settings, `cmd-/` the list of shortcuts, `cmd-s` Save Now, `cmd-shift-s` the export picker
(so Excalidraw's "save to disk", which has no switch, never runs), `cmd-=`/`cmd-+`, `cmd--`
and `cmd-shift-0` the zoom (routed to Excalidraw's zoom, 4.9), `cmd-f` Find (bound once more
in `BoardView`, routed to Excalidraw's search, 4.10), `cmd-alt-s` and the sidebar's width,
`cmd-shift-f` library search, `cmd-alt-a` the Agents panel, `cmd-alt-p`, `cmd-alt-m` and `cmd-j`
(toasts, 6.3), `cmd-w`,
`cmd-q`, `cmd-h`, `cmd-m`.

**The whiteboard's own keys** (`innerword_board_web::keys::WHITEBOARD_KEYS`, read from
0.18.1's key tests; B3 read them again in the package and corrected the list: **43 keys** in
`innerword-keymap`'s normal form, with `alt-up`, `-down`, `-left`, `-right` (Excalidraw's
flowchart navigation) added, and `cmd-enter` also editing a line's points; `BoardKey`,
`TOOL_KEYS_ROW`, `is_whiteboard_key`):

| Keys | Excalidraw | What the window binds there today |
|---|---|---|
| `cmd-z`, `cmd-shift-z` | undo, redo | (the text's; not on a whiteboard's path) |
| `cmd-c`, `cmd-x`, `cmd-v`, `cmd-shift-v` | copy, cut, paste, paste as plain text | (the text's) |
| `cmd-a` | select all | (the text's) |
| `cmd-d` | duplicate | |
| `cmd-g`, `cmd-shift-g` | group, ungroup | find next and previous (text, page) |
| `cmd-[`, `cmd-]` | send backward, bring forward | outdent, indent (text) |
| `cmd-alt-[`, `cmd-alt-]` | send to back, bring to front | **previous and next comment, window-wide** |
| `cmd-alt-c`, `cmd-alt-v` | copy styles, paste styles | **show comments and Vim mode, window-wide** |
| `cmd-shift-up`, `-down`, `-left`, `-right` | align top, bottom, left, right | (the text's selection) |
| `cmd-shift-,`, `cmd-shift-.` | smaller, larger font | smaller, larger text (text) |
| `cmd-'` | grid on or off | quote (text) |
| `cmd-k` | link of an element | link (text) |
| `cmd-shift-l` | lock or unlock | align left (text) |
| `cmd-up`, `-down`, `-left`, `-right` | flowchart: a next node | (the text's) |
| `cmd-enter` | confirm in its dialogs | page break (text) |
| `cmd-backspace`, `cmd-delete` | clear the canvas (it asks) | (the text's) |
| `alt-z`, `alt-s`, `alt-/`, `alt-r`, `alt-h`, `alt-v`, `alt-shift-c` | zen mode, snapping, statistics, view mode, distribute, copy as PNG | |

- **How GPUI is kept off them**: after every binding of the app, the app binds each of them,
  **in every spelling `innerword_keymap::Keystroke::spellings` gives** (`cmd-shift-,` and
  `cmd-shift-.` also as `cmd-<` and `cmd->`, the only two with a second spelling, which a
  test holds: 45 bindings for 43 keys; `boards::WhiteboardKeys::fixed_bindings`,
  `key_spellings`, `is_fixed_binding`), to `gpui::NoAction` in `BoardView` (**fixed
  bindings**: not defaults, not rows of the list's commands, not in the shortcuts file),
  **each key unless the user gave it to a command that works on a whiteboard** (below,
  "A key the user gives a command"). They are appended **after**
  `keymap::take_defaults` (`lib.rs`), at the end of both `keymap::install` and
  `keymap::measure_install`, so they never become defaults or rows, and the test that
  puts the keymap back from its parts (`keybindings.rs`,
  `the_keymap_put_back_from_its_parts_resolves_every_key_as_before`) adds them to its
  expected keymap; the binding count the `keyscost` step reports grows by them. In gpui
  0.2.2 a binding without a context ranks like the deepest context, and among bindings at
  one depth the one added later wins; a `NoAction` that wins disables every binding under
  it (`Keymap::bindings_for_input`, read). So on a whiteboard, `cmd-alt-c` matches nothing:
  GPUI leaves the chord to the web view, and `takes_key` answers false for it. Elsewhere in
  the window `cmd-alt-c` still shows the comments.
- **A key the user gives a command** (built after a user's report: the palette changed to
  `cmd-k` opened Excalidraw's link editor on a whiteboard, because the fixed binding of
  `cmd-k`, Link of the selected element, came last and won over the user's own change).
  The rule: **a key the user gave a command that applies where a whiteboard has the
  keyboard is no longer held for Excalidraw**. One function decides it,
  `boards::whiteboard_keys(installed, effective)`: for each key of `WHITEBOARD_KEYS`, in
  every spelling, the binding GPUI would run in `[Workspace, BoardView]`
  (`Keymap::bindings_for_input` over the bindings that run, without the fixed ones); when
  that winner is a key the user made (`Origin::is_users` of `innerword-keymap`'s
  `Effective`: a changed or an added key, never a default) in every spelling that matches
  there, the key is given up to that command. So a command that works anywhere, and one
  bound in `BoardView`, have it on a whiteboard; a command of the text does not apply
  there and changes nothing. A key whose winner there is a default stays held as before:
  the window's defaults are kept off Excalidraw's chords except the comments' and Vim
  mode's, which have always been held, and a default that ever collides keeps the hold.
  The keymap works it out at every install (`keymap::install` and `measure_install`, one
  `assemble`), keeps it (`keymap::whiteboard_keys`), and five places ask it, so they cannot
  disagree: the fixed bindings (none for a key given up, so GPUI runs the user's command
  and AppKit never reaches the web view); the menu bar while a whiteboard is open
  (`menus::menu_key_bindings(.., whiteboard)`, `WhiteboardKeys::holds_from`: the key stays
  beside the user's command and beside no other item, so `Link…` shows none there); the
  fixed row of the list (6.8); the question about a key (6.8); and the page, whose
  tooltips and menus stop naming a key given up (`WhiteboardKeys::given_up`: `keysGivenUp`
  in `board.json` at every load, and the call `keys` when an install changes them while the
  whiteboard is open, `Workspace::tell_the_board_its_keys`; "The keys Excalidraw shows"
  below). An import of a shortcuts file and "reset all" go through the same install.
  Excalidraw's function of a key given up is reached from its own menus and toolbar.
- **How the menu bar is kept off them** (reviewer's point, taken: an item is enabled
  whenever its action has a handler anywhere, and four of these have app-wide listeners:
  `ToggleComments`, `NextComment`, `PreviousComment` in `innerword-comments`, `ToggleVimMode`
  in `innerword-vim-mode`; so their items stay enabled on a whiteboard, and when AppKit
  offers a chord to the menu bar first, `cmd-alt-c` would show the comments): **while a
  whiteboard is the open item, the menu bar is built without any key equivalent that is in
  `WHITEBOARD_KEYS` and held** (a key given up to a command of the user's stays beside that
  command), except the six of the Edit menu that AppKit sends to the web view
  (Undo, Redo, Cut, Copy, Paste, Select All; `menus::keeps_its_key_on_a_board`): the web
  view asks the page first, and Excalidraw takes undo, redo and select all on the canvas,
  while copy and paste reach it only as the `copy:` and `paste:` those items send (0.18.1
  has no key handler for them), and a text field of the page needs `selectAll:` (final
  review: without them `cmd-c` and `cmd-v` did nothing on a whiteboard).
  `menus::menu_key_bindings(keymap, open: OpenItemKind)` leaves the others out for
  `OpenItemKind::Board`, and keeps every key while the whiteboard is not drawn (no engine,
  a page that failed to start); the workspace rebuilds the menu bar when a
  whiteboard becomes or stops being the open item (1 to 5 ms, the cost of a key change,
  measured for 0.5.0), as `menus::set_assistant_shown` does. A click on such an item still
  works: on a whiteboard the workspace root's `capture_action` (which already captures the
  comment actions) answers Show or Hide Comments, Next and Previous Comment and Vim Mode
  with the toasts of 6.3 and stops them, so the comments of a text that is not shown and
  Vim mode are not touched. Not taken from the review: leaving these four without a toast.
  A handler only makes GPUI take a key that a binding on the path matches, and on a
  whiteboard the fixed `NoAction` bindings match first; so the toast answers a click and
  never a key, and without it a click on the item would toggle the comments or Vim mode of
  the text that was open before. Find Next and Find Previous and the text's commands have no
  handler there (greyed). A test: with a whiteboard open, `menukey:` of each of those items
  is empty, and after it closes the keys are back.
- **The keys Excalidraw shows.** Excalidraw's interface names keys in two places, and a key
  it names must be one that does it here (product rule 3). Its help dialog is not rendered
  (our main menu's Help and its `?` key post `help`, and the app opens its own list of
  shortcuts at "On a whiteboard", 4.4). Its tooltips and the keys of its menus (the
  right-click menu, the main menu) keep the keys that are Excalidraw's on a whiteboard (the
  tool letters and digits, and the chords of `WHITEBOARD_KEYS` that are held), and lose
  every key the window takes and the user can change (zoom in, zoom out, reset zoom, the
  command palette, the help, the library's search, save and export: `keys.js`,
  `KEYS_THE_WINDOW_TAKES`) and every chord of `WHITEBOARD_KEYS` the user gave a command (the
  app tells the page which, above). A `MutationObserver` reads every `title`,
  `aria-keyshortcuts` and key of a menu item as Excalidraw writes it (`getShortcutKey`:
  "Cmd+Option+[", read into the keymap's normal form, `cmd-alt-[`), cuts those keys out (a
  menu item's key is hidden), and puts a key back when the app gives it back. WebKit cases
  read every tooltip against the window's list, and a key given up and one held in the
  tooltips and the right-click menu, at the load and after the call `keys`
  (`keys_given_up`, `tests/webkit/bundle.rs`).
- **The Edit menu.** On a whiteboard the workspace's `edit_where_the_keyboard_is` sends Copy,
  Cut and Paste to the web view (`tryToPerform:copy:` and the others: the page gets its
  clipboard events, which Excalidraw handles) and Undo, Redo and Select All to the glue
  (4.5), never to the web view's `NSUndoManager` or `selectAll:`. A `cmd-z` reaches either
  Excalidraw's own key handler (the view asked first) or the menu's Undo (the menu asked
  first); both undo once. **Built (follow-up): a WebKit case holds it**
  (`board_edit`, `tests/webkit/bundle.rs`, `the_edit_menu_on_a_whiteboard`): `paste:` and
  `copy:` sent to the whiteboard's view as the app sends them (`WebPage::edit`,
  `tryToPerform:` on the view, its window's first responder) reach the page as `paste` and
  `copy` events, Excalidraw puts the pasted text on the canvas and writes its own
  clipboard (`excalidraw/clipboard`) of the selection, and the glue's Undo, Redo and
  Select All take the paste out, bring it back and select the whole drawing. WebKit reads
  and writes the pasteboard in the UI process, through `PlatformPasteboard`, which asks
  AppKit for it by name: for the length of the case `+[NSPasteboard generalPasteboard]` and
  `+[NSPasteboard pasteboardWithName:]` with the general name answer a pasteboard the test
  made with a unique name (`OwnPasteboard`, put back on drop), the case checks that AppKit
  answers it before it sends anything, sends `copy:` only once the paste read the test's
  text, and checks the change count of the user's general pasteboard unchanged at the end.
- **A command that opens a field or a panel in the page** (Find, Mermaid to drawing, the
  shape library) gives the page the keyboard as it opens it (final review).
- **`escape`** is Excalidraw's (deselect, leave a text, close its dialogs). The ways out of
  a whiteboard are the window's keys and clicks.
- **Keys the glue keeps from Excalidraw** (built): `cmd-s`, `cmd-shift-s`, `cmd-o`,
  `cmd-shift-e`, `cmd-/`, `cmd-shift-p`, `cmd-p` and `alt-shift-d` are stopped before
  Excalidraw sees them, without `preventDefault`, so the menu bar still gets them; the glue
  also closes Excalidraw's palette and export dialogs should one open
  (`keysTheWindowTakes`).
- **Typing with an input method** into a text element is WebKit's; the glue reports
  `composing` (`compositionstart` to `compositionend`), and an agent's change waits for it
  (6.7).
- **What a script cannot do**: Excalidraw's keys are the page's own handlers; no key reaches
  the app through the bridge.

### 4.8 Pointer

Everything on the canvas is Excalidraw's: drawing, selecting, dragging, its right-click menu,
its style panel. Over a whiteboard the page element sets no cursor (WebKit's cursors). The
app's items are on the title's chevron (product rule 1: a small visible control) and in the
menu bar.

### 4.9 Zoom, scroll, pinch, theme

- The app's zoom commands act on **Excalidraw's canvas zoom**, never on the web view's
  `pageZoom` (which would scale Excalidraw's toolbar too): Zoom In and Zoom Out call `zoom`
  with `in`/`out` (Excalidraw's own steps), Actual Size sets 100 %, Zoom to 50 % … 400 %
  set that, **Fit Width and Fit Page show the whole drawing** (`scrollToContent` with
  `fitToContent`, as Excalidraw's own `shift-1` does), Life Size and Automatic say what a
  whiteboard is (6.3). The status bar shows the zoom of the last
  `status`, and its menu has Zoom In, Zoom Out, Actual Size, Zoom to Fit and the steps.
  Excalidraw's own range (10 % to 3,000 %) holds; the wheel with `cmd` and Excalidraw's own
  buttons zoom too. The zoom is not one of the app's stored zooms (`view.zoom.*`); it is the
  whiteboard's `view_state`, written with each save and when the whiteboard closes.
- Scrolling is Excalidraw's (two fingers, the wheel, `space` with a drag, the hand tool).
- **The pinch**: `Workspace::pinch` does nothing without a text session, so a magnify event
  over a whiteboard goes to WebKit untouched, where Excalidraw zooms on Safari's gesture
  events (`allowsMagnification` stays NO, so WebKit does not magnify the page). Not tried by a
  person: section 10.
- **Theme**: the view's `NSAppearance` follows the app's theme as for a page; the glue
  listens to `matchMedia("(prefers-color-scheme: dark)")` and sets Excalidraw's `theme`
  (Excalidraw draws its dark mode by inverting the canvas). Excalidraw's own theme switch is
  off. The PDF is always made in light colours (5).

### 4.10 Find

`editor_tools::Find` is bound once more in `BoardView` (`cmd-f`, as in `PageView`) and has a
handler on a whiteboard: the glue's `search` opens Excalidraw's own search (its sidebar's
"Search" tab), which finds text on the canvas, steps through the hits with its own
`enter` and `shift-enter`, and closes with `escape`. Find and Replace, Find Next and Find
Previous have no handler on a whiteboard (greyed; `cmd-g` is Excalidraw's group). A hit of
the library search on a whiteboard opens it and searches the query.

### 4.11 Links

An element can carry a link (`cmd-k` in Excalidraw). **A link opens in the reader's browser
only when the reader opened it**, the gate pages already have, unchanged:

- Excalidraw's link popover above a selected element shows the address as a real `<a>`: a
  click on it is a real `mouseDown:` on a link, so the gate opens that address, once.
- A click on the small link mark on the canvas makes Excalidraw call `window.open`: no `<a>`
  lies under the pointer (it is a canvas), so the gate opens nothing. The glue's
  `onLinkOpen` therefore does nothing for `http`, `https` and `mailto` (the popover's anchor
  goes on to the gate), scrolls to the element for an element link of the same whiteboard
  (Excalidraw's `?element=` links; `preventDefault`, so no navigation is tried), and calls
  `preventDefault` for anything else.
- **No message of the bridge opens a link**, ever: a script could send it at any time, which
  is what the gate forbids (pages' review, ISO-4).

### 4.12 Pictures

- **Paste** (`cmd-v`, Excalidraw's menu): Excalidraw's own; it resizes a picture over 1440
  pixels and refuses one over 4 MB after that. The glue posts the new file (`file`), then the
  scene; the app stores the bytes (4.6).
- **A drop from the Finder**: the web view takes no drops (`pages::take_no_drops`, as for a
  page), so a drop reaches GPUI's view and the board element's `on_drop`: picture files
  (by `innerword_image`'s own reading: the same files Insert > Image… takes) are read,
  `prepare`d and stored in the background, then `insertPictures` puts them on the canvas at
  the point of the drop, each at its own size up to 1440 pixels (Excalidraw's own limit),
  selected; `.html` files and folders are added as pages, as everywhere; anything else is
  left out with the toast a drop on the text gives.
- **Insert > Image…** (`workspace::InsertImage`, the menu bar, the title menu's "Insert
  Picture…", the palette's "Image"): the open panel (`images::choose`, the seam tests already
  use), then as a drop, at the middle of the view.
- A picture the app inserted has `FileId` hex as its Excalidraw file id; its bytes are
  stored before the glue is asked, so it is `Stored` from the first moment.
- Copying a picture out (`cmd-c`) puts Excalidraw's clipboard data on the clipboard, as
  Excalidraw does in a browser.
- **An agent's picture** (7.2) arrives as part of the agent's change: stored before the
  change is written, then adopted into the open view with its `files` entry (6.7), the
  source told its `FileId`, so the page fetches it from `_files/` like a picture the
  whiteboard had. A Mermaid diagram of a kind Excalidraw does not turn into shapes arrives
  as an SVG picture through the `file` message, like a pasted one (4.13).

### 4.13 What is switched off in Excalidraw

| What | How |
|---|---|
| Open, Save to…, Export image, the file actions | `UIOptions.canvasActions`: `loadScene: false`, `saveToActiveFile: false`, `export: false`, `saveAsImage: false` (Export is the app's: PDF) |
| Its theme switch | `toggleTheme: null` (the app's theme rules) |
| Its image tool (an open panel the view cannot show) | `UIOptions.tools.image: false`; pictures come by paste, drop and Insert > Image… |
| Live collaboration, share links | not rendered (no `LiveCollaborationTrigger`); their addresses would be blocked anyway |
| Web embeds (YouTube, Figma, gists), and the `iframe` element whose `customData.generationData` Excalidraw renders as a frame's `srcdoc` with `allow-scripts` (0.18.1, read) | `validateEmbeddable={false}` (a pasted address stays a link); `.excalidraw__embeddable-container` hidden by CSS and the embed tool's menu item removed by the glue (no prop hides them; the item shares its `data-testid`, `toolbar-embeddable`, with Mermaid's, so not by that: by its `onSelect`'s label, the en locale's `toolBar.embeddable`, which B3 reads in 0.18.1, with a `MutationObserver` on the extra tools' dropdown); **the glue deletes every `iframe`, `embeddable` and `magicframe` element the moment it is on the canvas**: in `onPaste` before Excalidraw takes the clipboard, in `onChange` (an undo can bring one back; deleted by an `updateScene` with `captureUpdate: NEVER`), in `adopt` and at the load; Rust drops them from every scene read; and the whiteboard's policy (4.2) lets no frame and no inline script run in the meantime. A WebKit case pastes such an element with a `srcdoc` script that would post a message: no message, and the element is gone |
| Text to diagram, wireframe to code (AI) | `aiEnabled={false}` |
| Mermaid to Excalidraw | **kept** (answer 4): its item under "Generate" in the extra tools' menu, its dialog (`ttd`, tab `mermaid`; with `aiEnabled={false}` the dialog has no AI tab), and Excalidraw's conversion of pasted Mermaid text; the app's command "Mermaid to drawing…" opens the dialog (`mermaid` call). What it needs at run time is below the table |
| The shape library | **kept and stored** (answer 6, section 4.15): its sidebar tab, "Add to library" and the items. Out of reach: "Browse libraries" (`.library-menu-browse-button`, a link to libraries.excalidraw.com), "Publish library" (a `POST` to a cloud function), the library menu's Load and Export (an open panel the view answers with nothing, and a download the view cancels), each hidden by CSS on its class or `data-testid` (`lib-dropdown--load`, `lib-dropdown--export`, the publish item), which B3 reads in 0.18.1; the policy and the rule list block the addresses anyway, and `useHandleLibrary` (which reads `#addLibrary=` from the address) is never called |
| Its command palette (`cmd-/`, `cmd-shift-p`) | the window keeps both keys (4.7); its main menu item is not rendered |
| The main menu | our own `<MainMenu>`: Find on Whiteboard (Excalidraw's search), Keyboard Shortcuts (posts `help`: the app's list at "On a whiteboard"), Clear Canvas, Canvas Background. No Open, Save, Export, collaboration, "Excalidraw+", social links |
| Its help dialog and the `?` key | not rendered; `?` posts `help` (4.7) |
| The welcome screen on an empty canvas | our own `<WelcomeScreen>` with Excalidraw's hints of the toolbar and the help, and no links |
| The language | `langCode="en"` |
| Keys | `handleKeyboardGlobally={true}` (the page is only the whiteboard), `autoFocus` |
| Tracking | off in the published package already (`VITE_APP_ENABLE_TRACKING: "false"`); no Sentry |
| Its font CDN | patched out (section 3) |

The scout measured, with all of this in place and the app's exact CSP and rule list: no CSP
report from a load, editing, an SVG or PNG export, a PDF, or Mermaid, except the three
requests the scout made on purpose, all blocked. That was under a page's policy, which
allows eval: B4's bundle case does Mermaid again under the whiteboard's, and it holds (a
flowchart in about 70 ms, a sequence diagram, a pie chart as an SVG picture posted as
`file`, a pasted flowchart; an image node refused by `img-src`; an `onerror` label posts
nothing).

**Built**, beside the table: the `iframe` row's last clause is now that a view with the
bridge loads no frame at all (4.2), the policy alone not being enough for a `srcdoc` that
names the bundle's own script; the main menu is Find on Whiteboard, Keyboard Shortcuts,
"Reset the canvas" (Excalidraw's own label for Clear Canvas) and Canvas background; the
welcome screen says: Pick a tool and draw. The whiteboard keeps every stroke as you go.

**What Mermaid to drawing needs at run time**, read for this design in the prototype's
`node_modules` (`@excalidraw/mermaid-to-excalidraw` 2.2.2, `mermaid` 11.17.2) and in the 105
chunks the prototype's full build made of it (3,493,120 bytes, `board/proto/dist`; the built
bundle has 100 chunks, 2.66 MB, without the locales):

- **No eval.** No `eval(`, no `new Function`, no string timer in any of the chunks. The one
  `Function(` is lodash's `freeGlobal || freeSelf || Function("return this")()` (four
  copies; six in the whole built bundle), which a window never reaches: `self` is there.
  (Built: the whole bundle has one dead `(0,eval)(t)` of chevrotain, after a `return`; the
  test counts it.) So the live view's
  `script-src 'self'` holds, without `'unsafe-eval'`.
- **No worker, no WebAssembly, no `blob:` script.** Its `Blob`s are data handed to a canvas.
- **No request.** Its addresses are documentation strings and namespaces. Two ways a
  diagram can name one: an image node (`@{ img: "https://…" }`, a `new Image` in the
  flowchart renderer) and a `click … href` link. The first is refused by `img-src` and the
  rule list (the node is drawn without its picture, or the dialog shows Mermaid's error);
  the second only binds a handler on the SVG that the conversion renders off screen and
  removes, and a navigation is refused anyway (`one_document`; a link reaches the browser
  only through the reader's act, 4.11).
- **The DOM, and nothing out of the page.** `parseMermaid` puts an off-screen `div` into the
  page's `body`, has `mermaid.render` draw the SVG into it, measures it (`getBBox`,
  `getComputedTextLength`, `getBoundingClientRect`) and removes it. Mermaid's SVG carries an
  inline `<style>`: allowed by `style-src 'unsafe-inline'`, which the policy has already.
  Mermaid stays at its `securityLevel: "strict"` (Excalidraw passes no configuration, and a
  diagram's own `%%{init}%%` cannot change `securityLevel`, which is in Mermaid's `secure`
  list): labels go through DOMPurify, no click callback runs, the `sandbox` level's
  `iframe` is never made (and `frame-src 'none'` would refuse it). Even a script that got
  past DOMPurify into that SVG would not run: the policy allows no inline script and no
  inline handler.
- **Fonts.** Mermaid lays out in its default family (`"trebuchet ms", verdana, arial,
  sans-serif`): fonts of every Mac, nothing fetched. KaTeX's JavaScript is in the chunks
  and its fonts are not in the bundle; no `@font-face` names one (read: no `KaTeX_` in the
  built files), so a math label (`$$…$$`) is drawn with the system's fonts. What reaches the
  canvas is Excalidraw elements in Excalidraw's fonts.
- **What it makes.** Flowcharts, sequence, class, ER and state diagrams become elements
  (shapes, arrows, labels), which the autosave keeps like any drawing. Any other kind (pie,
  Gantt, mind map…) becomes one picture, an SVG `data:` URL as a file of the scene, which
  goes through the `file` message, `sniff_svg` and the 4 MB limit like a pasted SVG (one
  larger is refused with its toast). The dialog remembers the last text in `localStorage`
  (Excalidraw's `mermaid-to-excalidraw` key), which lasts while the app runs and belongs
  to that whiteboard's origin.
- Measured by the scout (prototype, page policy): a flowchart of 13 elements in 57 ms, its
  chunks loaded from the bundle.

### 4.14 No engine, and the overlays

- **Without a page engine** (macOS below 12.3, `INNER_WORD_PAGES=off`): opening a whiteboard
  shows its listing of elements (section 7) in the quiet column a page's text uses, under the
  sentence "Whiteboards need macOS 12.3 or later." (`innerword_web::available`'s, worded for
  whiteboards: `boards::not_shown_sentence`) or "Whiteboards are not shown in this run."; it
  cannot be changed or exported in the app; New Whiteboard is not there. A whiteboard whose
  page failed to start (`START_FAILED`) is "not drawn" too: the drawing commands are not
  offered and say why ("This whiteboard could not be shown, so nothing can be added to
  it."), the status bar shows the listing's count and no zoom, and the menu bar keeps every
  key (final review).
- A modal, the palette or a context menu over a whiteboard: the cover, as for a page (the
  writer cannot draw meanwhile, which is right). Toasts get holes, as for a page.
  **Built, and replaced after 0.11.0:** at first the view was hidden under a snapshot, as a
  page's was. On Excalidraw's canvas the snapshot took WebKit 170 ms (14 ms on a page) and
  its pixels 90 ms more in the debug build, so quick open came 300 ms after `cmd-p`, the
  sidebar dimmed long before the canvas (the "slow, with flickering" a person reported),
  and the hidden view told Excalidraw `visibilitychange`, on which the glue posts what it
  holds, and stopped its frames. Now the view is never hidden for an overlay: it is covered
  (`WebPage::set_cover`: the modal's backdrop drawn natively over it, no event reaches it)
  and the overlay is seen through a hole, a frame after it opens (`docs/gpui-notes.md`, "A
  native view in the window"). Measured in scripted runs of the debug build with the
  window recorded: the modal 54 ms after `cmd-p` (a text: 43 ms), the canvas dimmed with
  the sidebar, closing in one frame; the page posts nothing.

### 4.15 The shape library

Excalidraw's library is a sidebar of shapes the writer keeps to reuse ("Add to library" on a
selection, then a click or a drag puts a copy on the canvas). In a browser it lives in the
browser's storage; here the view's data store is not persistent, so **the app keeps it in
the library** (answer 6).

- **Where.** One setting of the library, `board.shape_library`, in `library.db`'s settings
  like the others: Excalidraw's own library file form, `{"type":"excalidrawlib",
  "version":2,"source":"inner-word","libraryItems":[{"id","status","elements","created",
  "name"?}, …]}`, minified. One library for every whiteboard of the library, as
  Excalidraw's is one per browser. Not a row: it is no item of the tree, has no title, no
  versions, no trash and no words to search, and a setting is in every daily backup of
  `library.db` as it is. Absent: the library is empty.
- **Only `innerword_board::ShapeLibrary` reads and writes it** (`from_stored`, `from_view`,
  `to_stored`). Each item's elements are read as leniently as a scene's (unknown fields
  kept, lone surrogates mended); **an item that holds a picture, an `iframe`, an
  `embeddable` or a `magicframe` is dropped** with a warning on stderr. Excalidraw 0.18.1
  refuses those in "Add to library" already (`LIBRARY_DISABLED_TYPES`: `iframe`,
  `embeddable`, `image`, read in the package, with its own message), so a library item
  never names stored bytes: the setting has no `files`, and the collector's table needs no
  row for it.
- **Limits** (`innerword_board::limits`): `MAX_LIBRARY_BYTES` 8 MB stored, `MAX_LIBRARY_ITEMS`
  1,000 (a shape is about 600 bytes; a thousand items of ten shapes is 6 MB). A library
  over either is not stored: the last one kept stays in the setting, the canvas keeps what
  the writer has until the whiteboard closes, and one toast says "The shape library is too
  large to keep (over 8 MB). Remove some of its items; until then the last library kept
  comes back."
- **Loading.** The view's source serves `library.json` (`innerword_board_web::LIBRARY_PATH`):
  the stored setting as it is now, from a cell the session shares with the source as it
  shares `board.json`'s (4.3), which the app replaces after each save of the library. The
  glue fetches it beside `board.json` and mounts Excalidraw with it as
  `initialData.libraryItems`. The export page never reads it.
- **Saving.** The glue posts `library` (4.4) on `onLibraryChange`, 300 ms after the last
  change, with a counter of its own (`lrev`), and not for the load's own first call (its
  items equal what it was served). The app holds the newest post (`ShapeLibraryState {
  held, posts, saved, stored }` in the workspace, not in a whiteboard's session: it
  outlives one) and writes it in the background through the saver's second connection
  (`Saver::work_shape_library`: `ShapeLibrary::from_view`, the limits, `to_stored`, the
  setting), newest wins. **Built after review:** the page's `lrev` starts at 0 in every
  load, so it counts per view (`BoardState::library_lrev`, back to 0 at `ready`) and the
  workspace numbers the posts it takes itself (`posts`, across views); comparing every
  view's `lrev` with one held number had dropped the changes of a second whiteboard, or of
  one reopened or reloaded, until its counter passed the first's. Loading a whiteboard
  stores a held library change before it serves `library.json`. A post with `tooLarge`
  keeps the stored library and says so in the toast; its `items` (`null`) are never
  stored; the
  synchronous flush writes what is held; every `collect` (4.5) brings a change not posted
  yet, so the deliberate ends of 4.6 keep the last one. A change the app's own `library`
  call answered (Add selection to shape library: its `lrev` above the last post taken)
  counts as "the page holds more" until it is posted, so a deliberate end collects then
  (final review). **Built (follow-up): the status says when a change waits.** A change made
  in Excalidraw's sidebar was known to the app only from its post, 300 ms later, so a
  deliberate end inside those 300 ms could lose it. The page's `status` (4.4) and the
  status `collect` answers carry `libraryWaiting` (`Status::library_waiting` in
  `innerword_board_web::bridge`; absent in a status of an older glue: false): **true from
  Excalidraw's `onLibraryChange` (or the `library` call) until the `library` message is
  posted, or until `collect` brings the change, and for as long as the library is too long
  for one message** (posted as `tooLarge`, which does not count as given: `collect` brings
  it whole). A status is posted at once when it turns true and when it turns false (the
  glue's `whenLibraryWaitingChanges`, set to `sendStatus`), so the app knows within a turn
  of the page. `collect` brings a waiting change at once (its `library` field), and the
  status it answers says false. It is **not** part of `Status::holds_more`, which is about
  the drawing: the app collects at a deliberate end while either holds
  (`BoardState::holds_more`: the drawing's `Status::holds_more`, `library_waiting`, or a
  call's answered `lrev` above the last post taken, which still covers a status of an
  older glue). So switching to another item, closing, quitting, Duplicate and a PDF
  export collect a change made in Excalidraw's sidebar inside its 300 ms. The calls that
  collect only for the drawing (an agent's request on the open whiteboard, a message of
  the side chat, an Add command) keep to `Status::holds_more`: none of them reads or
  changes the shape library, and a waiting change still reaches the setting at its post or
  at the next deliberate end. The fake page engine models the field (a change not posted
  yet posts a status saying true; its post, and `collect`, say false); the app's tests
  `a_shape_library_change_waiting_its_post_is_kept_by_a_switch_at_once` (a sidebar change,
  the drawing untouched, another document opened at once: the page is asked and the
  setting holds the change) and
  `a_shape_library_change_a_call_answered_is_collected_without_the_status`. Checked in a
  scripted run of the debug app on a temporary library: Excalidraw's own `updateLibrary`
  (`board:js`), `wait:40`, `keys:cmd-n`: the new item was in the setting and came back
  after a relaunch; the same run with the field left out of `holds_more` lost it. The
  same for "Add selection to shape library" followed by `cmd-n` 50 ms later. The WebKit case `a_library_change_waiting_its_post` holds
  the status posted at the change, the `state` call, `collect` bringing it inside the
  300 ms and answering false, nothing posted after it, and the post and false without
  `collect`; `a_library_longer_than_one_message` holds the status true while too large and
  false after `collect`. Only one whiteboard view lives at a
  time, so there is no merge between two libraries; the app is the setting's only writer
  (no tool and no MCP tool reads or writes it, and the live channel has no operation for
  it).
- **Keys** (product rule 1; Excalidraw's sidebar is made for the pointer): "Show or hide
  the shape library" (the sidebar at its Library tab), "Add selection to shape library"
  (the glue makes the item from the selection as Excalidraw's own action does, labels and
  a frame's children with it; a selection with a picture gets Excalidraw's refusal as a
  toast) and "Insert from shape library…" (a picker of the app over the held library: each
  item by its name, or "Shape 3: rectangle, arrow, text" without one, newest first;
  `enter` puts a copy at the middle of the view, selected, through the `library` call). No
  default keys; palette, Insert > Whiteboard, the title menu.
- **Not in this version**: importing or exporting an `.excalidrawlib` file, libraries
  from libraries.excalidraw.com, a library per whiteboard, agents reading or changing it.

---

## 5. PDF, Print, Export

### The one code path

The app's Export as PDF, its Print, a folder's PDF export (one page per whiteboard, below),
the scripted `board:pdf:` and `print:pdf:`, and the tool's `innerword export board:12 x.pdf`
all make the PDF the same way, from the **stored**
scene (never from the live canvas, so the zoom, the theme and a selection change nothing):

1. **The source**: the bundle (`Assets`), `scene.json` (the stored scene), `_files/<FileId>`
   (its pictures' bytes). The app builds it over the library (`LibraryBoardSource`, the same
   type as the view's); the tool's child builds it over a directory that `innerword-agent`
   wrote (`innerword_board::export_dir`: `scene.json` and `_files/<FileId>`).
2. **The export page**: `innerword_web::run_script` (the app, in-process, on the main thread,
   started from the export's task between two awaits as `print_offscreen` is) or
   `run_script_blocking` (the child) loads `export.html` offscreen under
   `board_export_policy()` (4.2) and calls
   `calls::EXPORT_LAYOUT` with `{"padding": 32, "maxSide": 14400}`. The glue: fetches the
   scene and its pictures; loads the fonts the scene uses; `restore` with
   `refreshDimensions` and `repairBindings` (the text measured in the real fonts, whatever
   Rust wrote); `exportToSvg({ elements, files, appState: { exportBackground: true,
   viewBackgroundColor: <the scene's>, exportWithDarkMode: false, exportScale }, exportPadding:
   32 })`, which inlines the subsets of the fonts it uses as `data:` WOFF2 and makes no
   request; replaces the page's body with the SVG alone at (0, 0); answers its width and
   height. `exportScale` is 1 (Excalidraw's default is the display's pixel ratio, which made
   a Retina PDF twice the drawing's size), or less when the longer side with the margin
   would pass 14,400: then `14400 / that side`, the whole drawing scaled down (Acrobat's
   limit; WebKit made a 22,010 pt page without complaint).
   **Built (follow-up of the acceptance run): the SVG is mended where it differs from the
   canvas** (`asTheCanvasDraws` in `export.js`), before it is shown:
   - *A frame clips its children.* Excalidraw's canvas cuts a frame's children at the
     frame's edges (`frameRendering.clip`, on by default and in the restored `appState`),
     and its `exportToSvg` wraps each child in a group with the frame's `clipPath`, but sets
     the attribute with `setAttributeNS(SVG_NS, "clip-path", …)`: an attribute in the SVG
     namespace, which no renderer reads (presentation attributes are in none), so WebKit,
     like every browser, drew the child whole. 9.4's run moved an ellipse half out of its
     frame: the screen cut it, the PDF did not, and its page cut the ellipse off at the
     bottom instead. The glue sets each such attribute again without a namespace.
   - *The room around an arrow's label.* The canvas clears the label's box and
     `BOUND_TEXT_PADDING` (5 px) around it from the arrow's line; the SVG masks the line
     with a black rectangle of the label's box alone, so the line ran up to the letters
     ("—reads—>"). WebKit's `createPDF` draws the mask as it is (a mask is a soft clip in
     the PDF: the line is cut, not covered); the glue makes each hole 5 larger on every
     side, which is the canvas's gap.
   **The page's size is one rule in Rust and JS.** Excalidraw sizes the SVG with
   `getCanvasSize(getRootElements(elements))`: the frames, the elements in no frame (a
   `frameId` that names no frame of the scene is none), and the name Excalidraw adds above
   each frame for the export (Helvetica 14 at line height 1.25, one line per line of the
   name, "Frame" without one, ending 3 above the frame, never wider than the frame); a
   frame's children add nothing, since what of them is outside the frame is not drawn.
   `Scene::pdf_bounds` is that rule (`innerword-board/src/frames.rs`), and `Scene::pdf_size`
   measures it (before, Rust took every element's bounds and no name: 9.4's page was
   expected 734 x 224 and made 734 x 215). `Scene::bounds`, the drawn area of the listing
   and the summary, still holds every element. The WebKit case `board_pdf`
   (`tests/webkit/bundle.rs`, `the_pdf_is_the_screen`) exports 9.4's frame with the
   ellipse moved half out and holds the page at `pdf_size` (544 x 185), every child under a
   clip a renderer reads, no ink below the frame, and no ink in the 5 pt around the label
   with the line just beside it (three of its checks fail against the export of before).
3. **The PDF**: `createPDFWithConfiguration:` with the rect `(0, 0, ceil(w), ceil(h))`, after
   `document.fonts.ready` and two frames. One page, exactly the drawing and a 32 pt margin
   (11 mm), lines as vector paths, text as text in the embedded hand-drawn fonts (PDFKit read
   it back in the scout's run: `Pipeline | Draft chapter | sends | Review | Free text…`),
   pictures as the files they are. Measured: 3 ms for the PDF of a small drawing, 41 ms for
   3,000 elements; the SVG 113 to 135 ms the first time (the font subsetter loads), 54 ms
   after; the load of the page about 200 ms. As built (release): the PDF of a whiteboard of
   3,000 elements (5,250 with labels) 1.37 s from the stored scene; a small one 90 to
   200 ms in the WebKit program; the tool's `innerword export board:12 x.pdf` of the 9.4
   drawing 146 KB in 0.47 s.

A dark PDF is never made: Excalidraw's dark export inverts with a CSS filter, and WebKit then
puts a picture of the page in the PDF, without text.

**An empty whiteboard** (no element that is not deleted) has no PDF: the app says
"“Plan” is empty: there is nothing to export." (or "… to print."), the tool answers exit 9
(`wrong_state`) with the same words. Nothing loads WebKit for it: the emptiness is read in
Rust (`Scene::is_empty`).

### In the app

- **Export as PDF…** on a whiteboard (the command, File > Export as PDF…, the title menu,
  the picker's row): the save panel suggests "<title>.pdf" first (a cancelled panel makes
  nothing), then `collect` and a save, then the PDF as above, then the bytes written by a
  background thread and the toast that says where. It runs as the existing PDF export's job
  (`print.stage` is `(Kind::Export, Preparing)`, `print.last_export` is set), so "one job of
  pages at a time", the note "Preparing the PDF of “…”…" after 0.4 s, Cancel PDF export and
  the scripted `export:wait` work as for a document. (Today a page's PDF export does not go
  through `print.stage`: the whiteboard's does.) **Built:** the file is WebKit's bytes
  unchanged (so an element's link stays a link annotation); only a folder's PDF draws
  whiteboards in with `add_pdf_page`. `export:wait` reports it as "1 page, 1 whiteboard, …".
- **The export picker** (`cmd-shift-s`, File > Export…, the title menu, the row's menu) on a
  whiteboard has **one row, "PDF file"**, under the heading "“<title>”": the same picker
  everywhere, saying what a whiteboard offers. No "Copy as Markdown" or "Copy as plain text"
  rows.
- **Print…** on a whiteboard (File > Print…, the title menu, the palette; no key by default,
  as for a document): `collect`, save, the PDF as above, then a `printing::PrintJob { title,
  paper: <the PDF page's size>, pdf, fit: Fit::ShrinkOntoPaper }` handed to the installed
  `PrintTarget` (the print panel as a sheet, a test's `Recorder`, a scripted run's
  `Directory`). On the panel the drawing is put on **the user's paper**, landscape when it is
  wider than high, and scaled down when larger than the paper (never up). That needs one
  change in `innerword-print`: `PrintRequest { pdf, paper, scaling, title }` gets `paper:
  Option<(f32, f32)>` (`None`: the user's paper, read from a copy of
  `NSPrintInfo.sharedPrintInfo`, `-[NSPrintInfo paperSize]` joining `check_selectors`, 21
  entries) and `scaling: Scaling` (`Actual`, today's, `kPDFPrintPageScaleNone`, or
  `DownToFit`: `kPDFPrintPageScaleDownToFit`); with `DownToFit` the paper, given or the
  user's, is turned to the orientation of the PDF's first page (a square one counts as
  portrait). Margins stay 0 and centring on. The save job's case: a drawing of 1,600 × 900 pt
  with `Some(A4)` and `DownToFit` came out as one A4 sheet turned landscape, its text still
  text, scaled by 0.508. `printing::Fit` is the app's word for the
  two (`Fit::Actual`, a text's job: `paper: Some(job.paper)`, `Scaling::Actual`, as today;
  `Fit::ShrinkOntoPaper`, a whiteboard's: `paper: None`, `Scaling::DownToFit`), mapped in
  `PrintJob::request`. A test that must not depend on the Mac's default printer passes
  `Some(A4)` with `DownToFit` (`innerword-print`'s save job, and the app's recorder, which
  records the `Fit` and never asks `NSPrintInfo`). The scripted `print:job:` runs the same
  through `run_save_job`, with A4.
- **A folder's PDF holds its whiteboards** (answer 2): the picker's "PDF file" for a
  folder, and Export as PDF… with a folder chosen.
  - **What is in it**: the documents first, exactly as today (each on its own paper, the
    page numbers going on through the file); then **one page per whiteboard**, in the
    order the folder export walks the tree, each the whiteboard's own PDF page of its own
    size (the drawing's bounds and the 32 pt margin, scaled down past 14,400 pt as on its
    own). No title is set above a whiteboard and no page number is drawn on its page (the
    page is the drawing and its margin, as its own PDF is); it counts among the file's
    pages. The pages of the folder (web pages) are still left out, and the toast says so.
  - **How the pages are joined**: the whiteboard's PDF is made by WebKit as for Export as
    PDF… (the export page, `createPDFWithConfiguration:`), and the folder's PDF is
    `innerword-print`'s `PdfWriter`, which gains `add_pdf_page(pdf_bytes, page) ->
    Option<(f32, f32)>` (`page` from 1; `None`, adding nothing, for bytes that are no PDF,
    page 0, a page past the end or a page of no size): a page of **the size a reader shows
    for that page** (its crop box, turned by `/Rotate 90` or `270`, drawn through
    `CGPDFPageGetDrawingTransform`; built: the spec said its media box, which is the same for
    WebKit's one-box PDFs and wrong for a cropped or turned page), with the page drawn into
    it by `CGContextDrawPDFPage`, unscaled. Quartz writes the drawn page's content again into
    the new file, so its lines stay paths, its text stays text in its embedded font subsets
    and its pictures stay the files they are (B5's unit tests in `src/tests.rs`, not a
    `tests/` binary, which the crate does not have but for `save_job`: text, size, each word
    within 0.05 pt, pixels). **Its link annotations are not kept** (`CGContextDrawPDFPage`
    copies the content stream only): an element's link is a link in the whiteboard's own
    PDF and plain drawing in a folder's. An added page counts in `Report::pages` and in
    `Report::embedded_pages`, and `Report::line` names it right after the pages ("14 pages,
    2 whiteboards, …"); `rows`, `fonts` and `images` count the writer's own pages only. Not taken: joining PDFs with PDFKit
    (`PDFDocument.insertPage`), which would need the documents' PDF finished first and a
    second copy of it in memory, and is AppKit's where `PdfWriter` is CoreGraphics on the
    job's own context.
  - **Where WebKit runs**: in the export's job of pages (`workspace/print.rs`, purpose a
    file), on the main thread, started from the job's task between two awaits, one
    whiteboard at a time, after the documents' slices: `innerword_web::run_script` over a
    `LibraryBoardSource` of that whiteboard, as Export as PDF… of one whiteboard runs it.
    `gather_export` takes each whiteboard's **stored scene when the export is chosen**
    (after `collect` and a save when the open whiteboard is in the folder, 4.6), so what is
    exported is what was there at that moment, as for the documents. The PDF writer is not
    `Send`, so `add_pdf_page` runs on the main thread too, a few milliseconds a page.
    `export=[preparing N%]` counts each whiteboard as a step; Cancel PDF export works
    between two whiteboards (a `run_script` in flight finishes and its PDF is dropped).
  - **An empty whiteboard** is left out, and the toast says so: "Exported “Part one” to
    ~/Desktop/Part one.pdf: 12 pages, 2 whiteboards. “Plan” is empty and was left out."
    (`export:wait` names the whiteboards too: `… 12 pages, 2 whiteboards, …`).
  - **A whiteboard whose PDF cannot be made** (WebKit fails or passes its time limit, a
    picture the store cannot read) ends the export with nothing written, as a picture of a
    document does: "Could not export: the PDF of the whiteboard “Plan” could not be made
    (<the reason>), so nothing was written."
  - **Without a page engine** (macOS below 12.3, `INNER_WORD_PAGES=off`) the whiteboards
    are left out and the toast says why ("2 whiteboards were left out: whiteboards need
    macOS 12.3 or later."). On the fake engine (tests) each is the fixed one-page PDF of
    `Scene::pdf_size`, joined the same way.
- **The folder's Markdown, DOCX and plain-text exports leave whiteboards out**, as they
  leave pages out, in the app (`gather_export`, `export::left_out_sentence`) and for agents
  (`innerword-agent`'s `export.rs`, whose folder export has no PDF). The toast counts both:
  "1 whiteboard was left out: a whiteboard is exported as a PDF, on its own or in the
  folder's PDF." / "2 pages and 1 whiteboard were left out: …". **Built:** the agent's
  warning says "1 whiteboard was left out: a whiteboard is exported on its own, as a
  PDF.", since agents have no folder PDF; a folder that holds nothing but pages and
  whiteboards is refused ("nothing to export as one file").
- On the fake engine (tests) the PDF is a fixed one-page PDF made by `innerword-print`, of
  the size Rust estimates (`Scene::pdf_size(32.0, 14400.0)`, the bounds of the elements and
  the margin), so a test can hold the page against the drawing.

### For agents

`innerword export board:12 <file.pdf> [--overwrite]` and the MCP tool `export` with
`format: "pdf"`: the tool reads the stored scene (after asking an open app to save, as every
read does), refuses an empty one (exit 9), writes the directory (`innerword_board::export_dir::write`:
`scene.json` and each picture's bytes under `_files/`) into a `WorkDir` of its own, and
runs `innerword __board-pdf <dir> <out.pdf>` through the installed checker
(`PageChecker::print_board`, a required method; `ProcessChecker`: standard input closed, the
child killed at 60 s, `BOARD_PDF_KILL_MS`, or the checker's own limit when lower, the last
line of its standard error as the reason; the child gives WebKit 50 s). The child asks
`innerword_web::available()`, finds the bundle (`Assets::find`), builds the source and calls
`run_script_blocking` with `EXPORT_ENTRY`, `EXPORT_LAYOUT`, `export_argument()`,
`board_export_policy()` and `pdf: true`, then writes the bytes (`BoardDirSource` serves
`scene.json`, the `_files/<32 hex>` pictures typed from their bytes, and the bundle;
`crates/innerword-cli/src/board_pdf.rs`, handled before the command line is parsed). A
failure is one line on standard error and exit 1. WebKit never runs in the MCP server's process.
`INNER_WORD_PAGE_CHECK=fake` in a debug build answers in the process with the fixed PDF, as
for a page's PDF. Any other format of a whiteboard (`.md`, `.docx`, `.txt`, `html`, a
folder) is exit 15: "“Plan” is a whiteboard, and a whiteboard is exported as a PDF only."

---

## 6. The app

### 6.1 The shape: an open web item

`Workspace::page: Option<page::PageSession>` becomes the session of the open **web item**,
with a kind; built: `PageSession` kept its name and gained `item: WebItem { Page(PageItem),
Board(Box<BoardState>) }`. The shape:

```rust
pub(crate) struct PageSession {            // or WebSession
    pub id: DocumentId,
    pub title: String,
    pub view: Option<pages::PageViewHandle>,
    pub state: PageState,                  // Loading | Loaded | Failed | Crashed { .. }
    pub overlay: Overlay,
    pub focus: FocusHandle,
    pub inbox: SharedInbox,
    pub item: WebItem,
}
pub(crate) enum WebItem {
    Page(PageItem),     // manifest, source_url, words, zoom (page zoom), find strip: what PageSession has today
    Board(BoardState),  // section 4.6: held scene, saved rev, status, pictures, the save in flight, failure
}
```

Every `self.page` in the app that means "a WebKit item is open" stays (hosting, focus,
overlays, holes, the keys' snapshot, the inbox task, crashes, `open_item_id`); every one
that means "a page" branches on the kind. The scouts' table of those places (scout-app,
section 0b: about sixty, in `session.rs`, `page.rs`, `mod.rs`, `render.rs`, `zoom.rs`,
`menus.rs`, `assistant.rs`, `right_panel.rs`, `live.rs`, `print.rs`, `page_import.rs`,
`export.rs`, `pdf_export.rs`, `version_history.rs`, `quick_open.rs`, `library_search.rs`,
`automation.rs`, `shortcuts.rs`, `keymap.rs`) is B8's checklist; **its first commit turns
every `kind == DocumentKind::Page` / `!= Page` / `is_page` of the app into an exhaustive
`match`**, so that the compiler lists each place a whiteboard has to decide. The
whiteboard's own code goes into `workspace/board.rs` (the session's board half, the calls,
the autosave, the commands) and `boards.rs` (the source, the bridge's events, the fake).
`open_document(id)` routes `Board` to `open_board(id)` as it routes `Page` to `open_page`.
`restore_session` reopens a whiteboard that was the last open item.

### 6.2 Commands and menus

New actions:

| Action | Title (palette, list) | Category / place | Default key | Where else |
|---|---|---|---|---|
| `workspace::NewBoard` | New whiteboard | File, "Anywhere" | none | File > New Whiteboard, the empty state's hint; only with a page engine |
| `library_panel::NewBoard` | New whiteboard | "In the library" | none | the library's menu and + menu ("New Whiteboard", after New Folder), a folder row ("New Whiteboard Here"); only when the app gave the panel the content of a new whiteboard (`set_new_board_content`, which it does with a page engine; `LibraryPanel::makes_boards`). **Built:** it is not in the panel's `commands()` (every command there has a key in the panel, and the palette has `workspace::NewBoard` already); its row comes from `innerword_library::new_board_command()`, which the app registers as a row of the list that the palette always leaves out, in its place through `HOME_CONTEXTS` (`library_panel` → `innerword_library::TREE_KEY_CONTEXT`, where `n` and `shift-n` are bound): without that, a key the user gave it would have applied anywhere |
| `board::OpenContextMenu` | Show the whiteboard's context menu | "On a whiteboard" | `ctrl-enter`, `shift-f10` | |
| `editor_tools::Find` in `BoardView` | Find | "On a whiteboard" | `cmd-f` | Edit > Find, the title menu ("Find on Whiteboard…") |
| `board::AddText`, `AddRectangle`, `AddEllipse`, `AddDiamond`, `AddArrow`, `AddFrame` | Add text, Add rectangle, … (palette category "Whiteboard", `actions::CATEGORY_WHITEBOARD`, keywords: draw, shape, new element) | "On a whiteboard" | none | Insert > Whiteboard > Text, Rectangle, …; the title menu's Add; the `ctrl-enter` menu. Each asks the glue for `center` (the middle of the view and the selection), converts one skeleton with `innerword_board::add` against the held scene (`HashedMaking` seeded by the held scene's stamp, the rev and the moment, so ids do not collide), and calls `insertElements` with it selected (a text: opened for typing by `HOLD_KEYS` and `OPEN_TEXT`, 4.5). **Built:** when the status says the page holds more, it collects first (the held copy of a shape the writer moved would otherwise replace the canvas's through a new arrow's binding); an arrow between two selected shapes is given `x` and `y` too (the converter needs them; without them Add Arrow between two shapes never worked). With the canvas not loaded yet they say "The whiteboard is still loading.", on a whiteboard that is not drawn "This whiteboard could not be shown, so nothing can be added to it." |
| `board::MermaidToDrawing` | Mermaid to drawing… (keywords: mermaid, flowchart, diagram, sequence) | "On a whiteboard" | none | Insert > Whiteboard > From Mermaid…, the title menu's Add; the `mermaid` call (4.13) |
| `board::ToggleShapeLibrary`, `board::AddToShapeLibrary`, `board::InsertFromShapeLibrary` | Show or hide the shape library, Add selection to shape library, Insert from shape library… (keywords: library, shapes, reuse, stencil) | "On a whiteboard" | none | Insert > Whiteboard > Shape Library, Add Selection to Shape Library, From Shape Library…; the title menu; the `library` call; the last opens the app's picker of the held library (4.15) |

Existing commands with a whiteboard's handler: Save Now, Rename (palette "Rename
whiteboard" while one is open, the old title among the keywords, as "Rename page"), Go to
text ("Go to whiteboard"), Reveal in Library, Version History, Export…, Export as PDF…,
Cancel PDF export, Print…, Cancel printing, Zoom In, Zoom Out, Actual Size, Fit Width, Fit
Page, Zoom to 50 % … 400 %, Image (Insert > Image…), Undo, Redo, Cut, Copy, Paste, Select
All (the Edit menu), Reload page ("Reload whiteboard", 4.6), Duplicate (collects first),
Go to assistant, Show or hide assistant, New Chat, Clear Chat.

### 6.3 Commands that do not apply

The text's commands (`text_commands()`, the image commands but Image, spelling's word
commands) have no handler on a whiteboard: greyed in the menu bar, left out of the palette,
as on a page. These say what a whiteboard is, in one toast, from their keys and the menu
bar, and are left out of the palette (`commands_not_for_the_open_item`, the page's list
made per kind):

| Command | Toast |
|---|---|
| Document settings, page view on or off, orientation, body text size, line spacing | "A whiteboard has no document settings." |
| Take title from text | "A whiteboard keeps the title it was given: rename it to change it." |
| Spelling language | "A whiteboard is not checked for spelling." |
| Add comment, go to comments (`cmd-2`); Show or hide comments, Next comment, Previous comment (from the menu bar only: their keys are Excalidraw's here, 4.7) | "A whiteboard has no comments." |
| Vim mode on or off (from the menu bar only) | "Vim mode is for texts; a whiteboard has its own keys." |
| Ask the assistant (quick chat, `cmd-j`) | "The quick chat works in a text. Ask about this whiteboard in the side chat." (with the side chat's key from the keymap); without the assistant `cmd-j` is left alone (final review) |
| Zoom to life size, Zoom to automatic | "A whiteboard has no paper size: Zoom to Fit shows the whole drawing." |

The four with app-wide listeners (comments, Vim mode) answer a click on their menu item
with the toast, captured at the workspace root and stopped there; their keys never reach
them on a whiteboard, because the fixed `NoAction` bindings keep GPUI off the chords and
the menu bar is built without those key equivalents while a whiteboard is open (4.7).
Find next, Find previous and Find and replace have **no handler** on a whiteboard (greyed,
left out of the palette), so `cmd-g` and `cmd-shift-g` are Excalidraw's group and ungroup.

### 6.4 The title and the status bar

As section 1 says. `display_title`, the window's title and the side chat's title take the
whiteboard's title. The elements item and the zoom item read the last `status`; the save
state item reads `BoardState` (dirty, saving, failure). Built: the elements item counts as
the listing does (`board_elements_label`, from the base scene when there is one), and the
labels are "Saved", "Saving…", "Edited", "Not saved: …" (`board_save_label`).

### 6.5 The version history

`toggle_version_history` takes `open_item_id` as it does for a page; for a whiteboard it
first collects and saves. A version's preview is its listing of elements
(`Scene::from_stored(&snapshot.content)?.listing()`), muted, as a page's text is. Restore:
`collect` and a save first, the current scene kept as a version (`snapshot_if_stale(id,
ZERO)`), the old scene saved (`Library::save_board` with `expect: Some(base.stamp)`; a
restore is an outside write in the sense of 4.6, so the view's posts are held from here
until the adopt answers), and when the whiteboard is open, adopted into the view as one
step of its undo (`adopt` with `restored.delta_from(&current)`, base versions from the
current scene; the writer can `cmd-z` it), the picture map seeded from the restored scene,
with the toast restoring a document gives. A restore by an agent (`restore_version` on the
open whiteboard) goes through 6.7 like any agent's change.

### 6.6 The assistant

- **The side chat** over a whiteboard is sent, with every message, the listing of its
  elements as the app holds it (the held scene, after `collect` when the status says the
  page holds more, waited for at most 1 s) in place of the Markdown. **Built:** the wait is
  `AssistantPanel::set_before_send` (`innerword-assistant-ui`, added for this: a task of the
  host's that a message waits for before the open document is read; more messages wait
  behind it in order; Stop and another document give the text back to the composer), which
  the app fills with `Workspace::collect_for_a_message` (`VIEW_COLLECT_PATIENCE`, then the
  message goes with what the app holds); the side chat's source holds a `BoardFeed`, the
  scene the app holds, kept current per frame and per post; and the keyboard coming into
  the side chat over a whiteboard by any way asks the page too
  (`collect_as_the_side_chat_takes_the_keyboard`). B10 had first collected only on `cmd-3`,
  then on every way in; the hook came with the final round. After one sentence
  (`innerword_assistant::BOARD_PREFACE`: "The open item is a whiteboard: a drawing of shapes,
  arrows, text and pictures. It is changed only by its owner, in the app. Its elements, one
  per line, with their ids, positions, sizes and text:"), with `kind="board"` in the tag
  (`OpenDocument::kind`, `OpenKind::Board`). Too long a listing goes as its beginning and its
  end, as a page's text does. The system prompt says what such an item is, and that the
  assistant cannot change it. Insert at Caret and Replace Selection are off over a
  whiteboard (no caret, no selection), Copy works; the conversation is the whiteboard's own
  (`assistant.conversation.<id>`). The assistant reads other whiteboards with the same
  read-only tools (`read_document` gives the listing).
- **The quick chat** does not open on a whiteboard (the toast of 6.3).
- **Built later: the side chat proposes drawings** (`docs/assistant-draws-design.md`): an
  answer may hold a drawing in the agents' formats, which the writer applies with a button,
  `cmd-shift-d` or the palette, on the open whiteboard as one step of Excalidraw's undo
  (the path of 6.5 and 6.7: collect, flush, the drawing on the stored scene, a version, the
  save compare-and-swap, `adopt`), or as a new whiteboard beside the open document. The
  assistant still changes nothing itself and keeps the read-only tools.

### 6.7 An agent's change to the open whiteboard

Any request of an agent (one that changes the library, and a read with its `flush`) while a
whiteboard is open or closing:

1. **Wait for what the page has not posted** (`Workspace::must_wait`, which becomes `&mut
   self`). Today it is `request.changes_library() && self.session.is_some() &&
   is_composing()`, which keeps reads out and is false on a whiteboard, where `session` is
   `None` (reviewer's point, taken). The new condition, in this order:
   - a whiteboard is in `closing_boards`: wait until its last save is done (at most
     `VIEW_COLLECT_PATIENCE`, 1 s, then it is saved as the app holds it), for every request,
     so a late collect never saves the pre-agent scene over the agent's change;
   - the open whiteboard's last `status` says the page holds more (`rev > posted`, the
     pointer down, a text being edited): the workspace calls `collect` once for this
     request and `serve` waits (`Turn::Wait`, polling every `COMPOSITION_POLL`) until its
     answer has been saved, **at most 1 s**; then the request goes on with what the app
     holds, a read and a write alike. That is safe for a write: whatever the page still
     holds is newer than the base the agent's change is merged against, and the base rule
     keeps it (4.5);
   - a request that changes the library while the glue says `composing` (an input method in
     a text element): it waits, and after `COMPOSITION_PATIENCE` (3 s) it is refused with
     `Refusal::Composing`, "nothing was changed", as for a text. A read does not wait for a
     composition. No new refusal.
   - **Built**, a fourth wait: an earlier outside write whose `adopt` has not answered yet,
     at most `ADOPT_PATIENCE` plus `VIEW_COLLECT_PATIENCE` (without it, two quick changes to
     one element were judged against an old `baseRev`, and the second could be taken for the
     writer's conflict and lost). Each wait has its own patience on the app's clock;
     `must_wait` gets the request's first moment from the serve loop (`first`).
2. **Carry out** (`carry_out`, as today): the flush saves the scene the page handed over, so
   the version the agent's operation keeps is what the writer had; the app notes the stored
   scene just before (`S1`, which is `base`); `live::execute` changes the stored
   whiteboard; from that moment the view's posts are held (4.6, "After an outside write").
3. **Take it over** (`take_over_open_board`, beside `take_over_open_document`): the stored
   scene after (`S2`), the source's cell takes it, the picture map is seeded from it, the
   delta `S2.delta_from(&S1)` (elements added or changed, each with its version in S1 as
   `baseVersion`; elements gone, by id with their S1 version; new pictures), and `adopt`
   with it, by the base rule (4.5). When `adopt` answers, `base` is S2 and `adopting` is
   the rev it answered; when it answers `same`, that rev counts as saved (the library has
   it already); otherwise the page posts the merge as a change of the writer's, which is
   saved on S2. When it fails, 4.6 says what happens (the merge in Rust, a reload). A
   trashed or deleted whiteboard closes as a page does. The keyboard and an open dialog
   stay where they are. A rename changes the title shown and nothing in the view.
4. **The note** (`workspace::AgentChanges`, `workspace::sentence`): "Claude made the
   whiteboard “Plan”" (`create_board`, Open), "Claude drew on “Plan”" (`add_to_board`),
   "Claude changed “Plan”" (`update_board`), "Claude removed elements from “Plan”"
   (`remove_from_board`); several in a row join as for a text ("Claude made 20 changes to
   “Plan”"). **When `adopt` answered conflicts**, the note says so after the sentence: "2
   of its changes were not taken, because you changed those elements meanwhile: api,
   reads." (at most five ids, then "and 3 more"); the library then holds the writer's
   copies of those elements after the next save, and the version the agent's change kept
   has what was there before. The agent's answer cannot say this (it goes back in the turn
   of the change, before the view has merged), which is why the note does. Its button is
   **Undo** while the whiteboard is open and its rev is still the one the last adopt
   answered (the writer has not drawn since): `undo` with as many steps as the run adopted
   (`Undo::Board { board, steps, rev }`, filled when the view answers;
   `agent_adopt_answered`, withdrawn by `agent_adopt_failed` and `board_moved_on`).
   **Built after review:** Undo is offered only when every change of the run is one step of
   that whiteboard's undo (`steps_before + 1 == count`); a run that began with a trash, a
   change elsewhere, or adopts on both sides of a drawing of the writer's has no Undo (it
   would have promised to take out more than it does). For a whiteboard that is not open,
   **Open**.
5. Cost: the delta is computed on the main thread from two parsed scenes. Measured (release,
   a whiteboard of 3,000 elements, 5,250 with labels, 3.0 MB): `cost=[save 16, change 81,
   window 41 ms]`, the tool 169 ms from start to answer; with a 16 MB picture, change
   76 ms, window 38 ms, the tool 401 ms (the listener stores the picture before the
   request is queued); the 9.4 drawing on a small whiteboard: change 1.4 ms (in CLAUDE.md
   with the agents' other costs).

A whiteboard changed by an agent while the app was closed opens with the stored scene, fitted
to its drawing (its `view_state`'s stamp is not the scene's).

### 6.8 The list of keyboard shortcuts

- `keymap::places`: `.typing(BOARD_VIEW)`; `shortcuts::Place::of_predicate`: `BoardView` is
  "On a whiteboard"; `KEY_GROUPS` gets the `board` namespace (its type
  `[(&str, &str); 20]` becomes 21, `shortcuts.rs`), `KEY_TITLES`
  `board::OpenContextMenu`. New rows: New whiteboard (Anywhere, and In the library),
  Show the whiteboard's context menu, Find, the six Add commands, Mermaid to drawing and
  the three shape library commands (On a whiteboard).
- **The whiteboard's own keys are rows too, marked "Fixed"**, as the insert menu's `/` is
  today (reviewer's point, taken: a note alone would leave the list unable to answer "what
  does ⌘D do on a whiteboard"): one row per entry of `WHITEBOARD_KEYS`, titled with its
  `what` ("Duplicate", "Group", …), place "On a whiteboard", and one row "Tools (letters
  and digits)" for the tool keys. **Built:** the fixed rows are grouped by what a key does,
  so the 43 keys make 42 rows (Clear canvas has two keys) and the tools row 43; a fixed
  row says `BOARD_KEY_NOTE`, the tools row `TOOL_KEYS_NOTE` (`shortcuts::Place::Board`).
  **A key given up** to a command of the user's (4.7, `keymap::whiteboard_keys`) is not
  shown on its row any more, and the row says who has it,
  `shortcuts::board_key_given_note`: "On a whiteboard ⌘K runs “Command palette”: you gave
  it that key. Excalidraw's “Link of the selected element” is reached there from its own
  menus and toolbar." (the command named as its row names it). Reset, the row is as
  before.
  They are rows of the list only: the fixed `NoAction`
  bindings are not commands, not entries of the shortcuts file, and the code that collects
  entries skips `gpui::NoAction` (the test
  `every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file` keeps holding, with the
  fixed rows counted apart, as `/` is). The list's search finds them by name and by key.
- **The counts move, and two tests pin them**: `tests/suite/pages.rs` asserts 355 rows and
  410 entries without the assistant (lines 1352 and 1372), CLAUDE.md says 385 and 410 with
  every part shown. B8 counts in a scripted run (`keys:cmd-/; state`, and "Export…") and
  in the test, and changes both numbers in `pages.rs` and in CLAUDE.md (through B10) with
  a sentence that says what they hold: today's rows, the two New whiteboard rows, the
  whiteboard's menu, Find, six Add commands, Mermaid to drawing, the three shape library
  commands, the fixed rows, Reload whiteboard being the same command as Reload page;
  entries grow by the commands only. **As built: 442 rows and 424 entries with every part
  shown, 412 rows and 424 entries without the assistant** (`pages.rs` asserts the second
  pair): whiteboards add 57 rows (the two New whiteboard rows, the menu, Find, the six Add
  commands, Mermaid to drawing, the three shape library commands, 42 fixed rows and the
  tools row) and 14 entries (the 14 commands of those rows). B8 had first counted 411 and
  423, before the library's New whiteboard became a row (6.2).
- The question about a key. **As first built**: `Listing::sees_keys_first("the whiteboard",
  [BOARD_VIEW], board_leaves, BOARD_TAKES_KEYS_FIRST)`, the heading "⌘D: the whiteboard
  sees it first" and "On a whiteboard the whiteboard uses this key itself, so there the
  shortcut does not run." **Since a key given to a command is that command's there (4.7)**
  that was no longer true, and the list says what is given up instead:
  `Listing::gives_keys_up(BOARD_NAME, [Workspace, BoardView], <every key of
  WHITEBOARD_KEYS with its what>, BOARD_GIVES_KEY_UP)` (`innerword-keymap-ui`:
  `GivesKeysUp`, `Listing::giving_up`; the view fills `{key}`, `{what}` and `{command}`). A
  key of `WHITEBOARD_KEYS` given to a command that works anywhere or in `BoardView` gets the
  sentence "On a whiteboard ⌘K is Excalidraw's “Link of the selected element”. Given to
  “Command palette”, it runs that command there, and Excalidraw's “Link of the selected
  element” is reached from its own menus and toolbar.", a warning, so the marked choice is
  "Press Another Key"; its heading, where nothing more important is said (taken, macOS),
  is "⌘D is also the whiteboard's own key" (`words.rs`: in the slot of "sees it first";
  never "free" above it). For ⌘K the heading is "⌘K is taken" (the text's "Add or edit
  link" around the text has it) with the whiteboard's sentence under it.
- `ALSO_FIXED_NOTE` becomes "Also fixed: the keys inside a right-click menu, ⇥ in the insert
  menu, clicks with ⌘, ⇧ or ⌃, and a whiteboard's own keys (listed On a whiteboard)".
- **That none of `WHITEBOARD_KEYS` is a key the window keeps by default** is tested in the
  app (`boards.rs`, `none_of_the_whiteboards_keys_is_a_key_the_window_keeps`, against the
  installed keymap), not in `innerword-board-web`, which cannot see the keymap (reviewer's
  point, taken). A user's change that gives a command one of those keys:
  `boards.rs`, `a_whiteboard_key_the_user_gave_a_command_runs_it_there` (⌘K and the two
  spellings of ⇧⌘. run the palette on a whiteboard, from GPUI's view and handed on by the
  web view; the menu bar; the row; reset brings the hold back) and
  `a_whiteboard_key_a_default_wins_stays_held`; `keybindings.rs`,
  `a_whiteboard_key_given_to_a_command_is_said_and_given_up` (the question, the change, an
  import of the file).

### 6.9 Scripted runs

| Step | What it does |
|---|---|
| `board:new[:<title>]` | makes a whiteboard as New Whiteboard does, named `<title>` without the sidebar's field, opens it; logs `board: made board:12 “Plan”` |
| `board:wait[:<ms>]` | waits until the open whiteboard's page posted `ready` (fails after `ms`, 10 s unless given); logs `board: ready after 412 ms (loaded in 380 ms)` (built: the load time, `BoardState::ready_in`) |
| `board:scene` | logs the listing of the open whiteboard as the app holds it, after `collect` |
| `board:draw:<json>` | converts skeletons with `innerword_board` against the held scene and gives them to the page as the writer's drawing (`insertElements`), so they go through `onChange` and the autosave; logs the ids |
| `board:press:<keys>` | sends real `keyDown:`/`keyUp:` events to the web view (as AppKit would hand them to the first responder; GPUI's `keys:` never reaches WebKit), for what reaches Excalidraw; not the menu bar's half |
| `board:js:<script>` | evaluates a script in the page and logs the answer (scripted runs only) |
| `board:pdf:<path>` | writes the whiteboard's PDF at once and logs `board: pdf 1 page, 1034 x 478 pt, 2 pictures, made in 0.31 s` |
| `board:menu` | opens the whiteboard's menu, as `ctrl-enter` |
| `board:library` | logs the held shape library: `board: library 3 items, saved` (`1 item` in the singular) |
| `board:mermaid:<definition>` | converts a Mermaid definition (`\n` for a new line) as the dialog's Insert does (the `mermaid` call with `insert`) and logs the ids |

`state` gets `board=[id=12 state=loaded elements=14 rev=31 posted=31 saved=31 dirty=no
base=3f2a9c1b adopting=no editing=no composing=no pointer=up zoom=100% pictures=2/2
recovered=no library=3/saved overlay=none focus=board responder=web closing=no]` while a
whiteboard is open, and `board=[none]` otherwise. The Add commands are `action:board::AddRectangle` and
the others.
`print:pdf:`, `print:job:`, `export:wait`, `zoom:in|out|actual|fit|125`, `agents:<n>`,
`theme:` and `screenshot:` work on a whiteboard. Do not script `cmd-c`/`cmd-x` on a
whiteboard (`board:press:cmd-c` writes the user's clipboard).

### 6.10 The fake engine, and the app's tests

`pages::FakePages` learns whiteboards; `FakeView` knows whether it is one. A test never makes
a `WKWebView` and never loads the bundle. The fake's canvas is a scene held in Rust; its
`adopt` applies the base rule with `innerword_board::merge`, so the app's tests exercise
the rule the glue implements, and B4's WebKit case holds the glue to the same table.

```rust
impl FakePages {
    pub fn boards_opened(&self) -> Vec<(DocumentId, String)>;     // (board, the board.json it was served)
    pub fn board_ready(&self);                                    // the open board posts "ready" with the stamp it loaded
    pub fn draw(&self, scene_json: &str);                         // posts "scene" with the next rev, as the glue does at a gesture's end
    pub fn draw_unposted(&self, scene_json: &str);                // the canvas changed, not posted yet (status: rev > posted)
    pub fn post_stale(&self, scene_json: &str, rev: u64);         // a post that left the page before an adopt
    pub fn paste_picture(&self, id: &str, mime: &str, bytes: &[u8]); // posts "file"
    pub fn forget_file_messages(&self);                           // the next scene names a picture whose "file" never came
    pub fn set_status(&self, status: innerword_board_web::bridge::Status); // posts "status"
    pub fn hold_collects(&self); pub fn release_collects(&self);  // the answer of `collect` waits
    pub fn fail_adopts(&self, how: FakeFailure);                  // the next adopt throws, or never answers
    pub fn collects_asked(&self) -> usize;
    pub fn adopted(&self) -> Vec<String>;                         // the deltas, in order
    pub fn canvas(&self) -> String;                               // the fake canvas's scene now
    pub fn board_calls(&self) -> Vec<String>;                     // "undo 1", "selectAll", "zoom in", "search q", "insertPictures 2", "files a,b", "insertElements 1", "mermaid", "library toggle"
    pub fn change_library(&self, items_json: &str);               // posts "library" with the next lrev, as onLibraryChange does
    pub fn library_served(&self) -> Vec<String>;                  // the library.json each load was served
    pub fn parked(&self) -> bool;                                 // the closing view is parked
    pub fn board_pdf(&self, scene: &innerword_board::Scene) -> innerword_web::PrintedPdf; // the fixed PDF of that size
}
```

**Built** (the fake lives in `crates/innerword-app/src/boards.rs`, `FakeBoard`): every lever
above, and `change_library_unposted` (the library changed, not posted yet), `post_scene_raw`,
`board_status`, `press_help`, `fail_start` (a start that posts `START_FAILED`),
`make_board_pdf`, `fail_board_pdfs`, `board_pdfs_made` and `select`; Reload loads the
whiteboard again from its cell, which also resets its library counter. App-wide seams beside
it: `boards::hold_board_saves` / `release_board_saves` (a background save held, for a flush
that meets one) and `boards::fail_board_saves` (every save of a whiteboard fails, for the
recovery file). `tests/suite/boards.rs` held 38 cases after B8's first round, 66 after its
fix round, and 74 after the final review.

B8's tests: `tests/suite/boards.rs` (a submodule of the one test binary), and additions to
`keybindings.rs` (the new rows and place, the fixed rows, `NoAction` skipped as an entry,
the binding of the fixed keys after the defaults in both installs and the put-back test,
the "sees it first" question, `takes_key` false for every key of `WHITEBOARD_KEYS` and true
for `ctrl-enter`, none of `WHITEBOARD_KEYS` a key the window keeps), the title menu of a
whiteboard and `ctrl-enter` on one (`context_menus.rs`), the chevron and the status bar's
items (`mouse.rs`), `launch.rs` (the last open whiteboard reopens; one changed while closed
opens fitted; the shape library comes back), `print.rs` and `pdf_export.rs` (a
whiteboard's job: one page of the drawing's size, `Fit::ShrinkOntoPaper` recorded, the
picker's one row, one job at a time, an empty whiteboard; **a folder's PDF**: the documents'
pages as before, then one page per whiteboard of the fake PDF's size, an empty whiteboard
left out and named in the toast, the open whiteboard of the folder collected first, a
whiteboard's PDF that fails writes nothing, Cancel between two whiteboards, no engine: left
out with the sentence), `versions.rs` (preview, and restore with adopt), `images.rs` (a pasted
picture stored before the save that names it; a refused one; an HTML file posted as
`image/svg+xml` refused; a drop and Insert > Image… on a whiteboard), and `pages.rs`'s two
counts (6.8). B10's: `live.rs`, `assistant.rs`, and the library's New Whiteboard items in
`context_menus.rs` and `mouse.rs` (File > New Whiteboard, the + menu, "New Whiteboard
Here"), which are B7's panel items and wait for wave 3. Cases that must be there:

- drawing, then opening another document **while the post has not come** (`draw_unposted`,
  `hold_collects`): the view is parked, the new document opens, and once the collect is
  released the stroke is in the library; reopening the same whiteboard meanwhile waits for
  it;
- **every way of closing and quitting** with something unposted saves it: `cmd-w`
  (`CloseWindow`), the close button (`should_close`), `Quit` through its listener,
  `quit::request` as the signal task calls it, and a whiteboard in `closing_boards` at each;
  `on_app_quit` alone saves what the app holds;
- a whiteboard's own save does not adopt and does not reload; a rename while a save is in
  flight keeps the new title;
- **the compare-and-swap**: a post that left the page before an agent's change
  (`post_stale`) is not saved and the library keeps the agent's change; a save that finds
  the row changed (`Stale`) adopts and saves the merge; a failed adopt (`fail_adopts`)
  merges in Rust, saves, and reloads the view from the stored scene;
- an agent's change keeps an element the writer moved after the collect, and the note
  names the conflict; an agent's rename of the open whiteboard is not undone by the next
  autosave;
- **pictures**: the first save after an open keeps every stored picture; a scene that
  names a picture whose `file` never came asks `files` and keeps it; a picture the library
  lost (removed from `images.db` and its copy) does not stop the saves;
- **recovery**: a held scene that cannot be parsed is written to the recovery file
  (`BoardState::recovered` in memory) and the save state says so; closing keeps it;
- Reload whiteboard collects, saves and serves the current scene; a crash reloads from it;
- `cmd-shift-s` on a whiteboard offers exactly one row; a folder's Markdown, DOCX and
  plain-text exports leave whiteboards out and say so, its PDF holds them (above);
- **the shape library**: a `library` post is stored in `board.shape_library` and served
  as `library.json` to the next whiteboard opened, also after a relaunch; a change not
  posted yet comes with `collect` at quit; an item with a picture is dropped; a library over
  the limits is not stored, the last one stays and the toast says so; the three commands
  make their calls, and the picker lists the held items;
- "Mermaid to drawing…" from the palette makes the `mermaid` call; a scene posted after it
  with a Mermaid SVG picture stores that picture as an SVG;
- `cmd-g`, `cmd-alt-c`, `cmd-alt-[`, `cmd-d` are left to the page (`FakePages::press` answers
  false), `cmd-shift-p`, `cmd-f`, `cmd-s` are the window's; **the menu bar while a
  whiteboard is open has no key equivalent of `WHITEBOARD_KEYS`** (Show or Hide Comments,
  Vim Mode, Next and Previous Comment carry none), and has them back after it closes; a
  click on Show or Hide Comments there gives the toast and opens no panel;
- the six Add commands from the palette: an element at the middle, selected
  (`insertElements` with the converted skeleton); "Add arrow" with two shapes selected binds
  them;
- the Edit menu's Undo, Redo and Select All become glue calls; Copy becomes the view's
  `copy:`; the six items keep their key equivalents on a whiteboard (final review);
- `help` from the page opens the list of shortcuts at "On a whiteboard";
- no engine: a whiteboard shows its listing and the sentence; New Whiteboard is not in the
  palette.

---

## 7. Agents

### 7.1 Where skeletons become elements: pure Rust

**Decided: in Rust, in the new crate `innerword-board`**, not in a WebKit child running
Excalidraw's own `convertToExcalidrawElements`.

- An operation that writes runs where `Library::write` runs it: in the tool, **and on the
  app's main thread** (`live::execute`) when the app is open, and in both test stand-ins
  (`StandIn` of `innerword-agent`, `LiveApp` of `innerword-cli`). WebKit can run in none of
  these: the app must never run WebKit for an agent, and the stand-ins have no window
  server. A child could only convert in `Prepare`, against the scene the tool read, and
  `run` would still need Rust to merge, bind and order; update and remove are bookkeeping on
  the stored scene that no child would help with.
- Time: a WebKit child costs seconds (the page check, the nearest measure: 5 s from start to
  answer). An agent that draws a diagram in twenty calls would wait over a minute and a
  half; every other write of the tool takes 20 to 30 ms.
- Tests: every operation runs twice in `innerword-agent`'s suite (directly and through the
  stand-in app), and through the CLI and MCP suites; with a child, the default runs would
  not test the conversion at all.
- What it costs: **text sizes are close, not exact** until Excalidraw has seen the scene.
  Mitigated three ways. (1) `innerword-board` measures with the advance widths of the
  bundled fonts (U+0020 to U+024F; other characters 0.6 em, CJK 1.0 em), from
  `src/metrics/font-metrics.json`, a copy of the `dist/font-metrics.json` that B3's build
  writes from the font files (section 3; `include_str!`, parsed once); a test of
  `innerword-board-web` holds the copy equal to `dist/`'s, and B3's `UPGRADING.md` says to copy
  it again. Line heights are Excalidraw's own per family (exact: `height` is lines × size ×
  line height). (2) Excalidraw measures every text again whenever it loads a scene (the
  view, the export page: `restore` with `refreshDimensions` and `repairBindings`), so what
  the writer sees and every PDF are exact; the view's measure reaches the library with the
  writer's next change. (3) A case of `innerword-web`'s WebKit program holds the Rust output
  against Excalidraw's own for a corpus of skeletons (section 9, B11). **Built, and what is
  left:** every text-width difference is kerning: `innerword-board` sums advance widths and
  WebKit applies the fonts' GPOS pair kerning (fontkit's shaped widths of the bundle's
  Excalifont equal WebKit's to the hundredth). On the corpus texts are within 0.6 to 2.9 %
  ("Yes?" at 16 px is the 2.9 %); measured on 15 heavily kerned words in every family, 33 of
  105 are more than 3 % off, the worst "P." in Liberation Sans (+15.8 %), "LT" in Nunito
  (+12.5 %), "P." in Excalifont (+11.5 %); ours is always the wider, and the families without
  kerning (Virgil, Cascadia, Comic Shanns) are exact (`kerning_measured`, a note of the
  WebKit program, not a check). So a shape fitted to a short kerned label is a few pixels
  too wide until Excalidraw measures again. The fix is named and not done: the bundle's build
  writes the fonts' nonzero pair kerning into `font-metrics.json` (fontkit is there), and
  `metrics::line_width` applies it.
- What is written is what Excalidraw 0.18.1 writes: every field of every element type,
  their defaults read from the pinned package (`newElement`, `DEFAULT_ELEMENT_PROPS`,
  `FONT_METADATA`, `getLineHeight`, `BOUND_TEXT_PADDING`, the binding gap), fractional
  `index`es (a port of `fractional-indexing`'s `generateKeyBetween`), `seed` and
  `versionNonce` 31-bit numbers, ids like Excalidraw's (21 characters of nanoid's
  alphabet) unless the agent gave one.
- **Every value it makes is derived, never random** (reviewer's point, taken): the
  agents' suite runs every operation directly and through a stand-in app and compares the
  two libraries row by row, and the stand-in calls `live::execute(store, request)`, which
  no test-only `Library` constructor reaches; and a dry run reports the ids a real run
  would give. So `HashedMaking::new(&[board id, the stored scene's ContentStamp, the
  request's skeletons or changes as JSON, the `file` ids of its pictures])` (never the
  pictures' bytes, which the app's listener has already stored and dropped) gives the n-th id, seed and nonce from SHA-256 of those and n (an id
  that is already on the whiteboard is skipped by taking n + 1), and `now_ms` is the
  store's clock (`Store::clock().now_ms()`, manual in tests). The same input gives the same
  elements in the tool, in the app and in a dry run. Excalidraw's own `seed` only shapes
  the hand-drawn wobble and its `versionNonce` only breaks ties between collaborators, so
  derived values cost nothing. (The app's Add commands seed it with the held scene's stamp,
  the rev and the moment.)
- Elbow arrows (`elbowed: true`, routed around shapes) are not made by agents (refused) and
  are kept as they are when the writer drew them. **Built after review**, the rules that
  keeps: when what an elbow arrow joins moves, is resized, turned or labelled anew, the
  arrow stays exactly as it was (same version, not among the changed), its bindings with
  their `fixedPoint` untouched (0.18.1's `repairBinding` unbinds an elbow arrow whose binding
  lacks one; the first build rewrote them and bent the route); it moves whole only when
  everything it is bound to moved by the same distance in one change (a frame holding both
  ends). On an elbow arrow `start`, `end`, `points`, `width`, `height` and `angle` are
  refused (0.18.1 cannot turn one), and `x`, `y`, `dx`, `dy` while it is bound (as
  Excalidraw's own drag refuses), each naming the field and "elbow arrow". Excalidraw
  routes such an arrow again when the writer next moves what it joins; until then an arrow
  whose shape an agent moved looks detached, its binding intact.
- **Nothing it makes is invisibly small**: Excalidraw's `restoreElements` drops such an
  element at every load (`isInvisiblySmallElement`), so it would vanish from the canvas and
  then from the library after the agent was told "added". `add` and `update` refuse (exit
  2, with the index and the field) a line, an arrow or a freedraw whose points all lie
  within 1 px of each other, a shape or a frame narrower or lower than 1 px, a text that is
  empty or only white space, and a label that is; `update` refuses a change that would make
  an element so. The rule is 0.18.1's, read in its package, and `UPGRADING.md` lists it.
  (Built: this rule is stricter than 0.18.1's own `isInvisiblySmallElement`, which drops
  only fewer than two points or a width and height both 0.)
- **What else the converter decides** (built, beside the spec): a shape with a label and no
  size is sized to fit its label, as Excalidraw's converter does (only a shape without a
  label defaults to 100 × 100); a bound arrow's end slides along the given line onto the
  outline of its element (exact for rectangle, ellipse and diamond; aimed at the centre when
  the line misses it), its `focus` from a port of Excalidraw's `determineFocusDistance`, gap
  1, and every new arrow then loses 0.5 px at each end on each axis it runs along, as
  Excalidraw's converter does; an arrow's or a line's `width` and `height` may be 0 or
  negative (the extent of its points); a freedraw's width and height come from its points,
  with `pressures: []` and `simulatePressure: true`; a container's label comes first in its
  `boundElements`; a frame cannot hold a frame, a label cannot be named as a child, an arrow
  cannot bind to itself, a line, an arrow, a freedraw or a label. `add`, `update` and
  `remove` succeed whole or leave the scene as it was (a dry run runs them on a clone), find
  elements through an index (`skeleton::Positions`, and in `update` the arrows bound to each
  element and the children of each frame), so they are linear in the whiteboard and the
  request: 2,000 moves on 11,400 elements take 0.18 s in release (7.27 s before the review).
  A request with several bad fields names the first in sorted order, the same in `target/`
  (where gpui's `preserve_order` keeps insertion order) as in `target-pure`.

### 7.2 The skeleton an agent writes

A JSON array of objects in Excalidraw's own camelCase, so that an agent that knows
Excalidraw needs nothing new:

```json
[{"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,
  "backgroundColor":"#a5d8ff","fillStyle":"solid","label":{"text":"API"}},
 {"type":"ellipse","id":"db","x":420,"y":80,"width":160,"height":90,"label":{"text":"Postgres"}},
 {"type":"arrow","id":"reads","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
 {"type":"text","id":"title","x":100,"y":20,"text":"System overview","fontSize":28},
 {"type":"frame","id":"backend","children":["api","db","reads"],"name":"Backend"}]
```

| Type | Needs | May have |
|---|---|---|
| `rectangle`, `ellipse`, `diamond` | `x`, `y` | `width`, `height` (100 each by default; built: a shape with a label and no size fits its label), `label` |
| `text` | `x`, `y`, `text` | `fontSize` (20), `fontFamily`, `textAlign`, `verticalAlign` |
| `arrow` | `x`, `y`, and either `points` or `width`/`height` or `start`/`end` | `start`, `end` (`{"id": "<an element of the board or of this request>"}`), `startArrowhead`, `endArrowhead`, `label` |
| `line` | `x`, `y`, and either `points` or `width`/`height` | (built: `start`, `end`, the arrowheads and `label` are refused on a line; in 0.18.1 only arrows bind, have arrowheads and take a label) |
| `freedraw` | `x`, `y`, `points` | |
| `frame` | `children` (ids) or `x`, `y`, `width`, `height` | `name` |
| `image` | `x`, `y`, `path` (a picture file of this Mac) | `width`, `height` |

- Every type may also have: `id` (1 to 64 characters of `[A-Za-z0-9_-]`, made when not
  given; **built: an id the whiteboard already has is refused**, exit 2, "the whiteboard
  already has an element api; give another id", where the spec said one would be made: a
  silent new id would leave the agent naming the wrong element), `angle` (radians), `strokeColor`, `backgroundColor`,
  `fillStyle` (`hachure`, `cross-hatch`, `solid`, `zigzag`), `strokeWidth`, `strokeStyle`
  (`solid`, `dashed`, `dotted`), `roughness` (0, 1, 2), `opacity` (0 to 100), `roundness`
  (Excalidraw's `{"type": 3}`, or `null`), `groupIds`, `link` (`http`, `https` or `mailto`),
  `locked`. **Built:** an element link (Excalidraw's `?element=<id>` on the view's own
  address) is refused: Excalidraw treats one as such only when its host is the page's, which
  the converter does not know. Accepting `{"element": "<id>"}` and building the link where
  the host is known is the way to add it later.
- `label`: `{"text", "fontSize"?, "fontFamily"?, "textAlign"?, "verticalAlign"?,
  "strokeColor"?}`: a bound text inside the shape or on the arrow, wrapped to the shape's
  width as Excalidraw wraps it, the shape made taller when the text needs it.
- `fontFamily`: Excalidraw's number (5 Excalifont, 1 Virgil, 6 Nunito, 7 Lilita One, 8 Comic
  Shanns, 9 Liberation Sans, 3 Cascadia; B2 read the numbers in 0.18.1) or the name (ours
  only: Excalidraw's own converter takes a number and measures a name in a fallback font,
  so the cross-check turns names into numbers first, as it gives a frame without `children`
  an empty list, which Excalidraw's converter needs).
- **Text has no `width`, `height` or `lineHeight`** from an agent (refused): Excalidraw
  works out a line height from a given height, and a wrong one stays (a text 1 px high in the
  scout's run).
- `start`/`end`: the arrow's end is put on the edge of that shape (rectangle, ellipse and
  diamond worked out exactly), with Excalidraw's binding (`startBinding`/`endBinding` with
  `focus` and `gap`; the arrow added to the shape's `boundElements`). A new shape described
  inline (`{"type": "rectangle"}`, which Excalidraw allows) is refused: give it its own
  entry.
- `frame`: its children get `frameId`; without a size the frame is the bounds of its
  children and Excalidraw's padding.
- **`image`** (answer 3): `path` names a picture file of this Mac, as `innerword image add`
  names one: a whole path, `~/`, `file:`, or a path relative to the folder of `--file`
  (else to where the command runs; `Library::with_base_dir`); the MCP server is given no
  folder and takes whole paths only; an address on the web is refused (exit 2: "give the
  path of a picture file on this Mac"). The file is read **in the process that was asked**
  (section 7.6) and made by the whiteboard's own rule (section 2, "Pictures"): an SVG
  document as it came (`sniff_svg`, at most 4 MB), anything else through
  `innerword_image::prepare` (PNG, JPEG or GIF; HEIC, TIFF, BMP, WebP, AVIF converted; no GPS
  position; at most 4096 pixels and 16 MB). Without `width` and `height` the element has the
  picture's own size (its pixels at the density it declares, `pixels × 72 / ppi`, made
  smaller so that neither side passes 1,440, the size Excalidraw gives a pasted picture);
  with one of them the other follows the picture's proportions; with both, both are taken.
  The element is Excalidraw's image element (`fileId` the `FileId` hex, `status: "saved"`,
  `scale: [1, 1]`, `crop: null`) and the scene's `files` gets `{mimeType, file, created}`.
  One file named by two skeletons is read once and stored once. It may also have the
  fields every type has (`angle`, `opacity`, `link`, `locked`, `groupIds`, a frame's
  `children`); `strokeColor` and the fill fields mean nothing on a picture and are refused.
  A file that is not there or cannot be used refuses the request as `innerword image add`
  refuses it (exit 2, with its sentence, `PrepareError::sentence` about the file, and the
  element's index): nothing is written.
- **Refused**, exit 2 with the element's index and the field: an unknown type (`iframe`,
  `embeddable`, `magicframe`, `selection`), an unknown field, `customData`, `fileId`,
  `status`, `scale`, `crop`, `elbowed: true`, a text's size, a duplicate id, a value out of
  range, an element that would be invisibly small (7.1), more than `MAX_REQUEST_ELEMENTS`
  (2,000) elements, pictures weighing more than 32 MB together after they were made
  (`live::MAX_ATTACHED_BYTES`, the limit of `innerword image add`'s request), a request that
  would pass the whiteboard's limits. An id named by `start`, `end` or `children` that is
  neither on the whiteboard nor in the request: exit 3.

### 7.3 Changes and removals

`update` takes an array of changes by id:

```json
[{"id":"api","dx":40,"backgroundColor":"#ffc9c9"},
 {"id":"db","text":"Postgres 17"},
 {"id":"reads","end":{"id":"cache"}}]
```

- Fields: `x`, `y` (absolute) or `dx`, `dy` (relative), `width`, `height`, `angle`, `text` (of
  a text element, or of a shape's or an arrow's label: a shape without one gets one), every
  style field of 7.2, `fontSize`, `fontFamily`, `textAlign`, `link` (`null` removes it),
  `locked`, `name` (a frame), `start`/`end` (an arrow bound again), `points` (a line, an
  arrow, a freedraw). Pictures (the writer's and agents') can be moved, resized and rotated
  by id like any element; a change of a picture's file is refused (`path` and `fileId` are
  not fields of a change): remove the element and add another.
- What follows by itself, as when the writer drags: a shape's label moves with it (built:
  by the same distance when the shape is only moved, as Excalidraw's drag does, and laid out
  again only when its size or text changed, so the labels the writer drew do not drift by
  our font metrics); the bound ends of its arrows move with it (back onto the outline at
  the gap; a binding keeps its other fields when it names the same element); a frame moves
  what is in it; a resized line or arrow has its points scaled; a text the writer made
  narrower (`autoResize: false`) keeps its width and is wrapped to it again (Excalidraw's
  `refreshTextDimensions`; a turned text keeps the corner its alignment names); every
  element touched gets its `version` raised, a new `versionNonce` and `updated`.
- **Built (final review):** a change that sets what an element already has is left out,
  and each change of a request is judged against the drawing as it stood just before it,
  so a change that sets back what another change of the same request did (a frame moved,
  its child set back) is kept. A request that changes nothing answers "Nothing changed:
  board:1 “Architecture” already is so." and keeps no version.
- An id the whiteboard does not have: exit 3, with the ids it has as candidates (at most
  twenty, the nearest by name first).
- **Built (follow-up of the acceptance run):** a change that leaves an element not wholly
  inside the frame it belongs to (its bounds, rotation included, within the frame's
  rectangle and half a pixel), or its label not, is a warning: "db sticks out of frame
  backend: what is outside is not shown." The screen cuts a frame's children at its edges
  and so does the PDF (section 5), so a shape moved half out of its frame is half gone,
  which an agent cannot see. Said for each element the request changed (a label by its
  container) and for each child of a frame it changed (a frame made smaller), once per
  element, in the order of the scene; the same for `add` (an element added in a frame, a
  frame of a given size added around elements that do not fit in it; a frame fitted to
  its children says nothing). `innerword_board::frames::sticking_out`, in `Added.warnings`
  and `Updated.warnings`, passed through `BoardChanged.warnings`, a dry run too; the tool
  and the server print each as a `Note:` line.

`remove` takes ids. A shape goes with its label; arrows bound to it stay, unbound (removed
from its `boundElements`, their binding taken off); an arrow goes with its label; a label
id removes only the label; a frame goes and its children stay (their `frameId` cleared:
to remove them too, name them); a picture's `files` entry goes when no other element names
it (the bytes stay for the collector). The result lists what went and what was unbound.

### 7.4 Command line

```
innerword board new [<title>] [--in <folder>] [--create-folders] [--first|--before <item>|--after <item>]
                    [--content <json> | --file <path> | --stdin] [--author <name>] [--dry-run] [--json]
innerword board add <board> (--content <json> | --file <path> | --stdin) [--author <name>] [--dry-run] [--json]
innerword board update <board> (--content <json> | --file <path> | --stdin) [--author <name>] [--dry-run] [--json]
innerword board remove <board> <id>... [--author <name>] [--dry-run] [--json]
innerword boards [<folder>] [--json]                  (also: innerword board list; whiteboard, whiteboards)
innerword get <board> [--format text|json] [--json]
innerword export <board> <file.pdf> [--overwrite]
innerword info|find|history|rename|move|duplicate|trash|restore|delete <board> ...   (as for any item)
```

All in the group "Documents" after the page commands; the four that write say "This
command changes the library." `board new` takes the elements to start with as optional
content, like `create`. A skeleton of type `image` names its file with `path`: with
`--file`, a relative path is taken from that file's folder, otherwise from where the
command runs, as for the pictures of `--file` Markdown. The elements and changes are a
JSON array given with `--content`, `--file` or standard input (a bad one is exit 2; a
missing picture says where a relative path was looked for); the help has tables of the
skeleton's fields and of a change's. `board new` also takes `--first`, `--before`,
`--after`. Outputs, **as built** (run against the debug tool when this text was revised):

```
$ innerword board new "Architecture" --in Sketches
made board:1 “Architecture” in Sketches (empty)

$ innerword board add board:1 --file shapes.json --author Claude
added to board:1 “Architecture”: rectangle api (label), ellipse db (label), arrow reads (label), text title, frame backend
kept the drawing as it was: version 1 “Before a drawing by Claude”; "innerword history restore 1" brings it back

$ innerword board add board:1 --author Claude --content '[{"type":"image","id":"icon","x":640,"y":80,"path":"/…/icon.png"}]'
added to board:1 “Architecture”: image icon (icon.png, PNG as it came, 40 x 30 pixels, drawn 40 x 30)
kept the drawing as it was: version 4 “Before a drawing by Claude”; "innerword history restore 4" brings it back

$ innerword board update board:1 --content '[{"id":"api","dx":40}]' --dry-run
Dry run: nothing was changed. The real run would report:
changed on board:1 “Architecture”: api; 3 followed (labels, the ends of joined arrows, what a frame holds)

$ innerword board update board:1 --content '[{"id":"api","dx":0}]'
Nothing changed: board:1 “Architecture” already is so.

$ innerword board remove board:1 db --dry-run
Dry run: nothing was changed. The real run would report:
removed from board:1 “Architecture”: db and 1 label; unbound (they stay): reads

$ innerword boards
board:1  Sketches/Architecture  4 elements, 0 pictures  changed 2026-11-23T17:39Z
```

**Built**, against the spec's examples: a dry run uses the prefix every command has, then
the real run's line; a removal names the ids and counts the labels, because `BoardChanged`
carries ids, not kinds; an update names the ids the request named and counts what followed
(a label's id is noise); the version line ends with the restore command, as other
commands' do (the MCP server makes it `restore_version`); a list line ends with `changed
<UTC minute>` like every other list; a picture says "PNG as it came" or "converted to
JPEG"; an untitled whiteboard is stored `""` and shown "Untitled" in every output.

`innerword get board:12` (and MCP `read_document`): the **listing**, one element per line,
ids first, a label with its container and not on a line of its own:

```
board:1 “Architecture” in Sketches: 5 elements, 1 picture; drawn area 590 x 160 at (90, 20)
rectangle  api      at (140, 80)  200 x 90                  fill #a5d8ff solid  label "API"
arrow      reads    from api      (341, 125) -> (419, 125)  label "reads"
text       title    at (100, 20)  222 x 35                  "System overview"  28 px Excalifont
frame      backend  at (90, 70)   500 x 110                 "Backend"  holds api, reads
image      icon     at (640, 80)  40 x 30                   picture stored (png)
```

(Built: the lines are in z-order; a picture says `picture stored (png)` or `picture
missing`, without a pixel size, which a scene does not hold. The header's count of
elements leaves labels out: a label is part of its shape or arrow, and `info`, `boards`,
the status bar and the assistant count the same way, `info` naming the labels apart.)

`Scene::listing()` writes it, the one function behind the tool, the MCP server, the
assistant, the version history's preview and the app's view without an engine. `--format
json` prints the stored scene as it is (built: unparsed, so a scene the parser refuses can
still be printed, with `board: None`; the listing formats fail on it, exit 13).
`--json` (and the MCP tools' structured results)
gives `DocumentText` with `board: Some(BoardInfo { summary, elements: [{id, type, x, y,
width, height, angle, text, label, container, start, end, points, frame, groups, style:
{…}, link, locked, picture: "stored"|"missing"}] })`. `--format markdown` is the listing too.
`info` adds `board: BoardInfo { summary }` (built, it prints "holds: 4 elements (1 arrow, 1
frame, 1 rectangle, 1 text) with 2 labels, 0 pictures, 5 words"); `find board:12 "reads"`
finds in `Scene::text()` and names the element of each hit (`reads (arrow label): reads`;
built: it asks `innerword_board` for the reading order, so a hit names its element where
one text begins another). `innerword boards --json` gives `BoardListed { board, elements,
pictures, trashed, unreadable }` (built: a whiteboard whose scene cannot be read is listed
with `unreadable` and why, never failing the list); every candidate of an error carries its
reference (`Candidate.reference`: `doc:`, `page:`, `board:` or `folder:`), so no front end
writes a prefix of its own.

Hints (`innerword-cli`, `hint_in`): a text command on a whiteboard ("Here: innerword board
add, board update and board remove change a whiteboard; "innerword get <board>" lists its
elements with their ids."), a whiteboard command on a text ("Here: innerword boards lists
the whiteboards."), an export of a whiteboard that is not a PDF ("Here: a path that ends in
.pdf."), an element id not found (exit 3, "lists the ids"). Built: the hints say `<board>`,
because the error report carries no id. The overview's limit of 75 lines (`help.rs`) is
raised to 81, and its export line says "a whiteboard as PDF". The help of exit code 15
becomes "a page or a whiteboard where a document is needed, or the reverse" (built: it
cannot keep the old words "a page where a document is needed"; the skill's test now reads
the new ones).

### 7.5 MCP

| Tool | Effect | Arguments |
|---|---|---|
| `create_board` | add | `title`, `folder`, `create_folders`, `elements` (array, optional), `author`, `dry_run` (built: also `first`, `before`, `after`, as `create_document`) |
| `add_to_board` | add | `board`, `elements` (array), `author`, `dry_run` |
| `update_board` | change | `board`, `changes` (array), `author`, `dry_run` |
| `remove_from_board` | change (`destructiveHint` as `delete_comment`'s) | `board`, `ids` (array of strings), `author`, `dry_run` |
| `list_boards` | read, **hidden** (as `list_pages`) | `folder` |

- 41 listed tools, **12 of them read-only, as before**: a whiteboard is read through
  `read_document`, `document_info`, `find_in_document`, `list_versions`, `read_version`
  (their board branches); `export` takes `format: "pdf"` for a whiteboard ("A whiteboard is
  written as a PDF only."). So the assistant's tools do not change, and the isolation check
  of 0.4.0 still describes them.
- The removal tool is not called `delete_from_board`: a test forbids listed tools that start
  with `delete_d` or `delete_f` (`tools.rs`).
- `args.rs` gets `array_of_objects(description)` and `array_of_strings(description)`; an
  array sent as a JSON string that parses as an array is taken (models do that); built: a
  single object, or a single id, counts as an array of one, and an id given as a number
  becomes a string.
- `tool_error_text` says, for a text operation on a whiteboard, which tools change one
  (`BOARD_TOOLS`), and after an export that is not a PDF what a whiteboard is exported as
  (`BOARD_FORMAT`). Built: `tool_text` has no mapping for "innerword board add" and the
  others, since no text the server returns names those commands; the error text appends the
  tools instead. A `pictures` argument is refused by the schema and again by the operation
  (`PICTURES_REFUSED`). `add_to_board`'s description says an id the whiteboard has is
  refused (7.2). An ambiguous name lists each candidate by its own prefix. `INSTRUCTIONS`
  has one sentence about whiteboards.
- `render.rs`: the kind's prefix (`board:12`) and noun ("whiteboard") from
  `DocumentKind::reference_prefix()` and `noun()`; the listing for `read_document`; built:
  `board_changed` (with `BoardDid`), `boards`, `board_info` (counted as the listing counts,
  its kinds as plurals and its labels apart), "Untitled" for an item without a title in
  every output, a damaged whiteboard listed with "cannot be read" and why, and `history
  restore` of a whiteboard speaking of the drawing ("Restored version N (…) of “Plan”: the
  drawing is as it was then.", "The drawing as it was is kept as version N"), not of the
  text and its words.
- **Pictures through MCP**: `create_board` and `add_to_board` take skeletons of type `image`
  with a whole `path` (the server is given no folder, as for `add_image`); the server's
  process reads and prepares the file (it is the process that was asked) and the request
  carries it. Their descriptions say so in one sentence ("A picture: {"type":"image",
  "x":…, "y":…, "path":"/Users/…/photo.jpg"}; a whole path of this Mac."). No tool returns a
  picture.

### 7.6 The operations table and the channel

Four rows of `operations.rs`, after `add_page`:

```rust
/// Makes a whiteboard, empty or with elements. See [`CreateBoardRequest`].
create_board / CreateBoard(CreateBoardRequest) -> BoardChanged = boards::create_board;
/// Adds elements to a whiteboard: shapes, text, arrows bound to shapes, frames. See
/// [`AddToBoardRequest`].
add_to_board / AddToBoard(AddToBoardRequest) -> BoardChanged = boards::add_to_board;
/// Moves, resizes, restyles or rewrites elements of a whiteboard by id. See
/// [`UpdateBoardRequest`].
update_board / UpdateBoard(UpdateBoardRequest) -> BoardChanged = boards::update_board;
/// Removes elements of a whiteboard by id, with their labels; arrows bound to them stay,
/// unbound. See [`RemoveFromBoardRequest`].
remove_from_board / RemoveFromBoard(RemoveFromBoardRequest) -> BoardChanged
    = boards::remove_from_board;
```

- `attach.rs`: `brings_nothing!` for `UpdateBoardRequest` and `RemoveFromBoardRequest`;
  **`CreateBoardRequest` and `AddToBoardRequest` bring the pictures their skeletons name**
  (`impl Prepare`, answer 3): for each skeleton of type `image`, its `path` is resolved as
  `innerword image add` resolves one (`file_of`, the request's `base_dir` or the library's),
  read, and made by `board_picture` (`innerword_board::sniff_svg` and at most 4 MB, else
  `innerword_image::prepare_file`); the result goes into the request's `pictures`
  (`BoardPicture { source, file, mime_type, pixels, density, untouched, data, stored }`,
  `data` base64 on the channel, as `AttachedImage`'s; `stored` never travels, as
  `Blob.stored`; `untouched` says the file was stored as it came; an SVG's size is read
  from its root element), one entry per distinct path, the total at most
  `live::MAX_ATTACHED_BYTES` (32 MB). **`Prepare` always prepares again** and drops whatever
  `pictures` the request came with, as `AddPageRequest`'s does with `prepared`, and both front
  ends refuse a request that names `pictures` (the field is not an argument of the command
  line or of the MCP tools): the bytes a whiteboard gets are always the bytes of the file
  the asked process read. A dry run prepares (so it says what the file would be) and stores
  nothing.
- **Stored before the scene names them.** Carried out directly (the app closed), the
  operation stores each picture (`Store::put_file`, which checks the hash) before it saves
  the scene, in the same write under the lock. Carried out by the app, **the listener's
  thread stores them** through its own `ImageFile` before it queues the request, as it stores
  a page's files: `live::store_page_files` learns a whiteboard's pictures beside a page's
  blobs (a picture's bytes whose hash is not their `file`, or whose type is not what they
  are, are refused, with a sentence of their own: built, `DAMAGED_BLOB`'s names a page),
  marks them `stored` and drops their bytes, so the main thread writes the scene
  only; `carries_page_files` is true for a request with pictures too (refused at once while
  the owner's switch is off, and nothing is stored). Both keep their names, because the
  listener of the app, the stand-in of `innerword-cli`'s harness and their tests call them;
  their documentation says "a page's files or a whiteboard's pictures". In both, `save_board` and `create_board` refuse a scene that
  names a file `images.db` lacks (`FilesMissing`), so bytes always come first. A picture
  stored for a request that then fails is named by nothing and goes with the collector.
- **Protocol version 4** (`live::PROTOCOL_VERSION`), with a paragraph in the module's notes:
  "Version 4 added whiteboards: four operations an app of version 3 does not know (it would
  answer `invalid`, 'the request cannot be read'), the kind in `Detail::WrongKind`, and the
  pictures a whiteboard's request carries (`pictures`, stored by the listener before the
  request is queued)." So a 0.10 app and a 0.11 tool refuse each other whole, exit 5,
  "nothing was changed", as for 0.6.0 and 0.7.0. A test sends a whiteboard's request, one
  with a picture among them, to a stand-in that answers as version 3.
- `Detail::WrongKind { is_page, export }` becomes `{ kind: DocumentKind, wanted:
  Option<DocumentKind>, export: bool }` on the wire (covered by the version); `id` and
  `name` stay where they are (7.7).
- A request without a picture is small (skeletons; at most `MAX_REQUEST_ELEMENTS`); one
  with pictures is at most 32 MB of them, about 43 MB of base64 in the line, under the
  256 MB a line may be.
- The app's half: `workspace::sentence` (6.7), `take_over_open_board`, the collect before
  (6.7); a request for a whiteboard that is not open changes the library only (the sidebar
  follows its events: `DocumentCreated`, or `VersionKept` then `DocumentText`; a rename
  `DocumentRenamed`). The app's listener already stores a whiteboard's pictures through
  `carries_page_files` and `store_page_files`; no other change was needed there.
- **Built, the types** (`innerword-agent/src/types.rs`): `CreateBoardRequest { title,
  folder, create_folders, placement: Option<Placement>, elements, pictures, base_dir
  (skipped), write (flattened) }` (`placement` as `CreateDocumentRequest` has it),
  `AddToBoardRequest { board, elements, pictures, base_dir, write }`, `UpdateBoardRequest {
  board, changes, write }`, `RemoveFromBoardRequest { board, ids, write }`, and the answer
  `BoardChanged { board, added, updated, removed, unbound, pictures: Vec<PictureAdded>,
  version, warnings, written (flattened) }`. The spec's `changed` is `updated`: the flattened
  `Written` has `changed: bool` already, and two "changed" keys made the app's answer
  unreadable. `pictures` (`PictureAdded { id, source, mime_type, pixels, untouched, width,
  height }`) lets the command line say "converted…, drawn W x H". The ids are derived from
  the board id, the scene's `ContentStamp`, the request as JSON with its keys sorted and the
  pictures' file ids, so the two runs of the suite store the same bytes.
- `READERS` (the test that every public method of `Library` is in the table or a known
  reader) gets `boards`.

### 7.7 The safety net, dry runs, errors

- A version before every change (the labels of section 2, "Versions"), pushed as `Change::VersionKept` then
  `Change::DocumentText`. `create_board` keeps none. `remove_from_board` erases nothing for
  good (its version has it), so it needs no `--permanently` and is a listed MCP tool.
- `--dry-run`: the conversion, the merge and every check run (a picture's file is read and
  made, so a file that cannot be used is refused as in the real run), nothing is written
  and no picture is stored; the result says what would be added (with the ids the real run gives, since they are derived, 7.1),
  changed, removed and unbound, or "would change nothing" (`Written::would_change`).
- **Built:** a new `Error::EmptyBoard { id, name }` for exit 9 (the PDF of an empty
  whiteboard), `ItemKind::Element` for exit 3, and `BOARD_IS_PDF_ONLY`. The table of
  operations has 30 rows; `seed` of the agents' suite holds a whiteboard (`seed_texts` is the
  old one, for the schema-1 test); every operation and read runs both ways.
- `Error::WrongKind { id, name, kind, wanted, export }`: **`id` and `name` stay** (the
  sentences name the item), `is_page` becomes `kind` (what the item is) and `wanted` (the
  kind the operation needed; `None` for "a text"). The sentences: "“Plan” is a whiteboard:
  it is changed with board add, board update and board remove." / "“Chapter 1” is a
  document, not a whiteboard." / "“Orbitarium” is a page, not a whiteboard." / "“Plan” is a
  whiteboard, and a whiteboard is exported as a PDF only."
  (`innerword_agent::BOARD_IS_PDF_ONLY` for the front ends to recognise.) **Every sentence
  of a page stays byte for byte** (" is a page: ", `PAGE_IS_NOT_A_TEXT_FILE`), so the
  command line's hints (`innerword-cli/src/main.rs`, which matches " is a page: ") and the
  server's `tool_error_text` keep working through wave 2 untouched; B9 adds the whiteboard's
  hints in wave 3. B6 changes the struct literals that name `is_page` outside its crate,
  which are tests only: `innerword-mcp/src/tools.rs` (three, around line 1899) and
  `innerword-agent/tests/suite/pages.rs` (around line 771); nobody owns `innerword-mcp` in
  wave 2. Exit codes as decision 15 says; a trashed whiteboard is exit 9, as a trashed
  document.
- **The first commit of B6 turns every `kind == DocumentKind::Page` / `!= Page` /
  `is_page()` of `innerword-agent` into an exhaustive `match`** (scout-agents, section 1.2:
  `content.rs` `load`, `write_text.rs` `read` and `find`, `browse.rs` `info`, `comments.rs`,
  `items.rs` `rename_document`, `versions.rs` (`reference_of`'s `_ => doc:` would call a
  whiteboard a document), `export.rs`, `pages.rs`, `types.rs`). `content::load` refusing a
  whiteboard is the line that keeps every text operation off one.

### 7.8 The skill

A paragraph "Whiteboards." after "Pages." in `crates/innerword-agent-setup/src/skill_body.md`:

> **Whiteboards.** A whiteboard is a drawing in the library, drawn in the app with
> Excalidraw: shapes, arrows, text, frames and pictures on a canvas. `innerword board new
> "Architecture" --in Sketches` makes one (with `--file shapes.json` it starts with
> elements). `innerword get board:12` lists its elements, one per line with id, type,
> position, size and text; `--format json` gives the scene as Excalidraw stores it. Add
> elements as Excalidraw's element skeletons: `innerword board add board:12 --content
> '[{"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"label":{"text":"API"}},{"type":"ellipse","id":"db","x":420,"y":80,"label":{"text":"Postgres"}},{"type":"arrow","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"}}]'`.
> Give shapes room for their labels and leave out the width and height of text. A picture
> from a file of this Mac is an element too, `{"type":"image","id":"photo","x":640,"y":80,"path":"/Users/ada/Pictures/harbour.jpg"}`
> (a whole path, or one relative to the folder of `--file`; it is drawn at its own size
> unless you give `width` or `height`); the tool reads the file and the library keeps a copy,
> as it does for `innerword image add`. `innerword
> board update board:12 --content '[{"id":"api","dx":40,"backgroundColor":"#ffc9c9"}]'` moves,
> resizes, restyles or rewrites by id; `innerword board remove board:12 db` removes elements
> with their labels. Every change keeps the drawing as it was as a version first. A
> whiteboard is exported as a PDF only: `innerword export board:12 ~/Desktop/architecture.pdf`.
> Its text is not edited with `edit` or `append` (exit code 15).

**Built:** the paragraph is as above, with two sentences more: "`innerword boards` lists the
whiteboards." after the `--format json` sentence, and "`innerword board add --help` lists
every field an element may have." at the end; it is followed by a block of the commands
(where the whiteboards are listed with `innerword board list --json`, because the skill's
test checks the help heading of every command it shows and `boards` answers as "board
list"); "whiteboards" joins the skill's description.

The exit code lines, as built: 15 "a page or a whiteboard where a document is needed, or
the reverse. A page cannot be edited; to bring a newer copy, add it again with `page add`
and its source address. A whiteboard is changed with `board add`, `board update` and `board
remove`, and exported as a PDF only."; 3 names "an element of a whiteboard" among what is
not found ("an element's id with `get`"); 9 adds "A whiteboard with nothing drawn on it has
no PDF either." `skill_against_binary.rs` gets
`the_examples_of_whiteboards_run_against_the_built_tool` (new, add, the picture's add with
its path replaced by a PNG the test writes itself into its temporary folder (built: not
`innerword_image::fixtures`, so as to add no dependency), update, remove, get, export to a `.pdf` with `INNER_WORD_PAGE_CHECK=fake`, on a
temporary library); the skill's text test holds the paragraph's sentences.

---

## 8. Security and isolation

A whiteboard's page is our own code, but what it draws is data an agent may have written
after reading anything, and the editor is a large third-party program. Every threat with
what closes it:

| # | Threat | Closed by |
|---|---|---|
| 1 | The bundle reaches the network (fonts from a CDN, collaboration, library browsing, embeds, AI, analytics, link previews, preconnects) | The pages' isolation unchanged: the CSP on every answer, the content rule list, WebRTC and media devices off, no preconnect, one non-persistent data store. Excalidraw's CDN font fallback patched out at build (the build fails when the patch stops applying); collaboration not rendered; embeds, AI and the library browser off (4.13); tracking off in the package. A WebKit test loads the bundle and edits, exports and makes a PDF with a 127.0.0.1 server watching: no request reaches it, no CSP report, nothing missing. |
| 2 | The bridge as a door into the app | Only a whiteboard's view has it; one name; the main frame of the whiteboard's own origin only, and **a view with the bridge loads no subframe** (every subframe navigation cancelled: WebKit reports a post a same-origin frame makes through `top.webkit.messageHandlers` as the main frame's, so the frame check alone would not do; built, B11); a string under 24 MB (after a lossy conversion that mends a lone surrogate); a known `v` and `kind`, else dropped and counted. What it can do is bounded: replace **this** whiteboard's scene with what its page holds (compare-and-swap, 4.6), offer picture bytes, which the bytes' own type routes through `sniff_svg` or `prepare`, replace the shape library with what the page holds (read by `innerword_board::ShapeLibrary`, items with pictures or frames of the web dropped, at most 8 MB), and open the list of shortcuts (`help`). Nothing it says is run, opened, or taken as a command; every scene is read by `innerword_board` before it is saved. A page's view has no handler (a WebKit test: `window.webkit.messageHandlers.innerwordBoard` is undefined in a page). |
| 3 | A scene carries something harmful (HTML in a text, a script link, a huge text, an embed, an `iframe` element whose `srcdoc` Excalidraw renders with `allow-scripts`), written by an agent, or pasted by the writer from Excalidraw JSON that any website can put on the clipboard, or brought back by an undo | Excalidraw draws text on a canvas and escapes it in its SVG; links open only by the reader's act and only `http`, `https`, `mailto`; `iframe`, `embeddable`, `magicframe` are deleted by the glue the moment they are on the canvas (paste, change, undo, adopt, load), dropped from every scene read and refused from agents; the whiteboard's own policy (4.2: `script-src 'self'`, no eval) lets no inline script run, and **a view with the bridge loads no frame at all** (every subframe navigation cancelled), so an `iframe` element's `srcdoc` runs nothing even before the glue deletes the element (built: the policy alone would let a `srcdoc` that names one of the bundle's own scripts run it, B11); no `alert` toast on a whiteboard; `customData` and `fileId` are refused from agents; a link with any scheme but `http`, `https`, `mailto` and the view's own (an element link) is taken out of a paste, out of every scene `innerword_board` reads (`link_is_kept`) and on the export page, since `exportToSvg` makes it a link of the PDF; a value nested deeper than 100 levels (Excalidraw keeps `customData` and unknown fields through a paste) is read as `null` with a warning instead of making the drawing unreadable (`json::parse_lenient`); the limits of section 2 and 7. The worst a bad scene does is draw badly in its own whiteboard, and every agent's change kept a version first. |
| 4 | A link opened without the reader (a script's click, a message) | The reader-only gate, unchanged (pages' ISO-4); no bridge message opens a link; the canvas's link mark opens nothing; the link popover's real `<a>` opens once, on a real click. |
| 5 | The whiteboard's page navigates away, or is replaced, and loses what is not saved | `one_document`: no navigation after the first load; downloads cancelled; `cmd-shift-s` taken by the window before Excalidraw's "save to disk"; the open panel answers nothing. |
| 6 | Another origin reads a whiteboard's storage, or a page reads a whiteboard's | Each whiteboard its own origin, `board-<id>-<created_at>`, never `page-…`; storage lasts only while the app runs. |
| 7 | The supply chain of the bundle (npm) | Exact versions, the lockfile with integrity hashes, `npm ci --ignore-scripts`; `dist/` committed and reviewed as code, with the sha256 of every input in `BUILD-INFO.json`, checked by `cargo test` and rebuilt byte for byte by `node build.mjs --check` (and by `scripts/bundle.sh` where node is); a new Excalidraw is a change of its own (`UPGRADING.md`). |
| 8 | Someone points the app at other JavaScript | No variable does: a bundled app reads only its own signed `Resources/Whiteboard/`; a released binary compiles in no other path. |
| 9 | Pictures carry the writer's location, or a picture file is something else (the page names the type of what it posts, an agent names a path) | An agent's file is read in the process that was asked (the tool, the MCP server), never by the app, which stores the bytes the request brought and opens no path an agent named (as for `add_image`); a request cannot bring bytes of its own choosing under a picture's name (`Prepare` prepares again, the front ends refuse `pictures`, the store checks every hash). The bytes decide, never the named type: bytes stored as they came only when `sniff_svg` reads an SVG, everything else through `prepare` (no GPS position, upright, the formats the library knows); a picture is served and recorded with the type of its stored bytes, never `text/html`; an SVG is only ever drawn as a picture (its scripts never run); what `prepare` refuses is not stored. |
| 10 | The user's clipboard | Only the writer's own copy and paste touch it (Excalidraw's, on their key or menu item); a scripted run never presses `cmd-c` on a whiteboard; `navigator.clipboard.read` from Excalidraw's menu shows WebKit's own Paste button, which the writer must press. |
| 11 | WebKit in the tool, or in the MCP server's process | The PDF is made in a child (`__board-pdf`) with standard input closed, a private work directory, a time limit; `available()` asked first. Conversion and every operation are pure Rust. |
| 16 | Mermaid as a way in: a diagram (written by the writer, pasted from anywhere, or put on the clipboard by a website) with HTML in a label, a `click` handler, an image node naming an address, an `%%{init}%%` that raises Mermaid's security level | Mermaid stays at `securityLevel: "strict"`: labels go through DOMPurify, no click callback runs, and `securityLevel` is in Mermaid's `secure` list, which a diagram's directive cannot set (read in 11.17.2). The SVG it renders lives in an off-screen `div` for the time of the conversion; the policy runs no inline script or handler in it and loads no picture from an address (`img-src`, the rule list); what stays is Excalidraw elements (text drawn on a canvas) or one SVG picture drawn through `<img>`, whose scripts never run, stored by `sniff_svg`'s rule. Nothing of Mermaid needs eval, a worker or the network (4.13), so the live view's policy is not widened for it. |
| 12 | A whiteboard that cannot be saved (too large, a scene the parser refuses, a picture the library lost), or a request that floods the app | Strict limits for agents (`MAX_SCENE_BYTES`, `MAX_ELEMENTS`, `MAX_TEXT_CHARS`, `MAX_REQUEST_ELEMENTS`); for the writer only a hard cap far above any drawing, a lenient parser, parts for a large scene, a store that refuses only *new* missing pictures, and a recovery file for whatever still cannot be saved (4.6); `MAX_MESSAGE_BYTES` per message (a picture whose `file` message would be longer is never posted and answered `missing`, so the autosave goes on); the PDF's side capped at 14,400 pt (`innerword_web::MAX_PDF_SIDE`, whatever the export page answers). |
| 15 | A late or forged scene writes over newer content (a post that left the page before an agent's change, a stale view after a reload, a foreign script replacing the glue's calls) | The compare-and-swap of every app save against the stamp the view's scene descends from; posts held while an outside write is adopted, and dropped while a view reloads; the source serves the current stored scene at every load, and a page without a readable `board.json` never posts `ready` (`START_FAILED`); the policy and the cancelled subframes let no script but the bundle's run in the page world, where `collect` and `adopt` live. |
| 13 | A whiteboard names stored bytes it should not see | The view's source answers `_files/<FileId>` only for ids the whiteboard names or the app stored for it. (Even so, nothing in the view can send bytes anywhere.) |
| 14 | A stale or missing bundle | `Assets::find` refuses a folder without its `MANIFEST.json`; the app then has no whiteboard view ("The whiteboard's files are missing from Inner Word") and the tool's PDF fails with that sentence. |

---

## 9. Implementation plan

### 9.1 Waves and owners

**One owner per crate per wave.** In a wave every package builds against the interfaces of
9.2, not against each other's unfinished commits; a package that needs another of its
wave's commits (B4's tests need B3's first `dist/`) says so.

| Id | Title | Crates (and the few lines elsewhere it may touch) | Wave | Depends on | Effort |
|---|---|---|---|---|---|
| B1 | Store: kind `board`, lazy schema 5, compare-and-swap | `innerword-store` (and the `WrongKind` patterns in `innerword-agent/src/pages.rs` and `innerword-library/tests/panel/pages.rs`: add `..`, keeping today's mapping, which B6 makes exhaustive) | 1 | none | high |
| B2 | The scene in Rust | `innerword-board` (new) | 1 | its metrics: B3's first milestone (`dist/font-metrics.json`) | xhigh |
| B3 | The bundle | `innerword-board-web` (new), root `.gitignore`, `.gitattributes`, `Cargo.toml` (its workspace line) | 1 | its tests: B2's first commit (the crate with its constants) | xhigh |
| B4 | WebKit | `innerword-web` only (`ViewExtras`, `new_with`: no line of the app) | 1 | B3's first milestone for its board cases | high |
| B5 | Small additive changes | `innerword-print` (`PrintRequest`, `Scaling`, `PdfWriter::add_pdf_page`), `innerword-ui` (`IconName::Whiteboard`), `innerword-assistant` (`OpenKind::Board`, `BOARD_PREFACE`, the prompt), and the one literal of `PrintRequest` in `innerword-app/src/printing.rs` (the only line of the app in wave 1) | 1 | none | medium |
| B6 | Agents' command layer | `innerword-agent` (with `ProcessChecker::print_board`), and the `WrongKind` literals of `innerword-mcp/src/tools.rs`'s tests | 2 | B1, B2 | xhigh |
| B7 | Library model and sidebar | `innerword-library` | 2 | B1, B5 (icon) | medium |
| B8 | The whiteboard in the app | `innerword-app` | 2 | B1, B2, B3, B4, B5 (not B7: it writes through `Library::change_outside` until wave 3) | xhigh |
| B11 | The converter's cross-check | `innerword-web`'s `tests/webkit.rs` (its board corpus case only) | 2 | B2 finished (converter and corpus), B3's `convertSkeletons`, B4 | medium |
| B9 | Tool, MCP server, skill, the assistant's tool count | `innerword-cli`, `innerword-mcp`, `innerword-agent-setup`, `innerword-assistant` (the count test of `claude/args.rs`) | 3 | B2, B3, B4, B6 | high |
| B10 | Agents and the panel in the app; docs; bundling | `innerword-app`, `CLAUDE.md`, `docs/gpui-notes.md`, `docs/pages-design.md` (the bridge's line), `scripts/bundle.sh`, `about.hbs`, `packaging/Read Me First.txt` if it lists contents | 3 | B6, B7, B8 | xhigh |

**Built:** every package was reviewed after its build; B2, B3, B8 and B10 had a fix round
in place, and five follow-up packages took the reviews' points that touched a crate whose
wave was over: B1f (`save_board` reads and hashes the stored scene once; the plan test of
`boards()`), B1g (`save_board`'s doc says what a refused save still does in `images.db`),
B6f (a damaged whiteboard listed, an untitled one stored `""`, every prefix from
`reference_prefix`, a JSON read unparsed), B7f (`new_board_command`, `TREE_KEY_CONTEXT`),
B11f (B3's six WebKit cases of the glue ported into `tests/webkit/bundle.rs`, the
cross-check's slack by alignment, the kerning measured, the `srcdoc` cases). A final review
of the whole feature then fixed what is marked "final review" in this text (the commits
after `acd66c1`). B10 also wrote CLAUDE.md, `docs/gpui-notes.md` (the bridge, the calls,
`park`, the fixed `NoAction` bindings, and the lesson of `cx.defer`: an app-wide action
listener reached from a keystroke runs inside that window's update) and
`docs/pages-design.md`'s line on the bridge. One slip of the process: B6's commit 4fd86be
also carries three `Cargo.lock` lines of B8's (`git commit -- Cargo.lock` takes the working
file), which matched B8's `Cargo.toml` and broke nothing.

**Inside wave 1, two declared hand-offs** (reviewer's point, taken: B2 and B3 needed each
other's work): B2's **first commit** is the crate with its public constants
(`EXCALIDRAW_VERSION`, `limits`, `export_dir::SCENE_FILE`, `FILES_DIR`) and the types of
9.2 with their bodies, so that B3's tests can dev-depend on it; B3's **first milestone**
(`dist/` built with the patches, its fonts, `dist/font-metrics.json`, `MANIFEST.json`) is
committed early, for B2's metrics and B4's board cases. Until it is there B2 works on
everything but the text measures, and B4 on everything but its board cases. Nobody waits on
a Python package or on a step outside the lockfile.

Rules for every package (CLAUDE.md's working rules hold): the pure crates
(`innerword-board`, `innerword-board-web`, `innerword-web`, `innerword-store`,
`innerword-agent`, `innerword-cli`, `innerword-mcp`, `innerword-agent-setup`,
`innerword-assistant`) build and test with `CARGO_TARGET_DIR=target-pure`;
`innerword-print`, `innerword-ui`, `innerword-library`, `innerword-app` in `target/`. No new
target directory, no worktree, no `cargo clean`. One integration-test binary per GPUI crate.
No test reaches the internet (B3's `npm ci` is a build step, not a test). No test of a GPUI
crate makes a `WKWebView`. Every test and command names a temporary library. Commit only
your paths, with the attribution lines. The root `Cargo.toml` gets `innerword-board` (B2) and
`innerword-board-web` (B3) in `[workspace.dependencies]`, each in its package's first
commit. New third-party crates: none (B2 uses `serde`, `serde_json` and `sha2`, in
`Cargo.lock` already; B3 `sha2`). New npm packages: none beyond section 3's (`fontkit` as a
devDependency of the build; Mermaid's packages come with Excalidraw's lockfile, nothing is
added for them).

**Disk space.** The Mac's disk is tight (31 GB free when this was written, and a GPUI test
binary is about 150 MB). **Every package runs `df -h .` in the repository before a large
build** (the first `cargo build` or `cargo test` of a GPUI crate in its session, any build
of `innerword-app`, a release build, `scripts/bundle.sh`, `npm ci`) **and stops when less
than 5 GB is free**: it builds nothing more and reports what `df -h .` printed, the sizes of
`target/` and `target-pure/` (`du -sh`), and the step it was about to take, so that whoever
runs the work frees space; it never deletes a target directory or runs `cargo clean` itself
(CLAUDE.md, "Working rules").

### 9.2 The interfaces

#### B1. `innerword-store`

```rust
// documents.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind { #[default] Text, Page, Board }
impl DocumentKind {
    pub fn as_str(self) -> &'static str;            // "text" | "page" | "board"
    pub fn parse(text: &str) -> Option<DocumentKind>;
    pub fn reference_prefix(self) -> &'static str;  // "doc" | "page" | "board"
    pub fn noun(self) -> &'static str;              // "document" | "page" | "whiteboard"
}

/// The content of a whiteboard: its stored scene (innerword_board::Scene::to_stored) and
/// what the caller derived from it. No title: only create_board and rename set it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoardContent<'a> {
    pub scene: &'a str,
    /// The FileIds the scene names (Scene::stored_files); what the store checks.
    pub files: &'a [FileId],
    pub plain_text: &'a str,
    pub word_count: u64,
    pub char_count: u64,
}

/// The first 16 bytes of the SHA-256 of a row's content, 32 hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentStamp([u8; 16]);
impl ContentStamp { pub fn of(content: &str) -> ContentStamp; pub fn to_hex(&self) -> String; pub fn parse(hex: &str) -> Option<ContentStamp>; }

#[derive(Clone, Debug, PartialEq)]
pub struct BoardSaved { pub meta: DocumentMeta, pub stamp: ContentStamp }

impl Store {
    /// At the end of `folder` (None: the root). Takes the file to schema 5 first when it is
    /// at 4, in the same transaction. NotFound for a missing or trashed folder;
    /// FilesMissing when an id of `board.files` has no row in images.db (nothing written).
    pub fn create_board(&mut self, folder: Option<FolderId>, title: &str, board: &BoardContent<'_>) -> Result<BoardSaved>;
    /// New content; bumps updated_at; never the title. WrongKind on a text or a page;
    /// Stale when `expect` is Some and the row's content has another stamp; FilesMissing
    /// for an id of `board.files` that neither the row nor any snapshot of it names and
    /// images.db lacks. Works on a trashed whiteboard. Nothing written on an error.
    pub fn save_board(&mut self, id: DocumentId, board: &BoardContent<'_>, expect: Option<ContentStamp>) -> Result<BoardSaved>;
    /// Every whiteboard, live and trashed, most recently updated first (covering index).
    pub fn boards(&self) -> Result<Vec<DocumentMeta>>;
}

pub enum StoreError { /* … */
    /// `kind`: what the row is; `saving`: what the caller tried to save over it.
    WrongKind { id: i64, kind: DocumentKind, saving: DocumentKind },
    FilesMissing { ids: Vec<FileId> },   // wording: "the item names …"
    /// The row's content is not the one the caller's scene descends from.
    Stale { id: i64, stored: ContentStamp },
}
// lib.rs: pub use crate::documents::{BoardContent, BoardSaved, ContentStamp};
// schema.rs: V5, SCHEMA_VERSION 5, OPEN_SCHEMA_VERSION 4 (what an open migrates to).
```

Behaviour: `save_document` refuses every kind but `Text`, `save_page` every kind but `Page`,
`save_board` every kind but `Board`. `duplicate_document` copies the kind (as it does). The
shape library (4.15) is the setting `board.shape_library`, written and read with the store's
`set_setting` and `setting` as they are (a `serde_json` raw value): no new function.
`save_view_state` is unchanged (the app writes the stamp into the JSON). `collect.rs`'s
table gets its row. The `WrongKind` patterns elsewhere get `..` (9.1).
Tests: `tests/boards.rs` (create, save, the three kinds' guards both ways, the lazy step to
5 and its rollback, `Stale` on a changed row and nothing written, `FilesMissing` for a new
missing id and nothing written, a missing id the row or a version already named does not
refuse, a save never changes the title, a file named only by a version of a whiteboard
survives the collector, duplicate, trash and restore, search finds a label's words,
`boards()` live and trashed); the schema tests of section 2.
Done: `CARGO_TARGET_DIR=target-pure cargo test -p innerword-store`; `cargo build -p
innerword-app` and `CARGO_TARGET_DIR=target-pure cargo build -p innerword-cli` compile.

**As built** (B1, B1f, B1g): the signatures above, and `ContentStamp` has `Display` too; the
two page functions and the board's share private helpers (`insert_item`, `require_kind`,
`stamp_of`); `board_files_missing(Option<DocumentId>, &[FileId])` checks only the ids it is
given and pre-filters the row's and its snapshots' contents with `instr(lower(content),
hex)` before `scan::files_in`; `schema.rs` has `take_steps_above_open(&Transaction)`, and
`migrate` stops at 4; `open_read_only` and `NeedsUpgrade` use `OPEN_SCHEMA_VERSION` (section
2). The order of refusals, and what a refused save still does in `images.db`, are in
section 2 ("The store's half"). `tests/boards.rs` has 23 tests; the schema's unit tests
add `an_open_migrates_to_4_and_leaves_5_alone`,
`step_5_changes_nothing_and_goes_with_its_transaction`,
`a_build_that_knows_schema_4_refuses_a_library_of_schema_5` and
`the_list_of_whiteboards_does_not_read_the_scenes`. One test of `innerword-agent` asserted
`SCHEMA_VERSION` after an open and now asserts `OPEN_SCHEMA_VERSION`.

#### B2. `innerword-board` (new, pure, depends on nothing of ours)

```rust
pub const EXCALIDRAW_VERSION: &str = "0.18.1";

pub mod limits {
    // For agents (a request over them is refused).
    pub const MAX_SCENE_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_ELEMENTS: usize = 20_000;
    pub const MAX_REQUEST_ELEMENTS: usize = 2_000;
    pub const MAX_TEXT_CHARS: usize = 20_000;
    pub const MAX_ID_CHARS: usize = 64;
    // For the writer's own scene: a note above the first, a recovery file above the second.
    pub const LARGE_VIEW_SCENE_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_VIEW_SCENE_BYTES: usize = 256 * 1024 * 1024;
    pub const PDF_PADDING: f64 = 32.0;
    pub const PDF_MAX_SIDE: f64 = 14_400.0;
    // An agent's pictures (section 7.2): an SVG kept as it came, and the drawn size by default.
    pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
    pub const PICTURE_MAX_SIDE: f64 = 1_440.0;
    // The shape library (section 4.15).
    pub const MAX_LIBRARY_BYTES: usize = 8 * 1024 * 1024;
    pub const MAX_LIBRARY_ITEMS: usize = 1_000;
}

/// 32 lowercase hex digits: the id of stored bytes (innerword_store::FileId as text; this
/// crate does not depend on the store).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileHex(String);
impl FileHex { pub fn parse(text: &str) -> Option<FileHex>; pub fn as_str(&self) -> &str; }

#[derive(Clone, Debug, PartialEq)]
pub struct Scene { /* elements (live only), app_state, files */ }

#[derive(Clone, Debug, PartialEq)]
pub struct Element(serde_json::Map<String, serde_json::Value>);   // every field kept
impl Element {
    pub fn id(&self) -> &str;
    pub fn kind(&self) -> ElementType;
    pub fn x(&self) -> f64; pub fn y(&self) -> f64; pub fn width(&self) -> f64; pub fn height(&self) -> f64;
    pub fn angle(&self) -> f64;
    pub fn version(&self) -> i64;
    pub fn text(&self) -> Option<&str>;
    pub fn container_id(&self) -> Option<&str>;
    pub fn frame_id(&self) -> Option<&str>;
    pub fn file_id(&self) -> Option<&str>;
    pub fn link(&self) -> Option<&str>;
    pub fn json(&self) -> &serde_json::Map<String, serde_json::Value>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElementType { Rectangle, Ellipse, Diamond, Text, Arrow, Line, Freedraw, Image, Frame, Other(String) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry { pub mime_type: String, pub file: Option<FileHex>, pub created: i64 }

pub struct Parsed { pub scene: Scene, pub warnings: Vec<String> }

#[derive(Debug, Clone, PartialEq)]
pub enum SceneError { NotJson(String), NotAScene(String), TooLarge { what: &'static str, limit: usize } }

impl Scene {
    pub fn empty() -> Scene;
    /// The stored form (documents.content of a whiteboard, a snapshot's content).
    pub fn from_stored(json: &str) -> Result<Parsed, SceneError>;
    /// A scene as the view posts it (files without "file"). Lenient: unknown element types
    /// and fields are kept, a value of an unexpected type is kept as it came, lone
    /// surrogates are mended; it fails only on what is not JSON or has no `elements`
    /// array. `stored` maps Excalidraw's file id to the stored bytes and their type (the
    /// type written into `files`); a file it does not know is left out of `files` (its
    /// element stays). The caller has made sure that no file it names is still on its way.
    pub fn from_view(json: &str, stored: &dyn Fn(&str) -> Option<(FileHex, String)>) -> Result<Parsed, SceneError>;
    /// The Excalidraw file ids the scene's elements name (to find those the app does not know).
    pub fn named_file_ids(&self) -> Vec<String>;
    pub fn to_stored(&self) -> String;
    pub fn elements(&self) -> &[Element];
    pub fn element(&self, id: &str) -> Option<&Element>;
    pub fn files(&self) -> &std::collections::BTreeMap<String, FileEntry>;
    pub fn stored_files(&self) -> Vec<FileHex>;
    pub fn is_empty(&self) -> bool;
    pub fn text(&self) -> String;                  // section 2
    pub fn summary(&self) -> BoardSummary;
    pub fn listing(&self, header: &ListingHeader<'_>) -> String;   // section 7.4
    pub fn listed(&self) -> Vec<ListedElement>;    // the JSON form of the listing
    pub fn bounds(&self) -> Option<Bounds>;
    pub fn pdf_size(&self, padding: f64, max_side: f64) -> Option<(f64, f64)>; // the estimate the fakes use
    /// What changed from `before` to `self`: elements added or changed (version or
    /// versionNonce differs), each with its version in `before` as base_version (None:
    /// new); elements gone, by id with their version in `before`; files that are new.
    pub fn delta_from(&self, before: &Scene) -> Delta;
}

/// The base rule of section 4.5, in Rust: for each element, the writer's copy where the
/// writer changed it since `base`, else theirs. Used by the fake canvas of the app's tests,
/// by the app when an adopt failed, and against the glue's table in B4's WebKit case.
pub fn merge(base: &Scene, writer: &Scene, theirs: &Scene) -> Merged;
pub struct Merged { pub scene: Scene, pub conflicts: Vec<String> }

/// Section 2, "Pictures": bytes that are an SVG document (BOM, XML declaration, comments,
/// doctype, then an <svg> root in the SVG namespace; UTF-8; at most 4 MB).
pub fn sniff_svg(bytes: &[u8]) -> bool;
/// Replaces every lone surrogate escape (\ud800 without its pair) in JSON text by \ufffd.
pub fn well_formed_json(text: &str) -> std::borrow::Cow<'_, str>;
pub struct ListingHeader<'a> { pub reference: &'a str, pub title: &'a str, pub folder: Option<&'a str> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardSummary { pub elements: u64, pub by_type: std::collections::BTreeMap<String, u64>, pub texts: u64, pub pictures: u64, pub words: u64, pub bounds: Option<Bounds> }
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListedElement { /* id, type, x, y, width, height, angle, text, label, container, start, end, points, frame, groups, style, link, locked, picture */ }

#[derive(Clone, Debug, PartialEq)]
pub struct Delta { pub upserted: Vec<(Element, Option<i64>)>, pub removed: Vec<(String, i64)>, pub files: std::collections::BTreeMap<String, FileEntry> }
impl Delta { pub fn is_empty(&self) -> bool; pub fn to_json(&self, base_rev: u64, undoable: bool) -> String; }   // what the glue's `adopt` takes (4.5)

// What agents write.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Skeleton(pub serde_json::Map<String, serde_json::Value>);
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementChange(pub serde_json::Map<String, serde_json::Value>);

/// Where new ids, seeds, nonces and times come from (section 7.1): derived, never random.
pub trait Making { fn id(&mut self) -> String; fn seed(&mut self) -> i64; fn nonce(&mut self) -> i64; fn now_ms(&mut self) -> i64; }
/// The n-th value from SHA-256 of the parts and n; `now_ms` as given (the store's clock).
pub struct HashedMaking { /* … */ }
impl HashedMaking { pub fn new(parts: &[&[u8]], now_ms: i64) -> HashedMaking; }
impl Making for HashedMaking { /* … */ }

/// A picture an agent's `image` skeleton names, as the process that was asked made it
/// (section 7.2): keyed by the skeleton's `path` as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture { pub file: FileHex, pub mime_type: String, pub pixels: (u32, u32), pub density: u32 }
pub type Pictures = std::collections::BTreeMap<String, Picture>;

/// An `image` skeleton whose path is not in `pictures` is Invalid (the caller prepares
/// every path first).
pub fn add(scene: &mut Scene, skeletons: &[Skeleton], pictures: &Pictures, making: &mut dyn Making) -> Result<Added, BoardError>;
/// The paths the `image` skeletons name, distinct, in order: what the caller reads first.
pub fn picture_paths(skeletons: &[Skeleton]) -> Vec<String>;
pub fn update(scene: &mut Scene, changes: &[ElementChange], making: &mut dyn Making) -> Result<Updated, BoardError>;
pub fn remove(scene: &mut Scene, ids: &[String], making: &mut dyn Making) -> Result<Removed, BoardError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddedElement { pub id: String, pub r#type: String, pub with: Vec<String> }   // with: the ids of its label
pub struct Added { pub elements: Vec<AddedElement>, pub warnings: Vec<String> }
pub struct Updated { pub changed: Vec<String>, pub warnings: Vec<String> }
pub struct Removed { pub removed: Vec<String>, pub unbound: Vec<String> }
#[derive(Clone, Debug, PartialEq)]
pub enum BoardError {
    Invalid { element: Option<usize>, field: Option<String>, message: String },   // exit 2
    NotFound { id: String, known: Vec<String> },                                  // exit 3
    TooLarge { what: &'static str, limit: usize },                                // exit 2
}

pub mod metrics {   // src/metrics/font-metrics.json (a copy of B3's dist/font-metrics.json), line heights per family
    pub fn line_height(font_family: u8) -> f64;
    pub fn text_size(text: &str, font_family: u8, font_size: f64) -> (f64, f64);
    pub fn wrap(text: &str, font_family: u8, font_size: f64, max_width: f64) -> String;
    pub const METRICS_JSON: &str;   // the copy, include_str!; B3's test compares it with dist/
}

/// Excalidraw's shape library as the setting board.shape_library keeps it (section 4.15).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeLibrary { /* items, each with id, status, elements, created, name */ }
impl ShapeLibrary {
    pub fn empty() -> ShapeLibrary;
    /// The stored form; an item with an element of type image, iframe, embeddable or
    /// magicframe is dropped (a warning in Parsed).
    pub fn from_stored(json: &str) -> Result<ParsedLibrary, SceneError>;
    /// What the glue posts (`items`); lenient as Scene::from_view; TooLarge over the limits.
    pub fn from_view(items_json: &str) -> Result<ParsedLibrary, SceneError>;
    pub fn to_stored(&self) -> String;   // {"type":"excalidrawlib","version":2,"source":"inner-word","libraryItems":[…]}
    pub fn items(&self) -> &[LibraryItem];
}
pub struct ParsedLibrary { pub library: ShapeLibrary, pub warnings: Vec<String> }
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryItem { /* id, name, created, elements */ }
impl LibraryItem {
    pub fn id(&self) -> &str;
    /// Its name, or "Shape 3: rectangle, arrow, text" (the picker's row, 4.15).
    pub fn title(&self, index: usize) -> String;
}

pub mod export_dir {   // what the tool hands its PDF child
    pub const SCENE_FILE: &str = "scene.json";
    pub const FILES_DIR: &str = "_files";
    pub fn write(dir: &std::path::Path, scene: &str, files: &mut dyn Iterator<Item = (FileHex, Vec<u8>)>) -> std::io::Result<()>;
}
```

Tests (one integration binary, `tests/main.rs`, plus unit tests): the stored form round
trips every field of a scene the scout's run made (`board/proto/out-full/scene.json`, copied
in as a fixture, with its pictures' `dataURL`s replaced by `file`s); `from_view` leaves out
unknown files, drops the kinds never kept, keeps an unknown type, an unknown field and a
`null` number, mends a lone surrogate, and fails only on what is not a scene; `text` order;
the listing of 7.4 exactly; `delta_from` (added, changed, removed, files, base versions);
`merge`, every row of 4.5's table; `sniff_svg` (an SVG with a BOM, a declaration and a
doctype; an HTML file; a PNG; an SVG of 5 MB refused); the same input gives the same bytes
twice and a different board id other ids; every skeleton type and field of 7.2, each
refusal with its index and field, the invisibly small ones among them; bindings on rectangle, ellipse and diamond edges; labels
wrapped and shapes grown; frames from children; update's followers (labels, bound ends,
frame children, scaled points); remove's rules; fractional indices ordered; limits; `image`
skeletons (the default size from pixels and density and its 1,440 cap, one of width and
height given, both given, the element's fields and the `files` entry, a path missing from
`pictures` refused, a fill field refused, two skeletons naming one path, `picture_paths`);
`ShapeLibrary` (round trip of a library Excalidraw wrote, an item with a picture or an
`iframe` dropped, the two limits, a lone surrogate mended). Golden
files for the conversion of a corpus of skeletons (`tests/corpus/*.json`), which B11's
WebKit case also reads.
Done: `CARGO_TARGET_DIR=target-pure cargo test -p innerword-board`.

**As built** (B2 and its fix round): everything above with these signatures, and beyond
them: `pub const SOURCE: &str = "inner-word"`; `Scene::listing_with(header, has)` and
`listed_with(has)` (`has` tells whether `images.db` holds a picture's bytes: "picture
missing"), `Scene::app_state()`; `ElementType::{parse, as_str, is_shape, is_linear}`; more
getters on `Element` (`type_name`, `version_nonce`, `name`, `locked`, `index`,
`bound_elements`, `binding`, `points`, `group_ids`, `from_json`); `ListedStyle`;
`metrics::{FAMILIES, family_by_name, family_name, normalize_text, has_widths,
DEFAULT_FONT_*}` and the private `line_width`; `LibraryItem::{name, created, status,
elements}`, `ShapeLibrary::is_empty`; `Display` and `Error` for `SceneError` and
`BoardError`. `BoardSummary` gained `labels` (serde default) and counts `elements` and
`by_type` without labels, as the listing's header does (final review: `info` had said 7
where `get` and `boards` said 5). `LibraryItem::title(index)` takes `index` from 0 and
shows it from 1; `Delta::to_json` gives each file entry only `mimeType` and `file`;
`Removed.removed` holds the labels too and `Updated.changed` what moved with a change (an
elbow arrow left as it was is not among them); `merge` takes the canvas settings from
`theirs` only where the writer left them as they were, and orders elements by fractional
index, renumbering indices that do not grow. The tests: 15 unit and 75 integration tests
after the fix round (`suite/`: scene, svg, delta, add, update, remove, library, corpus,
robust), the corpus in `tests/corpus/*.json` with golden scenes in `tests/corpus/golden/`
and a README for B11 (bless with `INNERWORD_BOARD_BLESS=1`); a time-limited test of 2,000
changes on a large board. `src/metrics/font-metrics.json` is byte for byte the bundle's.

#### B3. `innerword-board-web` (new: the bundle; Rust depends on nothing of ours)

```rust
pub const EXCALIDRAW_VERSION: &str = "0.18.1";
pub const ENTRY: &str = "index.html";
pub const EXPORT_ENTRY: &str = "export.html";
pub const SCENE_PATH: &str = "scene.json";       // the export page's; equal to innerword_board::export_dir::SCENE_FILE (a test)
pub const BOARD_PATH: &str = "board.json";       // the view's: {"stamp", "scene", "view"} (4.3)
pub const LIBRARY_PATH: &str = "library.json";   // the view's: the stored shape library (4.15)
pub const FONT_METRICS: &str = "font-metrics.json";
pub const FILES_PREFIX: &str = "_files/";
pub const BRIDGE_NAME: &str = "innerwordBoard";

#[derive(Clone, Debug)]
pub struct Assets { /* root, the manifest */ }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInfo { pub media_type: String, pub bytes: u64, pub sha256: String }
impl Assets {
    /// Contents/Resources/Whiteboard in an app bundle, else this crate's dist/ (section 3).
    pub fn find() -> Result<Assets, String>;
    pub fn at(root: &std::path::Path) -> Result<Assets, String>;
    pub fn root(&self) -> &std::path::Path;
    pub fn get(&self, path: &str) -> Option<&AssetInfo>;
    pub fn read(&self, path: &str) -> Option<std::io::Result<Vec<u8>>>;   // None: not a file of the bundle
    pub fn paths(&self) -> impl Iterator<Item = &str>;
}

pub mod bridge {
    pub const MAX_MESSAGE_BYTES: usize = 24 * 1024 * 1024;
    #[derive(Debug)]
    pub enum Message {
        Ready { excalidraw: String, elements: u64, stamp: String },
        Scene { rev: u64, scene: Box<serde_json::value::RawValue> },
        ScenePart { rev: u64, index: u32, count: u32, text: String },
        File { id: String, mime_type: String, data_url: String },
        Status(Status),
        Library { lrev: u64, items: Box<serde_json::value::RawValue>, too_large: bool },   // built: too_large, items null
        Help,
        Error { message: String },
    }
    #[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
    pub struct Status {
        pub rev: u64, pub posted: u64, pub editing: bool, pub composing: bool,
        pub pointer_down: bool, pub zoom: f64, pub scroll_x: f64, pub scroll_y: f64,
        pub elements: u64, pub selected: u64, pub too_large: bool,
    }
    impl Status { pub fn holds_more(&self) -> bool; }   // rev > posted || editing || pointer_down
    pub fn parse(body: &str) -> Result<Message, String>;
    // Built: PART_BYTES (16 MB), VERSION (1), START_FAILED ("the whiteboard could not
    // start": an error message that begins with it is a failed start, 4.3), and
    // Message::to_body(), which writes a message exactly as the glue does (the fake engine).
}

pub mod calls {   // bodies for innerword_web::WebPage::call_async / ScriptInput::script
    pub const COLLECT: &str;          pub const FILES: &str;           pub const ADOPT: &str;
    pub const UNDO: &str;             pub const REDO: &str;            pub const SELECT_ALL: &str;
    pub const ZOOM: &str;             pub const SEARCH: &str;          pub const CENTER: &str;
    pub const INSERT_PICTURES: &str;  pub const INSERT_ELEMENTS: &str; pub const STATE: &str;
    pub const MERMAID: &str;          pub const LIBRARY: &str;         // 4.5
    pub const EXPORT_LAYOUT: &str;    // export.html; its argument export_argument()
    pub const CONVERT_SKELETONS: &str; // export.html; the converter's cross-check only (B11)
    pub fn export_argument() -> String;   // {"padding":32,"maxSide":14400}
    #[derive(Debug, serde::Deserialize)]
    pub enum CollectedScene { None, Whole(Box<serde_json::value::RawValue>), Parts }   // "parts": posted as scenePart before the answer
    #[derive(Debug, serde::Deserialize)]
    pub struct Collected { pub rev: u64, pub scene: CollectedScene, pub files: Vec<CollectedFile>, pub library: Option<CollectedLibrary>, pub status: super::bridge::Status }
    #[derive(Debug, serde::Deserialize)]
    pub struct CollectedLibrary { pub lrev: u64, pub items: Box<serde_json::value::RawValue> }
    #[derive(Debug, serde::Deserialize)]
    pub struct LibraryAnswered { pub lrev: u64, pub added: Option<String>, pub refused: Option<String> }
    #[derive(Debug, serde::Deserialize)]
    pub struct CollectedFile { pub id: String, pub mime_type: String, pub data_url: String }
    #[derive(Debug, serde::Deserialize)]
    pub struct FilesPosted { pub posted: Vec<String>, pub missing: Vec<String> }
    #[derive(Debug, serde::Deserialize)]
    pub struct Adopted { pub rev: u64, pub same: bool, pub conflicts: Vec<String> }
    #[derive(Debug, serde::Deserialize)]
    pub struct Center { pub x: f64, pub y: f64, pub selected: Vec<String> }
    // Built: EXPORT_PADDING, EXPORT_MAX_SIDE; argument builders files_argument,
    // steps_argument, zoom_argument(Zoom), search_argument, insert_pictures_argument(
    // &[PictureToInsert], at), insert_elements_argument, mermaid_argument,
    // library_argument(&LibraryAction), convert_skeletons_argument; answer<T>(); and the
    // answers Revised, Zoomed, Inserted, ExportSize, LibraryAnswered (with ids).
}

pub mod keys {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BoardKey { pub keys: &'static str /* GPUI keystroke: "cmd-d" */, pub what: &'static str /* "Duplicate" */ }
    pub const WHITEBOARD_KEYS: &[BoardKey];   // section 4.7's table: 43 keys
    // Built: TOOL_KEYS_ROW, is_whiteboard_key; the doc says to bind every spelling
    // innerword_keymap::Keystroke::spellings gives (a test: only cmd-shift-, and cmd-shift-.).
}
// Built, beside the paths: MANIFEST, NOTICES, BUNDLE_FOLDER ("Whiteboard"); Assets::find_for
// (executable); build.rs compiles no dist/ path into a build with INNER_WORD_BUNDLED.
```

JavaScript (`web/src/glue/`): the page's half of 4.3 to 4.15 and of section 5 (the export
page), including the base rule of `adopt` (4.5), the deletion of the kinds never kept, the
parts of a large scene, the well-formed text of every message, the tooltips' keys (4.7),
`help`, the Mermaid dialog kept and the embed item removed by its label (4.13), and the
shape library (`library.json` at load, the `library` post, the `library` call, its
unreachable parts hidden; 4.15: `glue/library.js`). B3's first milestone, **committed early for B2 and B4**: the prototype's glue
reshaped to this contract (the bridge's messages, `collect`, `adopt`, `undo`,
`exportLayout`), `dist/` built with the patches, its fonts, `font-metrics.json` and
`MANIFEST.json`. Then the rest (autosave timing, status, links, pictures, CSS, main menu,
welcome screen, `convertSkeletons`, the shape library, notices with `bundled-inside.json`).
Tests (`tests/main.rs`, no node, no WebKit): every file of `dist/` is in `MANIFEST.json` with
its size and sha256, and nothing else; `BUILD-INFO.json`'s hashes equal the inputs under
`web/` (a source changed without a build fails `cargo test`); the media types; `NOTICES.txt`
names every package of the metafile and every font folder; `Assets::find` outside a bundle
finds `dist/`, and inside a made-up `X.app/Contents/MacOS/` only `Resources/Whiteboard`;
`NOTICES.txt` also names every package of `web/bundled-inside.json` (langium, chevrotain
inside `@mermaid-js/parser`) and gives DOMPurify under Apache-2.0; the Mermaid chunks are in
`MANIFEST.json` and no file of `dist/` is a stub of Mermaid;
`bridge::parse` of every kind (`library` among them) and of the refusals; `WHITEBOARD_KEYS` parse as GPUI keystroke
text (that they are none of the window's keys is the app's test, 6.8); the two
`EXCALIDRAW_VERSION`s and `SCENE_PATH`s agree (dev-dependency on `innerword-board`'s first
commit); `innerword_board::metrics::METRICS_JSON` equals `dist/font-metrics.json`; no HTML
file of `dist/` has an inline script or an inline event handler (the whiteboard's policy
allows neither).
Done: `node build.mjs --check` passes and `CARGO_TARGET_DIR=target-pure cargo test -p
innerword-board-web` passes.

**As built** (B3 and its fix round): 46 tests (`suite/`: assets, bridge, bundle, calls,
keys); `NOTICES.txt` is held against `BUILD-INFO.json`'s `metafilePackages` and
`carriedInside`, written straight from esbuild's metafile and the source maps (the first
test held it against a list made from the same entries as the notices, which proved
nothing); the no-eval test counts the allowed occurrences exactly (section 3). The glue
departs from the spec where 4.3, 4.4 and 4.5 say "Built": the `same` rule, the restore
with copies, every wait before the merge, `START_FAILED`, `tooLarge`, `state` outside the
queue, `postNow({force})` on `visibilitychange` and `pagehide` (a pasted picture still on
its way no longer delays those posts), the fonts loaded through `exportToCanvas`, and the
keys it keeps from Excalidraw (4.7). `library.json` stays optional in the page (B4's cases
serve none); the app always serves it. B3's six real-WebKit cases of the fix round were
written in a scratch Swift harness and ported into `innerword-web`'s program by B11f.

#### B4. `innerword-web`

```rust
/// A whiteboard's view: the one script message handler (section 4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bridge { pub name: &'static str, pub max_bytes: usize }

// ViewConfig: unchanged.

/// What only a whiteboard's view has. Default: a page's view.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewExtras {
    pub bridge: Option<Bridge>,
    /// No main-frame navigation after the first load; no `alert` toast.
    pub one_document: bool,
    /// The policy of every answer, in place of CONTENT_SECURITY_POLICY.
    pub csp: Option<&'static str>,
}
pub const BOARD_CONTENT_SECURITY_POLICY: &str;          // 4.2
pub const BOARD_EXPORT_CONTENT_SECURITY_POLICY: &str;   // 4.2, with 'wasm-unsafe-eval'
// Built: BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD ('unsafe-eval'),
// WASM_KEYWORD_SINCE_WEBKIT = 614, pub fn board_export_policy() -> &'static str (the one
// every caller passes), MAX_PDF_SIDE = 14_400.0, check_with_policy and
// check_blocking_with_policy, testing::live_bridges().

pub trait PageEvents: 'static {
    // … every method of today …
    /// A message of the bridge that passed every check: only posts.
    fn message(&self, _body: String) {}
}

impl WebPage {
    /// As `new`, with what only a whiteboard's view has; `new` is `new_with(.., Default)`.
    pub unsafe fn new_with(gpui_view: *mut c_void, config: ViewConfig, extras: ViewExtras, events: Rc<dyn PageEvents>) -> WebPage;
    /// callAsyncJavaScript in the page world with `arg` = argument_json; `done` on the main thread.
    pub fn call_async(&self, body: &str, argument_json: &str, done: impl FnOnce(Result<String, String>) + 'static);
    /// Out of sight, still drawn (4.2).
    pub fn park(&self);
    // Built: pub fn is_parked(&self) -> bool; set_frame brings a parked view back (4.2);
    // Drop removes every script message handler.
}

pub struct ScriptInput {
    pub source: Arc<dyn PageSource>,
    pub entry: String,
    pub script: String,          // a body for callAsyncJavaScript, with `arg`
    pub argument_json: String,
    /// The script answers {"width","height"}: make a one-page PDF of (0,0,ceil(w),ceil(h)).
    pub pdf: bool,
    pub csp: &'static str,
    pub limit: Duration,
    pub rules_dir: PathBuf,
}
pub struct ScriptOutput { pub value: String, pub pdf: Option<PrintedPdf> }
pub fn run_script(input: ScriptInput, done: impl FnOnce(Result<ScriptOutput, String>) + 'static); // main thread
pub fn run_script_blocking(input: ScriptInput) -> Result<ScriptOutput, String>;                    // a program

pub struct ViewStats { /* … */ pub messages_received: u64, pub messages_dropped: u64 }
```

Behaviour: section 4.2; the guard's new selectors. Tests (`tests/webkit.rs`, the program of
its own; dev-dependency `innerword-board-web`):

- a bridge on a small inline page: a message arrives; one from a subframe, from another
  origin, too large or not a string is dropped and counted; a page's view has no
  `messageHandlers.innerwordBoard`; the handler goes with the view (no leak over 20 views);
- `call_async`: an awaited promise's value, an exception's message, arguments with quotes
  and line breaks arriving as they were;
- `one_document`: a `location =` to the view's own other file is cancelled after the first
  load; an `alert` makes no event;
- the board policy: an inline script, an `eval`, a `srcdoc` frame with a script and a
  `data:` frame do not run (each would post a message); the export policy runs
  WebAssembly;
- `park`: a parked view still runs `requestAnimationFrame`; a call answers from a parked
  view and from a hidden one;
- `run_script_blocking` with `pdf`: one page of the rect, content beyond the view's bounds,
  text extractable, a fractional size rounded up;
- **the bundle** (`Assets::find()`, B3's `dist/`), with the board policies: loaded through
  `check_blocking` with the 127.0.0.1 server watching: nothing blocked, nothing missing
  (Xiaolai's 404s only if patch 2 could not be applied), no CSP report, no script error; a
  view loads a stored scene and posts `ready` with its stamp; a synthetic edit posts
  `scene`; a scene of 20 MB comes as `scenePart`s; `collect` answers it; **`adopt` holds
  every row of 4.5's table** (an element moved by the writer after the base stays the
  writer's and is named in `conflicts`; one untouched takes the incoming copy at its own
  version; a removal of an untouched element deletes it; `same` true only without
  conflicts and without unposted changes), and a dispatched `cmd-z` (through `undo`) takes
  it out and `redo` puts it back; a pasted `iframe` element with a `srcdoc` script runs
  nothing and is gone from the scene; the toolbar's tooltips name no key of `keys.js`'s
  list; `?` posts `help`; the export page makes the PDF of the scout's scene (one page, its
  size the SVG's rounded up, its text has "Draft chapter" and "你好" falls back if Xiaolai
  is out, its fonts embedded);
- **Mermaid under the whiteboard's policy** (answer 4; the scout ran it under a page's,
  which allows eval): the `mermaid` call opens the dialog, and a flowchart, a sequence
  diagram and a pie chart converted through it, and a flowchart pasted as text, make
  elements (the pie one SVG picture, posted as a `file` the app would take by
  `sniff_svg`), with no CSP report, no script error and no request reaching the 127.0.0.1
  server; a flowchart with an image node naming that server's address makes a CSP report
  for `img-src` and no request; a label with `<img src=x onerror=…>` that would post a
  message posts none; the extra tools' menu has "Mermaid to Excalidraw" and no web embed.
  Should the policy refuse something Mermaid needs after all, risk 11 says what is done;
- **the shape library**: a view served a `library.json` of two items shows them in the
  sidebar; "Add to library" on a selection posts `library` with three; the `library` call
  inserts an item at the middle of the view and refuses a selection with a picture with
  Excalidraw's message; `collect` brings a change posted not yet; "Browse libraries",
  "Publish library", Load and Export are not shown, and no request leaves.

Done: `CARGO_TARGET_DIR=target-pure cargo test -p innerword-web` (the program and the unit
tests).

**As built** (B4, B11, B11f): the cases are in `tests/webkit/board.rs`, `bundle.rs` and
`crosscheck.rs` (run one alone with `INNERWORD_WEB_ONLY=<case>`: `bridge`, `park`,
`bundle`, `bundle_glue`, `crosscheck`), and beyond the list above: a message exactly at the
cap arrives and one over it only by its UTF-8 bytes is dropped; a lone surrogate in a post
and in a call's answer arrives as U+FFFD; 20 MB each way through `call_async` (about 17 ms);
`one_document` also cancels a link click and `location.reload()` while the app's reload
works; `frames_never_reach_the_bridge` (four views: with the bridge no frame of any kind
loads, under either policy; without it, the page policy runs every frame and the
whiteboard's runs a `srcdoc` that names its own script, written as a note); `park` gives
back the hidden state and the occlusion detection it found; a PDF answer of 1e7 × 200.5
comes out 14,400 × 201; risk 12 with a Rust-made text whose version Excalidraw raises at
load (an index out of order) taken at version 3 without a conflict; Xiaolai's 404s fail
unless `BUILD-INFO.json` says its patch was not applied; the six glue cases of B3's fix
round (`bundle_glue`: an adopt while a font load is held, in the middle of a freehand
stroke, as a stroke begins, while a new text is typed; a failed start with `state`
answering behind a waiting `collect`; a library over 25 MB), which fail 15 checks against
the bundle before the fix; and the cross-check (B11). The final review added a picture
longer than one message and what a paste brings (an `smb:` link, a `customData` nested 130
deep).

#### B5. Small additive changes

```rust
// innerword-print/src/system.rs
pub struct PrintRequest<'a> {
    pub pdf: &'a Pdf,
    /// None: the user's paper (NSPrintInfo.sharedPrintInfo). With DownToFit, the paper,
    /// given or the user's, is turned landscape when the PDF's first page is wider than high.
    pub paper: Option<(f32, f32)>,
    pub scaling: Scaling,
    pub title: &'a str,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scaling { #[default] Actual /* kPDFPrintPageScaleNone, today's */, DownToFit /* kPDFPrintPageScaleDownToFit */ }
// innerword-app/src/printing.rs, PrintJob::request: `paper: Some(self.paper), scaling: Scaling::Actual`
// (B8 then adds `pub enum Fit { Actual, ShrinkOntoPaper }` and `PrintJob::fit`, mapped in
// `request()`: Actual -> (Some(paper), Scaling::Actual); ShrinkOntoPaper -> (None, DownToFit)).

// innerword-print/src/pdf.rs (a folder's PDF with its whiteboards, section 5)
impl PdfWriter {
    /// Adds page `page` (from 1) of the PDF `pdf` as a page of its own, of the size a
    /// reader shows for it (its crop box, turned by /Rotate; built, where the spec said its
    /// media box), and the page is drawn into it with CGContextDrawPDFPage, so its paths,
    /// its text with its embedded fonts and its pictures stay what they are (not its link
    /// annotations). None (and nothing added) for bytes that are no PDF, page 0, a page it
    /// does not have or a page of no size; else the size.
    pub fn add_pdf_page(&mut self, pdf: &[u8], page: usize) -> Option<(f32, f32)>;
}
// Report gains `embedded_pages: usize` (counted in `pages` too), and `Report::line` names
// them ("2 whiteboards") when there are any; rows, fonts and images count the writer's
// own pages only (built).

// innerword-ui/src/icon.rs
IconName::Whiteboard   // "whiteboard", icons/whiteboard.svg, the set's style (a test as the globe's)

// innerword-assistant/src/context.rs
pub enum OpenKind { #[default] Text, Page { source_url: Option<String> }, Board }
pub const BOARD_PREFACE: &str = "The open item is a whiteboard: a drawing of shapes, arrows, text and pictures. It is changed only by its owner, in the app. Its elements, one per line, with their ids, positions, sizes and text:";
// OpenDocument::reference() is "board:<id>" for a board; the long listing goes as its
// beginning and its end, as a page's text; prompts.rs says what a whiteboard is.
```

Tests: `innerword-print`'s `tests/save_job.rs` prints a PDF of a 1,600 × 900 pt page with
`paper: Some(A4)` and `DownToFit` (the page fits the paper, turned landscape; never `None`,
which would depend on the Mac's default printer) and a text's job as before; `add_pdf_page`
(`tests/` of the crate's one integration binary): a PDF of two pages of different sizes
made by `PdfWriter` with text rows, a picture and strokes, one of its pages added to a new
writer after a text page (built: in the unit tests, `src/tests.rs`, since the crate's only
`tests/` program is `save_job`; and a turned page, `a_page_is_added_as_a_reader_shows_it`): the new PDF has two pages, the added one of the source's size,
the same text (`PdfFile::page_text`) and the same pixels (`page_pixels`), and the text is
still text (`PdfFile::find` locates a word), and bytes that are no PDF add nothing; the
icon's test;
the assistant's prompt and context tests with a board. **Built:** `OpenKind::is_board()`
and `is_text()` beside `is_page()`; a whiteboard's tag has `kind="board"` and no
`source=`; an empty listing is "(The whiteboard is empty.)"; a listing over 180,000
characters goes as its beginning and its end, and with tools the assistant is told to read
the rest with `read_document` ("board:<id>"); the quick chat's prompt does not mention
whiteboards. Done: `cargo test -p innerword-print`,
`cargo test -p innerword-ui`, `CARGO_TARGET_DIR=target-pure cargo test -p innerword-assistant`,
`cargo build -p innerword-app`.

#### B6. `innerword-agent`

```rust
// types.rs
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateBoardRequest {
    #[serde(default)] pub title: String,
    #[serde(default)] pub folder: Option<Target>,
    #[serde(default)] pub create_folders: bool,
    #[serde(default)] pub placement: Option<Placement>,   // built: as CreateDocumentRequest has it
    #[serde(default)] pub elements: Vec<innerword_board::Skeleton>,
    /// Filled by Prepare in the process that was asked, never by a caller (7.6).
    #[serde(default)] pub pictures: Vec<BoardPicture>,
    /// The folder a relative `path` is taken from (`--file`'s); None for MCP.
    #[serde(default, skip_serializing)] pub base_dir: Option<PathBuf>,
    #[serde(default)] pub write: WriteOptions,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddToBoardRequest {
    pub board: Target,
    pub elements: Vec<innerword_board::Skeleton>,
    #[serde(default)] pub pictures: Vec<BoardPicture>,
    #[serde(default, skip_serializing)] pub base_dir: Option<PathBuf>,
    #[serde(default)] pub write: WriteOptions,
}
/// A picture file an `image` skeleton names, read and made where the request was asked
/// (attach.rs, `board_picture`): an SVG as it came, else innerword_image::prepare's bytes.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]   // Debug without the bytes, as AttachedImage's
#[serde(default)]
pub struct BoardPicture {
    pub source: String,              // the skeleton's `path` as written
    pub file: String,                // FileId hex of `data`
    pub mime_type: String,           // image/png, image/jpeg, image/gif, image/svg+xml
    pub pixels: (u32, u32),
    pub density: u32,                // 72 when the file says nothing (and for an SVG)
    pub untouched: bool,             // built: stored as it came (the output's "as it came")
    #[serde(with = "base64_bytes")] pub data: Vec<u8>,   // emptied once stored
    #[serde(skip)] pub stored: bool, // built: never travels, as Blob.stored; the listener's mark
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateBoardRequest { pub board: Target, pub changes: Vec<innerword_board::ElementChange>, #[serde(default)] pub write: WriteOptions }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoveFromBoardRequest { pub board: Target, pub ids: Vec<String>, #[serde(default)] pub write: WriteOptions }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardChanged {
    pub board: DocumentSummary,
    pub added: Vec<innerword_board::AddedElement>,
    pub updated: Vec<String>,        // built: not `changed`, which the flattened Written has
    pub removed: Vec<String>,
    pub unbound: Vec<String>,
    pub pictures: Vec<PictureAdded>, // built: { id, source, mime_type, pixels, untouched, width, height }
    pub version: Option<VersionInfo>,
    pub warnings: Vec<String>,
    #[serde(flatten)] pub written: Written,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardInfo { pub summary: innerword_board::BoardSummary, #[serde(skip_serializing_if = "Option::is_none")] pub elements: Option<Vec<innerword_board::ListedElement>> }
// DocumentText.board, DocumentInfo.board: Option<BoardInfo> (skip when None)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardListed { pub board: DocumentSummary, pub elements: u64, pub pictures: u64, pub trashed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub unreadable: Option<String> }   // built (B6f)
// Target::parse reads "board:<id>" and "whiteboard:<id>" as a DocumentId.
// Built: DocumentSummary::is_board() and reference() ("doc:", "page:" or "board:<id>");
// Candidate.reference: Option<String> (B6f); DocumentText no longer derives Eq (floats).

// library.rs; built: as `pages` takes its request (`BoardsRequest { folder }`)
impl Library { pub fn boards(&self, request: &BoardsRequest) -> Result<BoardsList>; }   // BoardsList { boards }

// checker.rs
pub trait PageChecker: Send + Sync {
    fn check(&self, bundle: &innerword_page::Bundle) -> Result<innerword_page::CheckReport>;
    fn print(&self, dir: &Path, out: &Path) -> Result<()>;
    /// The PDF of a whiteboard: `dir` was written by innerword_board::export_dir::write.
    /// ProcessChecker (this crate, beside its `print`) spawns `innerword __board-pdf <dir>
    /// <out>` (the child itself is B9's, in innerword-cli); StaticChecker answers a fixed PDF.
    fn print_board(&self, dir: &Path, out: &Path) -> Result<()>;
}
pub const BOARD_PDF_ARGUMENT: &str = "__board-pdf";
pub const BOARD_PDF_KILL_MS: u64 = 60_000;

// error.rs
Error::WrongKind { id: i64, name: String, kind: DocumentKind, wanted: Option<DocumentKind>, export: bool }   // exit 15; a page's sentences unchanged
pub const BOARD_IS_PDF_ONLY: &str = "a whiteboard is exported as a PDF only";
// Built: Error::EmptyBoard { id, name } (exit 9), ItemKind::Element (exit 3).

// live/mod.rs
pub const PROTOCOL_VERSION: u32 = 4;
// Unchanged signatures; they now also do a whiteboard's pictures (7.6):
pub fn store_page_files(request: &mut Request, files: &mut innerword_store::ImageFile) -> std::result::Result<(), Error>;
pub fn carries_page_files(request: &Request) -> bool;
```

Behaviour: section 7 (`boards.rs` mirrors `pages.rs`: the four operations, the reads, the
export; `content::Reason::Drawing` and `Removal`; `export.rs` leaves whiteboards out of a
folder (its formats are Markdown, DOCX and plain text); the exhaustive matches of 7.7; the
pictures of 7.2 and 7.6: `impl Prepare` for `CreateBoardRequest` and `AddToBoardRequest`,
`board_picture`, `store_page_files` for pictures, and in `run` the pictures stored with
`Store::put_file` before the scene). Every operation makes its values with
`HashedMaking` seeded by the board's id, the stored scene's stamp, the request's
skeletons or changes as JSON and its pictures' `file` ids (7.1), and its time from the store's clock (7.1), in `run` and in `live::execute` alike, so the
two runs of the suite store the same rows and a dry run names the ids the real run gives.
The `WrongKind` literals of `innerword-mcp`'s tests and of `tests/suite/pages.rs` follow
the new fields (7.7).
Tests: `tests/suite/boards.rs`, run both ways (directly and `THROUGH_THE_APP`): every
operation and read, every refusal and exit code, the versions and their labels, dry runs
that write nothing, the PDF export through `StaticChecker` (a fixed PDF) and refused for an
empty whiteboard and for `.md`, a folder export that leaves a whiteboard out with the
warning; a skeleton that would be invisibly small refused; a dry run and the real run name
the same ids; **pictures** (files made by `innerword_image::fixtures` in the test's
temporary folder): a PNG stored as it came and a HEIC or TIFF converted, both named by
their `FileId` in the scene and in `images.db` `files` before the scene names them, in both
runs with the same rows; a relative path from `base_dir`; an SVG kept as it came; an HTML
file named `.svg` and a missing file refused (exit 2, nothing written); 33 MB of pictures
refused; a dry run reads and stores nothing; a request that comes with `pictures` of its
own has them replaced by what `Prepare` reads; with the switch off the stand-in's listener
refuses a request with pictures and stores nothing; `READERS` gets `boards`; the protocol
test of a version-3 stand-in (a request with a picture among them); the operations table's
test.
Done: `CARGO_TARGET_DIR=target-pure cargo test -p innerword-agent` (both runs).

**As built** (B6, B6f and the final review): `boards.rs` holds the four operations (each
keeping its version first: "Before a drawing / an edit / a removal by X"), `Library::boards`
and the board branches of read, info, find (`Occurrence.element`), `read_version`,
`restore_version`, rename and export; the agent's picture routine is `board_picture`,
`prepare_pictures` makes one picture per distinct path; the checker's fake
(`StaticChecker`, `INNER_WORD_PAGE_CHECK=fake`) writes one empty page of
`Scene::pdf_size(32, 14400)`. `tests/suite/boards.rs` (16 tests, both runs) has, beyond the
list above, a damaged whiteboard listed and printed as it is stored, a hint for each kind
with its own prefix, and one file named by two skeletons carried once; the pictures are
written by the test (a TIFF made with `sips`), not by `innerword_image::fixtures` alone. The
operations table has 30 rows; `READERS` has `boards`. The final review left out a change
that sets what an element has (no version, "would change nothing"), counted one element
count everywhere, and judged each change of `update_board` against the drawing just before
it (7.3).

#### B7. `innerword-library`

```rust
pub use innerword_store::{BoardContent, BoardSaved, ContentStamp};
impl Library {
    /// As create_page: DocumentCreated, TreeChanged.
    pub fn create_board(&mut self, folder: Option<FolderId>, title: &str, board: &BoardContent<'_>, cx: &mut Context<Self>) -> Result<BoardSaved>;
    /// As save_page: DocumentSaved (a save never changes the title, so no TreeChanged).
    pub fn save_board(&mut self, id: DocumentId, board: &BoardContent<'_>, expect: Option<ContentStamp>, cx: &mut Context<Self>) -> Result<BoardSaved>;
}
// panel: actions!(library_panel, [ …, NewBoard ]) — "Create a whiteboard in the selected
// folder and name it in place."
impl LibraryPanel {
    /// What New Whiteboard stores (an empty scene). Without it the panel's New Whiteboard
    /// items are not shown and NewBoard propagates (the app has no engine).
    pub fn set_new_board_content(&mut self, scene: impl Into<String>);
    /// Makes a whiteboard at the end of the selected folder and starts naming it in place;
    /// once named (or escaped) emits OpenDocument { id, focus_editor: true }.
    pub fn new_board(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<DocumentId>;
}
// rows, trash rows and the drag ghost carry `kind: DocumentKind` instead of `page: bool`;
// pub fn item_icon(kind: DocumentKind) -> IconName  (Board: IconName::Whiteboard)
//   built: one argument; no kind of item is drawn open or closed, only a folder, which is
//   no DocumentKind and keeps its icons through the private row_icon
// menus: "New Whiteboard Here" (folder row), "New Whiteboard" (the library's menu, the + menu)
// Built: LibraryPanel::makes_boards(); pub fn new_board_command() -> CommandDescriptor
// ("New whiteboard", the library's one CATEGORY, not in commands()); pub const
// TREE_KEY_CONTEXT ("LibraryPanel && mode == tree && !TextField"); the drag ghost has a
// debug selector library-drag-ghost-<icon name>; set_reveal_handler's doc names new_board.
```

`LibraryPanelEvent` gets no variant (the app's match stays whole). Tests (`tests/panel/`):
create and save emit their events; a whiteboard's row has its icon (live, open, trashed,
dragged); the three menu items are there with content set and not without; New Whiteboard
names in place and opens with `focus_editor: true`; escape opens it "Untitled".
Done: `cargo test -p innerword-library`.

**As built** (B7, B7f): the rename's end is an internal `RenameEnd { Submit, Cancel,
Elsewhere }` (`commit_rename` and `cancel_rename` keep their signatures): `enter` takes the
name and opens with `focus_editor: true`, `escape` leaves "Untitled" and opens the same,
the keyboard going elsewhere keeps the name and opens with `focus_editor: false`, and a
whiteboard trashed while it is named ends the naming at once and does not open; new
documents behave as before. `save_board` updates the cached entry and emits
`DocumentSaved { title_changed: false }` only; `Stale`, `WrongKind` and `FilesMissing` emit
nothing. Without the scene `NewBoard` passes the action on and makes nothing. The tests
(`tests/panel/boards.rs`, eight) add a hidden sidebar shown for New Whiteboard (and one the
host does not show, which ends the naming after the off-screen grace), the trashed case,
and the library's command for the list.

#### B8. The whiteboard in the app (`innerword-app`, wave 2)

**First milestone (commit it):** in the real binary, a whiteboard made with `board:new`,
opened, drawn on with `board:draw:` and `board:press:`, saved (switch to a document and
back: the strokes are there), with the keys and the Edit menu of section 4.7 as far as a
scripted run reaches them. **The person checks are the user's, at the end** (answer 7):
the user does every check of section 10 in one sitting once B10 is done, and no package
waits for them; B8 builds the keys as section 4.7 says and writes, in its report, what a
scripted run could not see. A failure of check 1 then goes back to the design (section
10, risk 1, whose fallbacks are ready to be taken).

Then: sections 4 (the app's half: the compare-and-swap saves, the picture map and its
seed, the recovery file, `end_then` and `quit::request` with the signals' change, the
parked view, Reload whiteboard; the shape library of 4.15: `ShapeLibraryState`,
`SaveJob::ShapeLibrary` (built: `Saver::work_shape_library`), `library.json`, its three commands and the picker; "Mermaid to
drawing…"), 5 (Export as PDF, the picker's row, Print with `Fit`, **a folder's PDF with its
whiteboards** through `PdfWriter::add_pdf_page`, the folder's other exports leaving
whiteboards out), 6.1 to 6.5 and 6.8 to 6.10;
`workspace::NewBoard` making a whiteboard through `Library::change_outside(|store|
store.create_board(…))` and opening it (B10 puts the naming in); the six Add commands; the
empty state's hint; `boards.rs` (`LibraryBoardSource`, `board_host`, the bridge's events in
`PageEvent`); the fixed `NoAction` bindings, appended after the defaults in `install` and
`measure_install`; the menu bar without `WHITEBOARD_KEYS` while a whiteboard is open; the
list's fixed rows and question; `quick_open.rs` and `library_search.rs` (icon, detail, a
hit opens and searches); the two counts of `tests/suite/pages.rs`. Internal shape: section
6.1. Tests: section 6.10's B8 part. Done: `cargo test -p innerword-app` passes; the
scripted checks of 9.4 that need no agent pass in the debug build (the person checks of
section 10 are the user's, at the end).

**As built** (B8 and its fix round; the "Built" notes of sections 4 to 6 say where it
departed): `boards.rs` (`BOARD_VIEW`, `SHAPE_LIBRARY_SETTING`, `VIEW_COLLECT_PATIENCE`,
`ADOPT_PATIENCE`, `board_host`, `not_shown_sentence`, `assets(cx)`, `prepare_picture`,
`picture_type`, `data_url_bytes`, `data_url_of`, `drawn_size`, `WhiteboardKeys::fixed_bindings`,
`key_spellings`, `is_whiteboard_keys`, `is_fixed_binding`, `fake_board_pdf`,
`stored_library`, `recovery_dir`, `BoardCell`, `SharedCell`, `LibraryBoardSource` (serving
the bundle, `board.json`, `library.json`, `scene.json` and `_files/<hex>`), `FakeBoard`);
`workspace/board.rs` (`BoardState`, `Picture`, `Held`, `Recovered`, `ClosingBoard`,
`ShapeLibraryState`, `End`; the actions of the `board` namespace; `open_board`,
`new_board(title)`, `collect_board(id) -> Task<bool>`, `adopt_outside_write(id)`,
`draw_on_board`, `insert_board_pictures`, `end_then(End)`, `board_holds_more`,
`describe_board`, `boards_closing()`, the public `board_save_label`,
`board_elements_label`, `board_listing_text`; `ShapeLibraryDelegate`, the picker);
`workspace/board_pdf.rs` (the PDF from the stored scene, WebKit or the fake);
`board_steps.rs`; `quit.rs` (`request(cx)`, deferred); `printing.rs` (`Fit`,
`PrintJob::fit`); `menus.rs` (`OpenItemKind`, `open_item`, `set_open_item`,
`menu_key_bindings(keymap, open)`, `keeps_its_key_on_a_board`); `keymap.rs` (`reinstall`);
`export.rs` (`left_out_sentence`, `ExportDelegate::for_board`); `actions.rs` (`NewBoard`,
`board_commands()`, `CATEGORY_WHITEBOARD`); `shortcuts.rs` (`Place::Board`,
`BOARD_KEY_NOTE`, `TOOL_KEYS_NOTE`, the fixed rows); `workspace/shortcuts.rs`
(`BOARD_TAKES_KEYS_FIRST`, the question, `ALSO_FIXED_NOTE`); `saver.rs` (`BoardSaveJob`,
`work_board`, `work_shape_library`, `WrittenSlot`, `view_state_at`). Library writes went
through `Library::change_outside`, not B7's methods. Two bugs its own new tests found:
`cmd-q` skipped the whiteboard (now `cx.defer`, 4.6), and Add Arrow between two shapes
never worked (now `x`/`y`, 6.2).

#### B9. Tool, MCP server, skill (wave 3)

- `innerword-cli`: the commands of 7.4 in `spec.rs` and `run.rs`, the outputs, the hints, the
  overview's limit, the exit code 15 wording; `src/board_pdf.rs`: `innerword __board-pdf <dir>
  <out.pdf>` handled in `main` before the command line is read (beside `page_check::run`),
  not in the help: `available()`, `Assets::find()`, a `PageSource` over the assets and `dir`
  (`BoardDirSource`), `run_script_blocking` with `EXPORT_ENTRY`, `calls::EXPORT_LAYOUT`,
  `export_argument()`, `pdf: true`, `csp: board_export_policy()` (built: the policy that
  fits the WebKit it runs on, 4.2), a 50 s limit, the bytes to `out`; failures as one line on stderr, exit 1. (B6's
  `ProcessChecker::print_board` spawns it.) The hints of `main.rs` (`hint_in`: the
  whiteboard's sentences beside the page's " is a page: ") and exit code 15's words in
  `help.rs` (line 111: "a page or a whiteboard where a document is needed, or the
  reverse"). **Pictures** (7.2, 7.4): `board new` and `board add` set the request's
  `base_dir` to the folder of `--file` (as `create --file` does for Markdown pictures), the
  output names each picture as 7.4 shows; the command line has no way to give `pictures`.
- `innerword-mcp`: the tools of 7.5, `args.rs`'s array helpers, `render.rs`, `tool_text`,
  `tool_error_text`; the descriptions of `create_board` and `add_to_board` with the
  picture's sentence, `pictures` refused as an argument; the count test (`tools.rs`, 37 to
  41; the read-only 12 stay).
- `innerword-agent-setup`: the skill's paragraph and exit code line (7.8), its tests.
- `innerword-assistant`: `claude/args.rs`'s count (`allowed + denied`, 37 to 41; `allowed`
  stays 12).
- The counts of 37 tools in `innerword-cli`'s tests: `tests/suite/mcp.rs` (two) and
  `tests/suite/pages.rs` (one), to 41.
Tests: `innerword-cli`'s suite with `INNER_WORD_PAGE_CHECK=fake` (every new command, text and
`--json`, dry runs, exit codes 2, 3, 9, 15, the version label, `export board:… x.pdf`, the
stand-in app for every writing command and every new operation name: the coverage tests of
`tests/suite/live.rs`; `board add` with a picture against the stand-in app, whose listener
stores it before the request is queued, and the same with the switch off: exit 5, nothing
stored; a relative path from `--file`'s folder; MCP `add_to_board` with a whole path and
refused with a relative one); **one case with the real child** (`__board-pdf` of the built tool, the
real bundle, a scene whose extremes are shapes and whose texts lie well inside them, so that
Rust's estimate of text widths cannot move the bounds: one page, its size the drawing's and
the margin within 2 pt of `Scene::pdf_size`, its text has the labels; a second scene with a
picture an agent added: the page has the picture, as an image of the PDF); the MCP tools
through the server's stdio. Done: `CARGO_TARGET_DIR=target-pure cargo test -p innerword-cli
-p innerword-mcp -p innerword-agent-setup -p innerword-assistant`.

**As built** (B9 and the final review): the commands as 7.4 shows them (`board list` and
`boards`, aliases `whiteboard` and `whiteboards`; a `Text` kind in `spec.rs` for elements
and changes, so these commands take `--content`), the help's tables of the skeleton and of
a change, the overview at 81 lines, the outputs and hints as 7.4 says; `page_check.rs`
shares `Failed`, `finish` and `rules_dir` with `board_pdf.rs`; `innerword-assistant`'s
`claude/args.rs` puts the four board tools on the denied list (41), and its tools test
learned the prefixes `update_` and `remove_`. The command list counts 46 commands
(`output.rs`). The real child's case reads API, Postgres, reads and Backend in the page,
and a second whiteboard with a picture gives `/Subtype /Image`. `innerword-cli`'s
stand-in app refuses a picture with the switch off, as the app's listener does
(`innerword-app/src/live.rs`), whose own test holds the same. The final review added tests
of one count, of a change that changes nothing (directly and through the app), restore,
Untitled and a damaged whiteboard in `boards`.

#### B10. Agents, the panel and the docs in the app; bundling (wave 3)

- `crates/innerword-app`: 6.7 (the collect before a request, `must_wait` as `&mut self`,
  `take_over_open_board`, the note's words and its Undo and Open, the refusals); New
  Whiteboard through the panel's naming flow (`panel.set_new_board_content(…)` with a page
  engine, `workspace::NewBoard` calls `panel.new_board`); the assistant's board context
  (6.6: `assistant.rs`'s `open_document`, `assistant_document_changed`, `follow_the_text`;
  the quick chat's toast); `tests/suite/live.rs`'s cases (the request that waits for a
  closing whiteboard, a read that waits at most 1 s, a write that goes on after 1 s and
  keeps the writer's unposted stroke by the base rule, a composition refused after 3 s, a
  late post not saved over the change, the conflicts in the note and its Undo, an agent's
  rename not undone by the autosave, **an agent's picture on the open whiteboard**: stored
  by the real listener before the request is queued, adopted into the view with its
  `files` entry and served under `_files/`), `assistant.rs`'s, and the library's New
  Whiteboard items in `context_menus.rs` and `mouse.rs`; the measures (an agent's change
  while a whiteboard of 3,000 elements is open: `live=[… cost=[…]]`; the same with a
  16 MB picture, whose bytes the listener stores; the autosave of that whiteboard:
  `last_save=[…]`; opening one; its PDF; a folder's PDF with three whiteboards).
- `scripts/bundle.sh` (section 3: Resources, `INNER_WORD_BUNDLED=1`, `--check` where node is,
  the smoke test with a whiteboard, the notices on the disk image), `about.hbs` (the
  paragraph), `.gitattributes` if B3 has not.
- `CLAUDE.md`: the crate table (two new rows; `innerword-store` with whiteboards and schema 5;
  `innerword-web` with the bridge, `call_async`, `run_script`; `innerword-agent`,
  `innerword-mcp` 41 tools, `innerword-cli` with `board` and `__board-pdf`; `innerword-app`),
  the dependency direction (`{app, agent} → board`, `{app, cli} → {board, board-web}`, both
  new crates depend on nothing of ours; `web`'s dev-dependencies), the working rules (both
  new crates in `target-pure`; never run `npm` but in `crates/innerword-board-web/web`, and
  only to change the bundle; `df -h .` before a large build, 9.1), a section "Whiteboards"
  in the style of "Pages" (with Mermaid, the shape library and its setting, agents'
  pictures and protocol 4, the folder's PDF), the Agents section's paragraph on protocol 4,
  the PDF export's paragraph on a folder (its whiteboards), the scripted steps, the counts of
  the list of shortcuts, the exit codes, the measures, the bundle's and the disk image's
  sizes.
  `docs/gpui-notes.md`: what the bridge, `call_async` and the fixed `NoAction` bindings taught.
  `docs/pages-design.md`, section 5: the bridge's line.
Done: `cargo test -p innerword-app`, `cargo test -p innerword-assistant-ui`; the acceptance
checks below. Then the user does the person checks of section 10 (answer 7).

**As built** (B10, its fix round and the final review): `workspace::sentence`'s four
sentences and `conflicts_sentence`; `must_wait(&mut self, first)` with the four waits of
6.7; `take_over_open_board`; `Undo::Board` and the rule that a run must be nothing but this
whiteboard's adopts; `Workspace::board_served(path)` and `BoardState::ready_in` for the
tests; New Whiteboard through the panel, and the library's New whiteboard registered as a
row (6.2); the side chat's `BoardFeed` and its collect (6.6; the hook in
`innerword-assistant-ui` came last, `acd66c1`, `34e9953`). `scripts/bundle.sh` as section 3
says; `about.hbs` and `packaging/Read Me First.txt` (the license file, a privacy line);
`THIRD-PARTY-LICENSES.txt` made again for the `Cargo.lock` of the time. The measures are in
6.7, 4.3 and section 5, and in CLAUDE.md.

#### B11. The converter's cross-check (`innerword-web`'s test program, wave 2)

Moved out of B4 (reviewer's point, taken: it needs B2's finished converter and corpus, and
an entry point into Excalidraw's own converter that the minified bundle did not have). One
case of `tests/webkit.rs`: for every skeleton file of `innerword-board/tests/corpus/`,
`innerword_board::add` on an empty scene, and the export page's `convertSkeletons` (B3:
Excalidraw's `convertToExcalidrawElements`, then `restoreElements(…, null,
{refreshDimensions, repairBindings})`) on the same skeletons, loaded with
`run_script_blocking`: the same elements by id with the same fields and values, except
`seed`, `versionNonce`, `updated`, `index` (ordered the same) and `version`; text widths
within 3 %; bound arrow ends within 1 px. Dev-dependency on `innerword-board`. Done:
`CARGO_TARGET_DIR=target-pure cargo test -p innerword-web`.

**As built** (B11, B11f, `tests/webkit/crosscheck.rs`, the case `crosscheck`): labels are
matched by `containerId`, a label's own id translated inside `boundElements`, which are
compared as a set; numbers within 0.001; what follows from a text's width may differ by as
much as that width did, **by alignment**: a left-aligned free text not at all, a centred
one by half the difference, a right one by all of it, and a label likewise within the room
its container gives it, which starts at the container's x plus a padding and a share of its
width (none in a rectangle, `(1 - √2/2)/2` in an ellipse, a quarter in a diamond; the room of
an ellipse or a diamond rounded to a whole pixel), so the container's own differences count
only by that share (final review; B11f had first allowed `dcx + dcw/2` for every left label); a shape fitted to its label by twice the label's difference; a freedraw's size,
`pressures` and `simulatePressure` only noted. Two extensions of the agents' form are
rewritten into Excalidraw's before its converter sees them (a frame without `children`, a
family by name). Planted controls must be named exactly: a colour, an arrow end 2 px off
and a missing element; three free texts moved within and beyond their slack; and (final
review) labels in a rectangle, an ellipse and a diamond, left, centred and right. Every
corpus file agrees (texts within 0.6 to 2.9 %, bound ends and their `focus` and `gap`
exactly); `INNERWORD_WEB_CROSSCHECK_DUMP=<dir>` writes both conversions. The kerning
measured beside it is in 7.1.

### 9.3 Which existing tests and counts change

Each with its owner; a package that changes a number writes what it now counts.

- `innerword-store` (B1): `SCHEMA_VERSION` 5 and `OPEN_SCHEMA_VERSION` 4; the `WrongKind`
  tests (the new field). The `..` in `innerword-agent/src/pages.rs` and
  `innerword-library/tests/panel/pages.rs`.
- `innerword-agent` (B6): the operations table, the error kinds and their wording (a
  page's unchanged), the protocol version (4) and its module notes, `store_page_files` and
  `carries_page_files` with pictures (their page cases unchanged), `READERS`, `tests/suite/pages.rs`'s
  `WrongKind` literals; and `innerword-mcp/src/tools.rs`'s three `WrongKind` literals in
  its tests (around line 1899).
- `innerword-print` (B5): the `PrintRequest` literals of its tests; `Report`'s new field
  where a test builds one.
- `innerword-app` (B8): `keybindings.rs` (rows, entries, places, the fixed rows, the fixed
  `NoAction` bindings in the put-back test, `the_keymap_put_back_from_its_parts_resolves_every_key_as_before`
  from line 118); `shortcuts.rs`'s `KEY_GROUPS` (20 to 21); `tests/suite/pages.rs`'s rows
  (355) and entries (410) at lines 1352 and 1372; `context_menus.rs`, `mouse.rs` and
  `menus` for the whiteboard's own items; the binding count `keyscost` reports;
  `pdf_export.rs`'s folder tests (`a_folder_is_one_pdf_with_each_document_on_its_own_paper`
  and the toast's words: a folder without whiteboards makes the same PDF as before). (B10:
  `live.rs`, `assistant.rs`, the library's New Whiteboard items, File > New Whiteboard.)
- `innerword-library` (B7): rows' `page: bool` → `kind`.
- `innerword-mcp` (B9): 41 tools, 12 read-only (`tools.rs` line 1783 and 1818); the forbidden
  prefixes hold.
- `innerword-cli` (B9): 37 tools to 41 in `tests/suite/mcp.rs` (lines 47 and 259) and
  `tests/suite/pages.rs` (line 564); the help's overview and its line limit, the live
  coverage of commands that change the library, exit code 15's words (`help.rs` line 111),
  the hints matched in `main.rs` (lines 293 to 300).
- `innerword-assistant` (B9): `claude/args.rs` line 391, `allowed + denied` 37 to 41.
- `innerword-agent-setup` (B9): the skill text, `skill_against_binary.rs` (examples of
  whiteboards; exit code 15's words).

**As built:** the list of shortcuts has 442 rows and 424 entries with every part shown,
412 and 424 without the assistant (`tests/suite/pages.rs` asserts the second pair; 6.8);
`keyscost` counts 45 more bindings (the fixed ones, 43 keys and the two second spellings);
`innerword-agent`'s operations table has 30 rows; `innerword-mcp` 41 tools, 12 read-only;
`innerword-cli`'s command list 46 commands (`output.rs`); the agents' suite's `seed` holds a
whiteboard, `seed_texts` the old seed for the schema-1 test; `innerword-agent`'s rules test
asserts `OPEN_SCHEMA_VERSION` after an open; `innerword-assistant`'s tools test knows the
prefixes `update_` and `remove_`.

### 9.4 Acceptance checks for the whole feature

1. Every crate's tests pass as each package says; `cargo build -p innerword-app` and
   `CARGO_TARGET_DIR=target-pure cargo build -p innerword-cli` compile; `node build.mjs
   --check` passes.
2. **The scripted run** (debug build, a temporary library; it draws nothing itself):

   ```bash
   LIB=$(mktemp -d); OUT="$LIB/out"; mkdir -p "$OUT"
   df -h .    # stop and report when less than 5 GB is free (9.1)
   cargo build -p innerword-app && cargo build -p innerword-cli
   sips -s format png /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns --out "$OUT/icon.png"   # a 1024 x 1024 PNG
   INNER_WORD_DATA_DIR="$LIB" INNER_WORD_EXPORT_DIR="$OUT" \
   INNER_WORD_AUTOMATION="wait:1500; board:new:Architecture; board:wait; state; log:board ready; agents:1:60000; wait:500; board:scene; state; screenshot:$OUT/board.png; keys:cmd-shift-s; type:pdf; keys:enter; export:wait; quit" \
   ./target/debug/inner-word
   # When the log says "board ready", from a second terminal:
   ./target/debug/innerword --library "$LIB" boards     # one line: board:<id> “Architecture”
   B=board:<that id>    # a fresh library holds the welcome document, so not necessarily 1
   ./target/debug/innerword --library "$LIB" board add "$B" --author Claude --content '[
     {"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"backgroundColor":"#a5d8ff","fillStyle":"solid","label":{"text":"API"}},
     {"type":"ellipse","id":"db","x":420,"y":80,"width":160,"height":90,"label":{"text":"Postgres"}},
     {"type":"arrow","id":"reads","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
     {"type":"text","id":"title","x":100,"y":20,"text":"System overview","fontSize":28},
     {"type":"frame","id":"backend","children":["api","db","reads"],"name":"Backend"},
     {"type":"image","id":"icon","x":640,"y":80,"width":120,"path":"'"$OUT"'/icon.png"}]'
   # After the run (the app closed), the tool's own PDF, through the real child:
   ./target/debug/innerword --library "$LIB" export "$B" "$OUT/tool.pdf"
   ```

   Expected: `board: made board:<id> “Architecture”`; a first `state` with `board=[id=<id>
   state=loaded elements=0 … dirty=no …]`; the tool's `added to board:<id> “Architecture”: …`
   with exit 0 (the app made the change: no direct write); `board:scene` listing `api`,
   `db`, `reads`, `title`, `backend` and `icon` (a picture, stored, png 1024 x 1024, drawn 120
   x 120) with their texts; a second `state` with `live=[… changes=1 last="Claude drew on
   “Architecture”" … offers=Undo …]` and `board=[… elements=9 … dirty=no … pictures=1/1 …]`
   (the six, and three labels); the screenshot shows the six on the canvas, the icon among
   them; `export: “Architecture” written to …/Architecture.pdf: 1 page, W x H pt …`; and
   `tool.pdf`: one page of the same size within 1 pt, whose text (read with
   `innerword_print::PdfFile` in a few lines in the scratchpad, not committed) has "API",
   "Postgres", "reads", "System overview" and "Backend", and whose pixels show the icon.
   `innerword --library "$LIB" history "$B"` lists the version "Before a drawing by
   Claude".
   **A folder's PDF with its whiteboards** (answer 2), a fresh temporary library `LIB2`: a
   run with `INNER_WORD_SEED_DEMO=1` and `INNER_WORD_AUTOMATION="wait:1500; quit"` seeds
   it; with the app closed, `innerword --library "$LIB2" board new "Plan" --in "The Salt
   Road/Part one" --content '<the five shapes above>'` and `board new "Empty" --in "The
   Salt Road/Part one"`; then `INNER_WORD_EXPORT_DIR="$OUT2" INNER_WORD_AUTOMATION="wait:1500;
   keys:cmd-p; type:chap 2; keys:enter; wait:400; keys:cmd-shift-s; type:pdf; keys:down;
   keys:enter; export:wait; quit"`: `export:` names the file, "1 whiteboard" and `0 rows
   off`; the PDF has the folder's documents' pages as before (as many as the same export of
   a copy of the library without the two whiteboards) and one page more, the last, of the
   size of `innerword export board:<Plan> x.pdf` within 1 pt, whose text has "API" and
   "Backend"; the toast said "“Empty” is empty and was left out."
   **Mermaid and the shape library**, the same build and a fresh temporary library:
   `INNER_WORD_AUTOMATION="wait:1500; board:new:Flow; board:wait; action:board::MermaidToDrawing;
   wait:500; screenshot:$OUT/mermaid.png; board:press:escape; board:mermaid:flowchart LR\n
   A --> B; wait:800; board:scene; state; board:press:cmd-a; action:board::AddToShapeLibrary;
   wait:600; board:library; quit"`, then a second run on the same library with
   `keys:cmd-p; type:Flow; keys:enter; board:wait; board:library; quit`: the screenshot
   shows Excalidraw's Mermaid dialog, the listing has two shapes and an arrow from the
   diagram, `state` says `dirty=no`, and both runs log `board: library 1 item, saved`.
   **Drawing from the keyboard alone** (product rule 1), the same build and a fresh
   temporary library: `INNER_WORD_AUTOMATION="wait:1500; board:new:Keys; board:wait;
   keys:cmd-shift-p; type:add rectangle; keys:enter; wait:300; keys:cmd-shift-p; type:add
   text; keys:enter; board:press:shift-h i; board:press:escape; wait:400; board:scene; state;
   quit"`: the listing has a rectangle and a text "Hi" at the middle of the view, and
   `state` says `dirty=no` with `saved` equal to `rev`.
3. The same runs in a release build, with the measures written into CLAUDE.md.
4. `scripts/bundle.sh` with `INNER_WORD_DIST_DIR` set to a temporary folder: its smoke test
   passes, `Contents/Resources/Whiteboard/` and the two notices are in the app and on the
   disk image, and the disk image's size is written into CLAUDE.md.
5. The person checks of section 10, done by the user at the end, in one sitting (answer 7).

**What the build's runs found** (B8, B10, their fix rounds; debug app and tool on
temporary libraries, and the release build for the measures): (2) passes: the tool answered
`added to board:2 “Architecture”` with exit 0; `board:scene` listed api, db, reads, title,
backend and icon, the icon as `picture stored (png)` (the listing has no pixel size, 7.4);
`state` had `board=[… elements=9 … dirty=no … pictures=1/1]` and `live=[… changes=1
last="Claude drew on “Architecture”" offers=Undo cost=[save 0.0 ms, change 3.8 ms, window
0.8 ms]]`; `export:wait` wrote 1 page of 734 x 244 pt; `tool.pdf` is byte for byte the
app's PDF (146,433 bytes), its text has System overview, Backend, reads, API and Postgres,
and its pixels the icon; the history has "Before a drawing by Claude". The folder's PDF: 3
pages and "1 whiteboard" against 2 pages without the whiteboards, its last page the size of
`innerword export`. Mermaid and the shape library: two shapes and an arrow, `dirty=no`,
"library 1 item, saved" in both runs. The keyboard check failed as first built (the text
stayed "Text": `shift-h` reached Excalidraw before the text was open) and passes since
`HOLD_KEYS` and `OPEN_TEXT` (4.5), run exactly as written, in three runs. The `screenshot:`
step failed in the build's environment ("could not create image from window"), so no
screenshot of the canvas or of the Mermaid dialog was looked at: that is left to the
person checks. (4) passed with the whiteboard's smoke test; the arm64 disk image is
15,550,195 bytes (0.10.1: 12,495,267), the editor 7.5 MB in the app.

---

## 10. Scope, checks only a person can do, risks

### Not in this version

- Text to diagram, the AI tools, web embeds (Mermaid to drawing is in, 4.13).
- Collaboration, share links; the shape library's import and export of `.excalidrawlib`
  files, libraries.excalidraw.com, a library per whiteboard, agents and the shape library
  (the library itself is in, 4.15).
- Exports other than one PDF: PNG, SVG, `.excalidraw` files; a dark PDF; a whiteboard in a
  Markdown, DOCX or plain-text export; copying a whiteboard as Markdown; the agents' folder
  PDF (agents export a folder as Markdown, DOCX or plain text; a folder's PDF with its
  whiteboards is the app's, section 5).
- Importing `.excalidraw` files, or dropping one on a whiteboard (the drop is left out with
  the toast of a file that is no picture).
- Comments, spelling, Vim mode, the format bar and the quick chat on a whiteboard; the
  assistant changing a whiteboard; an assistant answer inserted into one.
- Elbow arrows and inline shapes (`start: {"type": …}`) from agents; agents changing a
  picture's file (they add pictures, 7.2); a picture from an address on the web; element
  links (`?element=`) from agents, and a line that binds, has arrowheads or a label (7.2);
  kerning in the converter's text widths (7.1).
- Moving the Add Text key hold (`HOLD_KEYS`, `OPEN_TEXT`) into the glue (4.5).
- A zoom of the app's kept for a whiteboard (its own `view_state` is kept), Life Size.
- Excalidraw's undo history across a reload or a relaunch (versions are the undo that lasts).
- More than one whiteboard view at a time (one live web view, as for pages).
- Whiteboards below macOS 12.3.

### Checks only a person can do

Each in under a minute, on the real binary, on a temporary library (`INNER_WORD_DATA_DIR`),
a whiteboard made with a scripted `board:new:Test; quit` and the app then launched by hand.
The user does them at the end of the feature, in one sitting, after B10 (answer 7); the
builders list in their reports what each package could not check otherwise.

1. **Keys in an active window.** Click into the canvas. `r`, drag: a rectangle; `t`, type
   "hello", `escape`; `v`; `cmd-a`, `cmd-d` (duplicate), `cmd-g` (group), `cmd-shift-g`,
   `cmd-alt-c` then select another shape and `cmd-alt-v` (styles), `cmd-alt-]` (to front),
   `cmd-z` three times and `cmd-shift-z`: each does Excalidraw's thing, and no comment panel,
   no Vim and no find bar appears; the Edit, View and Format menus show none of those keys
   while the whiteboard is open. The palette's "Add rectangle", then the arrow keys and
   `enter` with typing: a labelled shape made without the mouse. Hover the toolbar: no
   tooltip names a key that does something else here. `cmd-shift-p`: the palette comes up over a picture of the
   whiteboard; `escape`: drawing works again. `cmd-0` then `cmd-1`: the canvas has the keys
   again. `cmd-f`: Excalidraw's search, type "hello": found. `cmd-=`, `cmd--`, `cmd-shift-0`:
   the canvas zooms and the status bar follows. `cmd-shift-s`: the export picker with one row.
   `ctrl-enter`: the app's menu.
2. **The Edit menu with the pointer**: Edit > Undo, Redo, Select All, Copy, Paste on the
   canvas; copy a shape and paste it into another whiteboard; copy a picture from Preview
   and paste it onto the canvas; Excalidraw's right-click Paste (WebKit's own Paste button
   comes up and must be pressed).
3. **Never lose a stroke.** Draw, and at once `cmd-p` to another document, then back: the
   stroke is there. Draw and at once `cmd-q`, relaunch: the stroke is there; the same with
   `cmd-w`, with the window's red button, and with `ctrl-c` in the terminal that started the
   app. Type in a text element and at once click another document: the text is there. (A
   quit the system starts is the one case allowed to lose the last unfinished gesture: try
   the Dock's Quit once and a log-out once, right after a stroke, and write down what
   happened.)
4. **Text with an input method** (Japanese Kana) in a text element: composition, candidate
   window, commit; meanwhile run `innerword board add` from a terminal: it waits, then the
   change arrives after the commit without breaking it.
5. **An agent while drawing.** Keep dragging a shape while a terminal runs `innerword board
   update board:N --content '[{"id":"<another>","dx":50}]'`: the other shape moves, the
   dragged one stays the writer's; the note's Undo takes the agent's change out.
6. **The pointer**: the cursors of the tools, no flicker between GPUI's arrow and WebKit's;
   Excalidraw's own right-click menu; the link popover opens Safari on a click, and a
   whiteboard whose agent-made element has a link to another site opens nothing until the
   popover's link is clicked.
7. **The pinch and two-finger scroll** on the canvas: the canvas zooms and pans smoothly;
   the app's zoom of texts is not touched.
8. **A drop from the Finder**: a photo onto the canvas lands where it was dropped; an
   `.html` file becomes a page.
9. **The print panel**: Print… on a wide whiteboard: the sheet comes up on the user's paper,
   landscape, the drawing scaled to fit; Save as PDF from its menu.
10. **Theme**: Night turns the canvas dark at once; the exported PDF is light.
11. **Offline**: Wi-Fi off, relaunch, open a whiteboard with text in every font of the
    font menu: every font is there; export a PDF: the same fonts in it. Still offline,
    Mermaid to drawing with a flowchart: it converts.
12. **Mermaid with the pointer and the keys**: the extra tools' menu has "Mermaid to
    Excalidraw" and no web embed; its dialog takes a sequence diagram and inserts it; the
    palette's "Mermaid to drawing…" opens the same dialog, `cmd-enter` inserts and `escape`
    closes; a click on the dialog's "flowchart" syntax link opens Safari, and nothing opens
    without the click.
13. **The shape library**: select two shapes, "Add to library" from Excalidraw's menu, open
    another whiteboard and drag the item onto it; quit and relaunch: the item is in the
    library; "Insert from shape library…" in the palette puts it at the middle with the
    keyboard alone; no "Browse libraries" and no "Publish" anywhere.
14. **A folder's PDF** opened in Preview: after the documents, the whiteboard's page of its
    own size; zoomed in, its lines stay sharp and its words can be selected and found (an
    element's link is a link in the whiteboard's own PDF only, section 5).

**Built:** what the builders' reports add to these, because no script could see it: the
canvas and the Mermaid dialog on screen at all (the `screenshot:` step failed in the
build's environment); Add Text while a person types fast, with a dead key or an input
method during the 2 s hold (`HOLD_KEYS` replays held keys as synthetic events); two parked
WebKit views at once (only the fake had them); a real crash of the WebContent process and a
real failed adopt; Excalidraw's toolbar tooltips and its right-click menu with a pointer.

### Risks, and how the build retires each early

1. **Command keys in an active key window** (read in AppKit's order and gpui 0.2.2, never
   tried by a person; a scout's run in a background window was inconclusive: AppKit gave
   `cmd-z` to the menu bar). Check 1 is the user's, at the end (answer 7); B8 builds and
   tests everything a script reaches meanwhile. Because the Edit menu goes to
   the glue, GPUI is kept off Excalidraw's chords by fixed `NoAction` bindings, and the menu
   bar carries none of their key equivalents while a whiteboard is open, either order (the
   view first or the menu bar first) does the right thing for every key of 4.7; what could
   still go wrong is a chord neither the menu nor GPUI takes and WebKit does not pass to
   the page. (Built: the menu bar keeps the six keys of the Edit menu on a whiteboard, 4.7;
   `menukey:` showed none of the others while a whiteboard was open and all of them after;
   still open until check 1.) Fallback, in order: the web view subclass overrides
   `performKeyEquivalent:` and hands the page every chord of `WHITEBOARD_KEYS` before GPUI's
   view is asked; the view becomes a subview of GPUI's view.
2. **Losing the last gesture** at a deliberate end. Retired by the posts at each gesture's
   end, the collect with the kept-alive view, the app's own Quit, and the tests that hold
   answers back; check 3. What stays: a quit the system starts within a gesture.
3. **The synthetic keys of undo, redo and select all** (Excalidraw 0.18.1 has no public undo).
   B4's WebKit case holds them for the pinned version; `UPGRADING.md` lists them.
4. **The converter drifting from Excalidraw** (text widths, defaults, bindings). B11's
   cross-check against `convertToExcalidrawElements` fails on drift; Excalidraw measures
   again at every load, so what is seen and printed is exact. (Built: the corpus agrees;
   the one drift left is kerning, up to 15.8 % on a short kerned word, 7.1, with its fix
   named.)
5. **A new Excalidraw** moves keys, switches, fields, patches. Pinned exactly; the build
   fails when a patch stops applying; `UPGRADING.md`; B4's and B11's cases.
6. **Bundle size and repository size.** Built: 7.44 MB in the app and in git (minified
   JavaScript, 2.66 MB of it Mermaid's, and fonts), 3.1 MB more on the disk image;
   Xiaolai would have added 12.7 MB of fonts that do not compress (answer 1: left out).
   `--check` keeps `dist/` honest. Mermaid's chunks are 100 files that change with every
   Excalidraw upgrade; `dist/` is marked `-diff -text` so a review is not drowned in them and
   git never changes a byte of them.
7. **WebKit's private switches** go with pages': a macOS without them has neither.
8. **Main-thread cost of an agent's change** to a large open whiteboard (parse, delta, the
   adopt call). B10 measures with 3,000 elements; fallback: parse the two scenes and compute
   the delta on a background task right after `carry_out` answers the agent, and adopt it in
   a later turn of the main thread (the library has the change already; only the view
   waits). Measured (release, 3,000 elements, 3.0 MB): change 81 ms, window 41 ms; the
   fallback was not needed.
9. **The autosave of a large whiteboard** (2 MB of scene per gesture through the bridge and
   the background save). Measured by B10; the scene is parsed only in the background; the
   main thread reads the message's head (`RawValue`). Fallback: post only the changed
   elements (a delta) and keep the whole scene in the app. Measured (release, 3,000
   elements): main thread 0.00 ms, background 59 ms; the fallback was not needed.
10. **The schema bump** makes 0.10 refuse the library once it holds a whiteboard (the step
    is lazy, section 2). Accepted, as for 0.8.0.
11. **The whiteboard's stricter policy breaks something of Excalidraw's or Mermaid's** (a
    `new Function` in a dependency, a worker made from a blob, an inline style it sets by
    script, which `'unsafe-inline'` for styles allows). Mermaid's chunks were read for this
    (4.13: no eval that runs, no worker, no WebAssembly, no request), but under a page's
    policy only it has run. B4's bundle case loads, edits, exports, pastes and converts
    Mermaid diagrams with the policy and fails on any CSP report; fallback, decided there
   and written into this section: `'unsafe-eval'` in the live view's `script-src`, while
    inline scripts and every frame stay blocked (the threats of section 8, rows 3 and 16,
    are an inline or framed script, which `'unsafe-eval'` does not let in). Built: retired;
    B4's case runs Mermaid under the whiteboard's policy with no report, and the live view
    keeps its policy. What the export page needed was a patch of the font subsetter (no
    eval, section 3) and a policy chosen by the WebKit it runs on (4.2). And the policy
    turned out not to keep a `srcdoc` frame off: the view now loads no subframe (4.2).
13. **Joining WebKit's PDF into the folder's** (`CGContextDrawPDFPage` into `PdfWriter`'s
    context): Quartz is expected to keep the drawn page's paths, text and embedded fonts,
    which is what a PDF drawn into a PDF context does; never tried with WebKit's output.
    B5's test joins a page of `PdfWriter`'s own; B8's `pdf_export.rs` case (fake engine)
    and the acceptance run of 9.4 (real WebKit) read the joined page's text and pixels.
    Fallback: the whiteboard's page drawn from its SVG is not available in Rust, so the
    fallback is PDFKit (`PDFDocument.insertPage:atIndex:` on the finished folder PDF, then
    `dataRepresentation`), at the cost of a second copy of the PDF in memory. Built:
    retired by the 9.4 run with real WebKit (the page's size and text as `innerword export`
    makes them); what the join loses is the link annotations (section 5), the one reason
    left to take the PDFKit fallback some day.
14. **An agent's large picture on the main thread**: none: the listener stores the bytes
    before the request is queued (7.6), as a page's; the main thread writes the scene.
    What remains is the adopt of a scene that names it (the view fetches `_files/<id>`
    through the source, in the background). Measured (release, a 16 MB picture on the open
    whiteboard of 3,000 elements): change 76 ms, window 38 ms, the tool 401 ms.
12. **"Untouched" misjudged** (Excalidraw raises versions while it restores, repairs
    bindings or measures text again). Retired by the glue's own record of versions per rev
    (4.5), not the stored `baseVersion`; B4's case loads a Rust-made scene (whose texts
    Excalidraw measures again) and adopts a change to an untouched text, which must not be
    a conflict. Built: that text was re-measured without its version raised, so B11 added
    one whose version Excalidraw does raise at load (an index out of order: stored at 1,
    loaded at 2); the agent's copy with `baseVersion` 1 is taken at 3, no conflict.

---

## Answered

The user answered the design's questions; the sections above are written to the answers.
Numbers as the questions had them (the text refers to "answer N").

1. **Chinese, Japanese and Korean handwriting**: Xiaolai is left out (the design's
   default). CJK text on a whiteboard is drawn in the Mac's own font (PingFang and the
   like), on screen and in the PDF; the disk image and the repository stay 12.7 MB smaller.
2. **A folder's PDF holds its whiteboards**: after the documents, one page per whiteboard
   of its own size, the whiteboard's own PDF page joined in by `innerword-print`; an empty
   whiteboard is left out and the toast says so (section 5). The folder's Markdown, DOCX and
   plain-text exports still leave whiteboards out and say so.
3. **Agents put pictures on a whiteboard in v1**: a skeleton of type `image` naming a file
   of this Mac, read and made in the process that was asked, carried in the request and
   stored in `images.db` `files` before the scene names it, on protocol 4 with the rest
   (sections 7.2 and 7.6).
4. **Mermaid ships**: no stub patch; it needs nothing more of the whiteboard's strict policy
   (section 4.13), and the bundle is 7.44 MB as built (section 3).
5. **Printing**: scaled down to fit the user's paper (the design's default).
6. **Excalidraw's shape library ships, stored in the library**: one setting,
   `board.shape_library`, shared by every whiteboard; "Browse libraries" and publishing stay
   unreachable; items with pictures are not kept; agents do not touch it (section 4.15).
7. **The person checks** are done by the user at the end of the feature, in one sitting
   (section 10); no package waits for them.
8. **The same shape changed by the writer and an agent at once**: the writer's change stays
   (the design's default); the note names the agent's changes that were not taken, and the
   version the agent's change kept has the drawing as it was before.
