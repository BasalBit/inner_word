# Instructions for agents

This repository's instructions for coding agents are in [CLAUDE.md](CLAUDE.md). Read it
before you change any code: it has the map of the crates, the rules of every feature, the
build and test commands, and the facts about GPUI that bite. Human contributors start with
[CONTRIBUTING.md](CONTRIBUTING.md).

Three rules protect the user's own data. Never break them:

1. **Use a temporary library.** Every run of the app and every `innerword` command names a
   temporary folder with `INNER_WORD_DATA_DIR=<dir>` or `innerword --library <dir>`. Without
   it they work on the user's real library in `~/Library/Application Support/Inner Word`,
   and a first run moves an old `The Writer` library there.
2. **Never write into an agent's folder in the real home directory** (`~/.claude`,
   `~/.codex`, `~/.gemini`, `~/.agents` and the like): no install from the Agents panel, no
   `innerword agents install`, no agent's own `mcp add`. A temporary `HOME` does not make an
   agent's own command safe.
3. **Never call the real Claude Code from a test.** Tests use the fake backend
   (`innerword_assistant::FakeBackend`), and scripted runs `INNER_WORD_ASSISTANT_FAKE=ready`.

Two target directories, and no others:

- `target/` (the default) for the crates that depend on GPUI, and for `innerword-print`.
- `target-pure/` (`CARGO_TARGET_DIR=target-pure`) for the crates without GPUI:
  `innerword-core`, `innerword-store`, `innerword-image`, `innerword-export`,
  `innerword-vim`, `innerword-agent`, `innerword-mcp`, `innerword-cli`,
  `innerword-agent-setup`, `innerword-assistant`, `innerword-keymap`, `innerword-page`,
  `innerword-web`.
