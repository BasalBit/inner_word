# Changelog

All notable changes to Inner Word are written here, newest first. The form follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Until version 0.9.0 the app was called
The Writer.

## [Unreleased]

## [0.12.1] - 2026-11-24

The assistant's drawings beside a saved web page, and a side chat whose message wraps at
the width of its field.

### Added

- Beside a saved web page, an answer with a drawing has New Whiteboard, as beside a
  document (`cmd-shift-d` in the side chat, or "New whiteboard from answer" in the
  palette): it makes a whiteboard with the drawing in the page's folder and opens it, and
  the note has a button back to the page. The page itself is never changed.

### Fixed

- In the assistant's side chat, a message that runs onto more lines now wraps at the width
  of the field, and the field grows to show every line. Before, the text wrapped at about
  half the field and its last lines were drawn below it.
- Trashing or duplicating a folder names what it holds by kind ("1 page", "2 whiteboards",
  "3 items" when they are mixed). Before, a page or a whiteboard was counted as a document.
- When the assistant's drawing or an agent's element is refused, the reason says "an
  arrow", "an ellipse" and "an image" where it said "a arrow", "a ellipse" and "a image".
- In the side chat, the chip of the open item has the icon of its kind (a globe for a page,
  the whiteboard icon for a whiteboard), and its tooltip names the kind.

## [0.12.0] - 2026-11-24

The assistant draws on whiteboards.

### Added

- The assistant's side chat can answer with a drawing. Beside an open whiteboard, ask it to
  add, change or take away what is drawn ("add the database the API reads from, and make
  the API blue"). The answer never shows the drawing's code: one line says what the drawing
  does ("Drawing: adds an ellipse and an arrow; changes the rectangle “API”."), and the
  answer has a Draw button.
- Draw (the button, `cmd-shift-d` in the side chat, the last item of the answer's menu, or
  "Draw answer on whiteboard" in the palette) puts the drawing on the open whiteboard:
  elements added, changed and removed, a new drawing placed where the canvas is free and in
  view, and the view brought to what was drawn when it is out of sight. The whiteboard as
  it was is kept as a version ("Before the assistant's drawing"), and the whole drawing is
  one step of undo: `cmd-z` on the canvas or Undo in the note takes it back.
- Beside a document, an answer with a drawing has New Whiteboard instead ("New whiteboard
  from answer" in the palette): it makes a whiteboard with the drawing in the document's
  folder, opens it, and the note has a button back to the document.
- Nothing is drawn until you press Draw or New Whiteboard. An answer that was drawn says
  so: its button reads Draw Again (New Whiteboard Again), and the key and the palette need
  a second press. The assistant still has only its read-only tools; a drawing brings no
  pictures, reads no file, and links only to `http`, `https` and `mailto` addresses.
- Insert, Replace and Copy take an answer without its drawing.

### Fixed

- A whiteboard follows the width of its view. When the side chat, the sidebar or the
  window made the canvas narrower or wider, Excalidraw kept its toolbars laid out for the
  old width.
- No dark band where a note at the bottom of the window met a dialog over a whiteboard or a
  saved web page (the version history with a note open, for example).
- The side chat over a saved web page says "page" in its empty state and its placeholder,
  and its note about where your text goes names the open page or whiteboard instead of
  "the open document".

### Changed

- The list of keyboard shortcuts has one more row, "Draw answer (on the whiteboard, or as a
  new whiteboard)", in the assistant's chats, and the shortcuts file one more entry.
- Over a whiteboard or a document the side chat's instructions to the model are about a
  thousand tokens longer, to teach it how to write a drawing. They are sent with every
  message of the conversation, as before.
- Agents: a link on a whiteboard element that starts with `http://` or `https://` and holds
  a backslash, or names no host (`https:///example.com`), or has `%` or a space in its
  host, is refused with exit code 2. A browser would open another site than the one such a
  link seems to name. The assistant's drawings follow the same rule.
- The library format (schema 5) and the channel between the `innerword` tool and the open
  app (version 4) are unchanged: a tool of 0.11.0 works with the app of 0.12.0.

### Repository

- The design of the assistant's drawings, with its reviews, is in
  `docs/assistant-draws-design.md`. `innerword-assistant` depends on `innerword-board`,
  which reads and applies an answer's drawing.

## [0.11.0] - 2026-11-23

Whiteboards.

### Added

- Whiteboards: a third kind of item in the library, beside documents and pages, drawn with
  Excalidraw. Excalidraw is built into the app and runs offline, with the network blocked
  as on a page: its fonts, its shape library and Mermaid to drawing work without a
  connection, and nothing of a drawing leaves the Mac. Whiteboards need macOS 12.3 or later.
- Shapes, arrows, lines, text, frames and pictures (paste, a drop from the Finder,
  Insert > Image...), and Mermaid to drawing, which turns a Mermaid diagram into shapes and
  arrows that can be edited.
- A shape library to reuse shapes from, one for every whiteboard, kept in the library:
  "Show or hide the shape library", "Add selection to shape library" and "Insert from shape
  library...".
- New Whiteboard: File > New Whiteboard, the palette, the library's menu and its + menu, and
  "New Whiteboard Here" on a folder. The new whiteboard is named in place in the sidebar and
  opens with the keyboard on the canvas.
- Drawing from the keyboard: "Add text", "Add rectangle", "Add ellipse", "Add diamond", "Add
  arrow" and "Add frame" (the palette, Insert > Whiteboard, the title's menu and the
  `ctrl-enter` menu) put an element at the middle of the view; an arrow with two shapes
  selected connects them. Excalidraw's own keys work on the canvas, and the list of
  keyboard shortcuts shows them under "On a whiteboard".
- A key you give a command in Keyboard shortcuts wins over Excalidraw's own key on a
  whiteboard: given `cmd-k`, the command palette opens there, and Excalidraw's tooltips and
  menus no longer show `cmd-k`. Excalidraw's link is still reached from its own menus and
  toolbar.
- A whiteboard is saved as it is drawn, at the end of every stroke or edit, and before
  anything that ends it: another item opened, closing it, Save Now, export, print, closing
  the window and quitting. A drawing that cannot be saved is written to a recovery file in
  the library's folder, and the save state says where.
- Whiteboards are in the trash, the version history (each version's elements listed;
  Restore takes the old drawing back into the open whiteboard), quick open and the library
  search, which finds a whiteboard by the words drawn on it.
- Export as PDF of a whiteboard: one page the size of the drawing, its lines as vector
  paths and its text as text, in light colours whatever the theme. Print puts it on your
  paper, turned to the drawing and made smaller to fit. A folder's PDF holds its
  whiteboards after its documents.
- The zoom commands and the status bar zoom the canvas (Fit Width and Fit Page show the
  whole drawing), Find opens Excalidraw's search, and the theme follows the app's.
- The assistant's side chat reads an open whiteboard as a list of its elements with their
  text, and can read other whiteboards with its tools. It does not change a whiteboard.
- Agents draw on whiteboards: `innerword board new`, `board add`, `board update`, `board
  remove` and `innerword boards`, and the MCP tools `create_board`, `add_to_board`,
  `update_board` and `remove_from_board`. Elements are given in Excalidraw's own form
  (shapes with labels, arrows that connect shapes by their ids, text, frames and pictures
  from a file); `innerword get board:<id>` lists the elements with their ids, positions,
  sizes and text, and `innerword export board:<id> <file>.pdf` makes the PDF. Every change
  keeps the drawing as it was as a version first, and `--dry-run` says what would change.
- While a whiteboard is open, an agent's change arrives in it at once, merged with what is
  being drawn, and the note at the bottom offers Undo.
- The skill for agents explains whiteboards, with examples, and exit codes 3 (an element the
  whiteboard does not have) and 9 (the PDF of an empty whiteboard). The Agents panel offers
  to update an installed skill.
- `THIRD-PARTY-LICENSES-WHITEBOARD.txt` on the disk image (and in the app) lists what the
  whiteboard's editor is made of, JavaScript and fonts, each with its license. "Read Me
  First.txt" says that whiteboards work offline.

### Changed

- The library moves to a new format (schema 5) when it gets its first whiteboard, made in
  the app or by an agent. Inner Word 0.10 then refuses to open it; a library without a
  whiteboard stays readable by 0.10.
- The channel between the `innerword` tool and the open app is at version 4: a tool of 0.10
  and an app of 0.11 (or the reverse) refuse each other's requests with exit code 5, and say
  so. The tool inside the app is always the same version as the app.
- The tool and the MCP server say "Untitled" for an item without a title in every output.
- Print can make a drawing smaller to fit the paper (used for whiteboards; documents are
  printed at their real size as before).
- Menus and dialogs over a saved web page come up at once, with no flicker. They no longer
  wait for a picture of the page; the live page is dimmed and covered instead.
- The disk image is about 3 MB larger, with the whiteboard's editor in it.

### Repository

- The landing page lives at <https://innerword.io/>, on its own domain instead of GitHub
  Pages.
- `crates/innerword-board` (a whiteboard's scene in Rust, and the converter that makes
  agents' skeletons into elements) and `crates/innerword-board-web` (Excalidraw 0.18.1 built
  offline from `web/` with exact versions and patches by `node build.mjs`; the built
  `dist/` is committed, and `node build.mjs --check` holds it byte for byte).
- The design of whiteboards, with what was built, is in `docs/whiteboard-design.md`.

## [0.10.1] - 2026-11-17

The first public release of Inner Word.

### Added

- The license of Inner Word (`LICENSE`, MIT, Copyright (c) 2026 BasalBit) and the licenses
  of the third-party code built into the app and the tool (`THIRD-PARTY-LICENSES.txt`), in
  the app's `Contents/Resources` and on the disk image beside the app.
- The copyright line in the About box ("Copyright © 2026 BasalBit. MIT License.").
- New sections in "Read Me First.txt": where the source code is and under which license,
  how to report a problem (and a security problem privately), what the app sends over the
  network and when (no account, no usage data, no update check, no crash reports), and
  which disk image to choose (universal or arm64).

### Changed

- The skill installed for agents and the help of the `innerword` tool use invented example
  ids and names in their examples.

### Repository

- The source is open under the MIT license, with a README, contributing notes, a security
  policy, a code of conduct, this changelog, AGENTS.md for agents that do not read
  CLAUDE.md, issue and pull request templates, and GitHub workflows for CI, releases and
  the landing page.
- A landing page in `site/`: static, no requests to other sites, real screenshots in Paper
  and Night, download links built from one version number.
- The third-party licenses are generated with cargo-about (`scripts/bundle.sh
  --third-party-licenses`), and `deny.toml` configures cargo-deny.
- The Rust toolchain is pinned in `rust-toolchain.toml` (Rust 1.97.1).
- The code is formatted with `cargo fmt`; `.git-blame-ignore-revs` names that commit.

## [0.10.0] - 2026-11-17

### Changed

- The Writer is now called Inner Word: the app, the bundle identifier
  `app.innerword.InnerWord`, the command line tool `innerword` and the crates `innerword-*`.
- The first launch moves the library from `~/Library/Application Support/The Writer` to
  `~/Library/Application Support/Inner Word` and leaves a link under the old name, so
  whatever still names the old folder finds the same library.
- `THE_WRITER_DATA_DIR` is still read where `INNER_WORD_DATA_DIR` is not set.
- The Agents panel recognises the skill and the MCP entry The Writer installed, offers to
  update the skill under the new name, and says how to remove the old MCP entry.

## [0.9.0] - 2026-11-16

### Added

- Export to PDF: "PDF file" in the export picker, File > Export as PDF... and the palette.
  The PDF has the same pages Print makes: paper, margins, fonts, pictures and page numbers
  as page view lays them out, whatever the window shows.
- A folder exports as one PDF, each document on its own paper, its title above it where it
  does not show it, the pages numbered on.
- The pages are made in slices with a note and Cancel. On an open page the command makes
  WebKit's PDF of the page.

## [0.8.1] - 2026-11-14

### Fixed

- The tick of a checked to-do is drawn as a sprite instead of a GPUI path, which held a
  window-sized texture and its multisampled copy, about 400 MB on a large Retina window.
- The caret no longer blinks in a window that is not active, in the editor and in text
  fields.

## [0.8.0] - 2026-11-14

### Added

- Pages: a web page (an HTML file or a folder, a claude.ai artifact among them) kept in the
  library beside the documents and shown in WebKit. Everything it needs is downloaded once
  when it is added, and showing it never touches the network.
- A page that cannot be kept whole is not kept at all, and the reason names what is missing.
- Pages are added with File > Import Page..., the library's menus or a drop, and by agents
  with `page add` and the MCP tool `add_page`.
- Each page is its own origin, with the network, forms, downloads, new windows and devices
  off. A link to another site opens in the default browser only when the reader opened it.
- An open page has find, zoom, print, export as a folder or a PDF, and the version history.

## [0.7.0] - 2026-11-13

### Added

- Printing: File > Print..., the title's menu, the palette and Vim's `:print`, through the
  macOS print panel, with the document as page view lays it out.
- Images in documents: Insert > Image..., paste, and drop from the Finder. Resize with the
  handles or the keys, move, align, describe and delete.
- The bytes of a picture are stored once in `images.db`, with a copy among the backups.
- Agents add pictures with `image add` and the MCP tool `add_image`.

## [0.6.0] - 2026-11-12

### Added

- Zoom of the text, which changes no size in the document: keys, menus, the status bar,
  `cmd` with the scroll wheel, and a pinch.
- A page opens as large as its paper, and page view can zoom above 100 %.
- The list of fonts in the settings.
- Agents can change documents while the app is open: the app carries the change out, shows
  it, and says so in a note with Undo. A switch in the Agents panel turns this off.

## [0.5.0] - 2026-11-12

### Added

- Every keyboard shortcut can be changed, in Keyboard Shortcuts (`cmd-/`).
- The shortcuts can be exported to a file and imported from one, with a preview.

## [0.4.0] - 2026-11-12

### Added

- The assistant, on Claude Code: a quick chat at the selection and a side chat beside the
  text. An answer changes a document only when one of its buttons is pressed.

## [0.3.0] - 2026-11-12

### Added

- Agents: the command line tool (then called `writer`), its MCP server, the skill that
  tells agents about it, and the Agents panel.

## [0.2.0] - 2026-11-12

### Added

- Spell checking with the macOS spell checker, with suggestions, Learn Spelling and Ignore
  Spelling, and a spelling language per document.

## 0.1.0 - 2026-11-11

### Added

- The first version, as The Writer: the rich-text editor with flow and page view, the
  library of folders and documents in SQLite with trash, versions, search and backups,
  comments, Vim mode, find and replace, the command palette, export to Markdown, DOCX and
  plain text, and an ad-hoc signed disk image.

[Unreleased]: https://github.com/BasalBit/inner_word/compare/v0.12.1...HEAD
[0.12.1]: https://github.com/BasalBit/inner_word/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/BasalBit/inner_word/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/BasalBit/inner_word/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/BasalBit/inner_word/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/BasalBit/inner_word/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/BasalBit/inner_word/compare/v0.8.1...v0.9.0
[0.8.1]: https://github.com/BasalBit/inner_word/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/BasalBit/inner_word/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/BasalBit/inner_word/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/BasalBit/inner_word/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/BasalBit/inner_word/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/BasalBit/inner_word/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/BasalBit/inner_word/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/BasalBit/inner_word/tree/v0.2.0
