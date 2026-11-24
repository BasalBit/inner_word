# Upgrading Excalidraw

A new version of Excalidraw is a change of its own, never part of another one
(`docs/whiteboard-design.md`, section 3). What this bundle took from 0.18.1 is listed here;
read each of it again in the new package (its `dist/dev/` is readable, `dist/prod/` is what is
bundled) and change what moved.

## The build

1. Change the versions in `package.json` (exact versions, no ranges): `@excalidraw/excalidraw`,
   and `react` / `react-dom` to the line the new Excalidraw is released against.
2. `npm install --ignore-scripts --no-audit --no-fund` (writes `package-lock.json`), then
   `node build.mjs`. Every patch must still apply as often as it says, or the build fails:
   - **No font CDN**: `return r.push(new URL(n,X.ASSETS_FALLBACK_URL)),r}` in Excalidraw's font
     face becomes a fallback only when no local path was given.
   - **No Xiaolai**: the list of Xiaolai's faces (`var x=[{uri:…Xiaolai…}]`) is emptied and the
     paths of its files are blanked.
   - **The locales**: every `import("./locales/<lang>-<hash>.js")` but `en-` and
     `percentages-` resolves to one empty module.
   - **No eval in the font subsetter**: emscripten's embind in Excalidraw's subsetter makes
     three kinds of function with `Function` (`createNamedFunction`, `makeDynCaller`,
     `craftInvokerFunction`); the build makes them closures. The export page's policy has
     `'wasm-unsafe-eval'` and no eval: without the patches the subsetter fails and every font
     goes into the PDF whole.
   - The table of font families (`{Virgil:1,Helvetica:2,…}`) must be found once.

   The test `nothing_in_the_bundle_evaluates_a_string` counts what is left exactly: lodash's
   `Function("return this")` (6 copies) and chevrotain's dead `(0,eval)(t)` after a
   `return` (1). A new count is a new way to run a string: read it before changing the
   number.
3. `bundled-inside.json`: the build reads the source maps of the packages that carry others
   inside their files (`@mermaid-js/parser`, `mermaid`) and fails on one it does not list. A
   carrier is named with its version: a new version of it fails until its entry is read
   again. Excalidraw itself carries `fonteditor-core` and `harfbuzzjs` as WebAssembly in its
   subsetter chunk (their license comments are there, no source map): read
   `dist/prod/chunk-*.js` for `@license` again. `textsOfOurs`: packages that ship no license
   file (Excalidraw, Radix UI, fastdom, react-remove-scroll-bar); a new one fails the build.
4. `fonts-licenses/fonts.json`: every font folder of Excalidraw with its license and the
   copyright lines of its files' name tables (the build checks each file).
5. Copy `../dist/font-metrics.json` into `crates/innerword-board/src/metrics/` (a test of this
   crate holds the two equal), and change `EXCALIDRAW_VERSION` in this crate's `src/lib.rs`
   and in `innerword-board`.
6. `node build.mjs --check`, then `CARGO_TARGET_DIR=target-pure cargo test -p
   innerword-board-web`.

## What the glue relies on (`src/glue/`)

- **Keys** (`../src/keys.rs`, `WHITEBOARD_KEYS`; section 4.7): read the actions' `keyTest`s
  and `App.onKeyDown` again (`dist/dev/index.js`: `keyTest:`, `"onKeyDown"`). The keys the glue
  stops before Excalidraw sees them (`keys.js`, `NOT_EXCALIDRAWS`), where Excalidraw writes a
  key (a `title` "Label — Key" or, for the link, "Label - Key"; `aria-keyshortcuts`; the
  classes `context-menu-item__shortcut`, `dropdown-menu-item__shortcut` and
  `welcome-screen-menu-item__shortcut`), and its spelling of keys (`getShortcutKey`: "Cmd",
  "Option", "Shift", "+" between them, "Cmd+Shift+<" for the shifted comma), which `keys.js`
  reads into the keymap's normal form to cut the keys the window takes and the ones the app
  gave up (`keyShown`).
- **Undo, redo, select all** are synthetic key events on `.excalidraw` (`cmd-z`,
  `cmd-shift-z`, `cmd-a`): 0.18.1 has no public undo. `innerword-web`'s WebKit case holds them.
- **Switches** (section 4.13): `UIOptions`, `aiEnabled`, `validateEmbeddable`, the main menu
  and welcome screen, the dialogs closed as they open (`board.jsx`, `DIALOGS_LEFT_OUT`), the
  embed's menu item found by its label (`toolBar.embeddable`) because it shares its test id
  with Mermaid's, the classes and test ids `board.css` hides (`.library-menu-browse-button`,
  `lib-dropdown--load`, `lib-dropdown--export`, and `lib-dropdown--remove`, which is
  "Publish"), the sidebar's tab names (`search`, `library`) and the search's input
  (`.layer-ui__search-inputWrapper input`).
- **Kinds never kept**: `iframe`, `embeddable`, `magicframe` (`scene.js`, `NEVER_KEPT`), and
  what "Add to library" refuses (`LIBRARY_DISABLED_TYPES`: `iframe`, `embeddable`, `image`;
  `library.js`).
- **`restoreElements`**: it raises the version of an element whose local copy is newer, so
  the glue never passes it the canvas's elements as local ones (`edit.js`); it repairs
  bindings only within the list it is given, so `adopt` and inserts restore the incoming
  elements with copies of the canvas elements they name (container, frame, the ends of a
  binding, what is bound to them) and keep only the incoming ones; it drops invisibly small
  elements and deletes an empty text, which is why it never sees the canvas's own elements.
  `refreshDimensions` acts only with `repairBindings`. `updateScene` repairs fractional
  indices itself (`syncInvalidIndices` in `Scene.replaceAllElements`, mutating the second of
  two elements with one index); the glue orders by index first, the canvas's element before
  an incoming one.
- **A gesture under way** (the reason the merges of `edit.js` keep every element they do
  not name as the very object Excalidraw has): a freehand stroke, a shape being drawn and a
  line's next point are extended with `mutateElement` on `appState.newElement`
  (`onPointerMoveFromPointerDownHandler` in `dist/dev/index.js`), and `updateScene` with
  `captureUpdate` leaves uncommitted local elements out of the undo step
  (`store.filterUncomittedElements`). Read both again; B3's WebKit cases (an adopt during a
  stroke, right after one began, and while a new text is typed) hold it.
- **Fonts**: Excalidraw adds its faces to `document.fonts` only when it loads fonts for an
  export (`Fonts.loadElementsFonts`), which is how `fonts.js` loads them before text is
  measured (`exportToCanvas` onto one pixel).
- **The export** (`export.js`): `exportToSvg`'s options (`exportScale`, `exportPadding`,
  `exportBackground`, `exportWithDarkMode`), the SVG's `width` and `height`, the font
  subsetter's worker (it loads the chunk that holds it, `import.meta.url`). What
  `asTheCanvasDraws` mends: a frame's clip set with `setAttributeNS(SVG_NS, "clip-path", …)`
  (`maybeWrapNodesInFrameClipPath`; a fixed version sets it plainly, and the mend then finds
  nothing), the mask of an arrow's or a line's label (`mask-<id>`, a black `rect` of the
  label's box, where the canvas clears `BOUND_TEXT_PADDING` more), and the size of the
  export (`getCanvasSize(getRootElements(…))` with `addFrameLabelsAsTextElements`'s names,
  `FRAME_STYLE`), which `innerword_board`'s `Scene::pdf_bounds` repeats in Rust. The WebKit
  case `board_pdf` holds all three.
- **Mermaid** (`mermaid.js`): `parseMermaidToExcalidraw`, the retry with `"` made `'`, the TTD
  dialog's name and tab (`openDialog: {name: "ttd", tab: "mermaid"}`).
- **The shape library**: `onLibraryChange` (the status's `libraryWaiting` turns true there),
  `updateLibrary`, `restoreLibraryItems`,
  `getLibraryItemsHash`, and how "Add to library" makes an item (selected elements, their
  labels, the contents of selected frames).
- **Links**: `onLinkOpen` and `redirectToLink` (`window.open` when not prevented), element
  links (`?element=`).
- **The layout of the width** (`layout.js`, `followTheWidth`): Excalidraw chooses its compact
  layout in its ResizeObserver (`refreshEditorBreakpoints`, `App.device`) and renders there
  only when the container's rectangle changed (`updateDOMRect`), which its `resize`
  listener (`onResize`) stored first; under React 19 that listener's update renders at once
  with the old choice. The glue calls `refresh` from an observer of its own. A version whose
  observer renders after choosing makes the call one render more for nothing; the WebKit
  case `board_width` holds the layout to the width either way.

Then run `innerword-web`'s WebKit program (its cases of the bundle, and the cross-check of the
converter against `convertToExcalidrawElements`, which fails when Excalidraw's own output
moved) and the person checks of section 10.
