# Contributing to Inner Word

Thank you for helping. This file says how to build and test Inner Word, the rules that
keep your own documents safe while you work on it, and what a pull request should contain.

[CLAUDE.md](CLAUDE.md) is the project's detailed engineering notebook: the map of every
crate, the rules of every feature, what each test file holds, and the facts about GPUI that
bite. It is written for the AI agents that helped build Inner Word and for people alike.
Read the parts about the code you change before you change it.

## What you need

- **macOS.** Inner Word builds and runs on macOS only. Its crates use AppKit, WebKit,
  ImageIO, CoreText, PDFKit and `NSSpellChecker`.
- **Xcode or the Command Line Tools for Xcode.**
- **Rust through [rustup](https://rustup.rs).** The toolchain, with `rustfmt` and `clippy`
  and the `aarch64-apple-darwin` and `x86_64-apple-darwin` targets, is pinned in
  [`rust-toolchain.toml`](rust-toolchain.toml). rustup installs it on the first build. The
  x86_64 target is needed only for `scripts/bundle.sh --universal`.
- No offline Metal compiler. GPUI is built with `runtime_shaders`.

## Protect your own data

Inner Word and its `innerword` tool work on the real library in
`~/Library/Application Support/Inner Word` unless they are told otherwise. The Agents panel
and `innerword agents ...` write into the folders of the agents on your Mac. A slip while
developing can change your own documents or your own agents. These rules prevent that:

- **Every run of the app and every `innerword` command names a temporary library**, with
  `INNER_WORD_DATA_DIR=<temporary folder>` or `innerword --library <temporary folder>`.
  Without it the first run of a build also moves an old `The Writer` library to the new
  folder name. Tests set up their own temporary libraries; keep it that way in new tests.
- **Never install into the real `~/.claude`, `~/.codex`, `~/.gemini`, `~/.agents` or any
  other agent's folder** while you develop: not with the Agents panel's buttons, not with
  `innerword agents install`, not with an agent's own `mcp add`. Setting `HOME` to a
  temporary folder does not make an agent's own command safe: some agents ignore `HOME`.
  Check such a command's syntax with its `--help` only. Tests use
  `agents::set_environment(Environment::isolated(...))` and a temporary home.
- **Tests never call the real Claude Code.** Every message spends the user's own plan, and
  its answers differ every time. Tests use `innerword_assistant::FakeBackend`; a scripted run
  of the app uses `INNER_WORD_ASSISTANT_FAKE=ready`.
- **Tests never ask the Mac's dictionary, never open the open panel or the print panel, never
  read or write the system pasteboard, never make a `WKWebView` (outside `innerword-web`'s
  own program) and never reach the network.** Each has a fake or a stand-in; CLAUDE.md
  names them.
- **In a scripted run of the app, never script Copy or Cut** (it is your clipboard), and
  never post `cmd-h`, `cmd-alt-h` or `cmd-m` to the app (Hide Others hides every other
  application).

## Build

```bash
cargo build -p innerword-app            # debug build
INNER_WORD_DATA_DIR=/some/tmp cargo run -p innerword-app   # run against a throwaway library
CARGO_TARGET_DIR=target-pure cargo build -p innerword-cli   # the `innerword` tool (target-pure/debug/innerword)
cargo build -p innerword-cli            # the same beside a debug app (target/debug/innerword), where the Agents panel looks for it
scripts/bundle.sh                    # release .app (the app and the tool) + .dmg in dist/
INNER_WORD_DIST_DIR=/some/tmp scripts/bundle.sh   # a trial build that leaves dist/ alone
```

Replace `/some/tmp` with a folder of your own under `/tmp`.

## Two target directories

The workspace uses two target directories, and no more:

- **`target/`** (the default) for everything that depends on GPUI, and for
  `innerword-print`: `innerword-ui`, `innerword-editor`, `innerword-editor-tools`,
  `innerword-vim-mode`, `innerword-comments`, `innerword-spell`, `innerword-library`,
  `innerword-keymap-ui`, `innerword-assistant-ui`, `innerword-assistant-quick`,
  `innerword-app`, `innerword-print`.
- **`target-pure/`** (`CARGO_TARGET_DIR=target-pure`) for the crates without GPUI, so they do
  not wait behind GPUI builds: `innerword-core`, `innerword-store`, `innerword-image`,
  `innerword-export`, `innerword-vim`, `innerword-agent`, `innerword-mcp`, `innerword-cli`,
  `innerword-agent-setup`, `innerword-assistant`, `innerword-keymap`, `innerword-page`,
  `innerword-web`.

A test binary that links GPUI is about 150 MB. Keep **at most one integration-test binary
per GPUI crate** (`tests/main.rs` with submodules), or use unit tests. Add an example only
when it is needed.

If cargo says "waiting for file lock on build directory", another build is running. Wait.

## Test

```bash
cargo test -p <crate>                                        # a GPUI crate, or innerword-print
CARGO_TARGET_DIR=target-pure cargo test -p innerword-core    # a pure crate
CARGO_TARGET_DIR=target-pure cargo test -p innerword-page    # the bundler (no network: MapFetcher, a server on 127.0.0.1)
CARGO_TARGET_DIR=target-pure cargo test -p innerword-web     # WebKit, offscreen, in a program of its own
```

Run the tests of every crate you changed, and `cargo build -p innerword-app` before you open
a pull request. Add `--lib --tests` to skip building the examples, which are large in the
GPUI crates (`innerword-cli` has no library: use `--bins --tests` there). Some tests are ignored by default because they use the real macOS spell checker,
measure a release build or reach the internet; run them only on purpose.

The app's own tests are one binary, `crates/innerword-app/tests/main.rs`, with a file per area
under `tests/suite/` (the story test, keys, saving, mouse, context menus, spelling, agents,
the live channel, the assistant, zoom, printing, PDF export, images, pages). CLAUDE.md says
what each holds.

Scripted runs of the real app, for looking at a change, are described in CLAUDE.md under
"Build and test" (`INNER_WORD_AUTOMATION`).

## Format

Run `cargo fmt --all` before you commit. CI runs `cargo fmt --all --check`.

## Product rules

A change must keep these. CLAUDE.md has the details.

1. **Keyboard and mouse are equals.** Every feature can be reached and finished without a
   mouse: a shortcut, the command palette, or the keys of whatever is open. And every
   feature can be reached and finished with the mouse alone: a right-click menu on every
   object, the native menu bar, and a small visible control where a right-click would not
   be found. Every overlay closes with `escape` or a click outside and gives the keyboard
   back to where it came from.
2. **Few menus in the window.** No ribbon and no permanent formatting toolbar. Formatting is
   contextual: shortcuts, Markdown-style input rules, the format menu, the insert menu,
   right-click menus, the palette and the format bar over a mouse selection.
3. **One source of truth for shortcuts.** Keys are GPUI key bindings, bound in a crate's
   `init`. The list of shortcuts, the palette's hints and the menu bar are generated from
   them. The user can change every key, so never write a key into a text, a tooltip or a
   comparison: ask the keymap.
4. **Never lose text.** Documents save themselves, and on close and quit. The bytes of a
   picture are stored before a block names them. Nothing deletes them but the collector.
5. **UI text is plain and specific. No emoji in the UI.**
6. **Saved names keep their old spelling.** `"the-writer-document"`,
   `"the-writer-fragment"` (`innerword-core/src/serial.rs`) and `"the-writer-shortcuts"`
   (`innerword-keymap/src/file.rs`) are written into files people keep. Never rename them:
   existing libraries and exported files would become unreadable.

Also keep the dependency direction in CLAUDE.md ("Workspace layout"): no crate depends on one
above it, and none of the agent crates depends on GPUI.

## Pull requests

A pull request should say:

- **What changed and why.** Link the issue when there is one.
- **Which crates it touches.**
- **Which tests you ran**, and in which target directory.
- That **`cargo build -p innerword-app` passes** and `cargo fmt --all --check` is clean.
- For a change people can see: that you **tried it with the keyboard and with the mouse**,
  and **screenshots in both themes** (Paper and Night).

Keep a pull request to one change. Commit in working steps; a commit message starts with the
crate it is about (`innerword-editor: ...`) and says what is now so. Every commit should
compile.

## Reporting bugs and asking for features

Open an issue with the bug report or feature request form. For a bug, say which macOS
version, Apple silicon or Intel, and the version from Inner Word > About Inner Word, and give
the steps. **Do not attach `library.db`, `images.db` or private documents.** Make a small
document that shows the problem instead.

Security problems go through private reporting, not issues: see [SECURITY.md](SECURITY.md).

## AI-assisted contributions

Contributions made with AI coding agents are welcome; most of Inner Word was built that way.
Point your agent at [CLAUDE.md](CLAUDE.md) (agents that read `AGENTS.md` find a pointer
there). You are responsible for what you submit: read it, run the tests, and check the
behaviour yourself.

## Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).

## License

Inner Word is released under the [MIT License](LICENSE). By contributing, you agree that your
contributions are licensed under the same license.
