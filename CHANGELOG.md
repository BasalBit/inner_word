# Changelog

All notable changes to Inner Word are written here, newest first. The form follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Until version 0.9.0 the app was called
The Writer.

## [Unreleased]

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

[Unreleased]: https://github.com/BasalBit/inner_word/compare/v0.10.1...HEAD
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
