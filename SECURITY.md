# Security policy

## Supported versions

Only the latest release of Inner Word gets security fixes. A fix comes in a new release;
older releases are not patched.

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting: open the
**Security** tab of this repository and choose **Report a vulnerability**. Only the
maintainers can read the report.

**Never report a vulnerability in a public issue, a pull request or a discussion.**

Please include:

- the version of Inner Word (Inner Word > About Inner Word) or the commit you built from;
- the macOS version, and Apple silicon or Intel;
- what an attacker can do, and what they need first (a document, a page, a running agent,
  a local account);
- the steps or a small proof of concept. Make it with a temporary library
  (`INNER_WORD_DATA_DIR`), and do not attach your own `library.db` or private documents.

## What is in scope

The parts of Inner Word where a boundary is meant to hold. [CLAUDE.md](CLAUDE.md) describes
each in detail.

- **The live channel.** While the app is open, the `innerword` tool hands changes to it over
  a Unix socket in `$TMPDIR/inner-word-<uid>/` (the folder mode 0700, the socket 0600, the
  peer's user checked). Another user reaching it, or a request that changes more than it
  says, is in scope.
- **The `innerword` tool and the MCP server (`innerword mcp`).** The tool takes the library's
  lock for each write, keeps a version before an agent changes a text, and moves deletions
  to the trash. Permanent deletion is behind a flag of the tool and not offered over MCP.
  A way around these is in scope.
- **The page sandbox in WebKit.** A page is served from its own origin over the
  `innerword-page` scheme, with a Content-Security-Policy and a content rule list that block
  `http(s)`, `ws(s)`, `ftp` and `file`; WebRTC, media devices, forms, downloads, new
  windows and navigation away are off; a link opens in the browser only when the reader
  opened it. A page that reaches the network, reads another page's storage, reaches the
  app, or opens a link by itself is in scope.
- **Fetching a page's pieces.** When a page is added, `curl` fetches only public addresses
  and checks every redirect. A fetch of a private or local address is in scope.
- **The assistant's isolation.** Claude Code runs in a neutral folder with the app's own
  system prompt, without the user's project instructions, hooks, skills, other MCP servers
  or built-in file and shell tools, with only the library's read-only tools. A path or an
  address in an answer is never read. A way for a document's text to make the assistant
  read a file or change the library is in scope.
- **Paths in agents' Markdown.** A local picture path in an agent's Markdown is read by the
  process the agent asked (the tool or the MCP server), never by the app; the MCP server
  reads whole paths only. A way to make the app read a file an agent or a document names
  is in scope.
- **The builds.** Releases are signed ad hoc and not notarised, and the first launch needs
  the user's approval. A problem in how `scripts/bundle.sh` builds, signs or packages the
  app is in scope. That macOS warns about an unnotarised app is expected and not a
  vulnerability.

Problems in GPUI, WebKit, SQLite, Claude Code or another agent itself belong to their own
projects. If Inner Word makes one reachable, report it here too.

## What to expect

The maintainers will acknowledge the report in its private advisory, work on a fix there,
and tell you when a release carries it. You can be credited in the published advisory if you
want. There is no bug bounty.
