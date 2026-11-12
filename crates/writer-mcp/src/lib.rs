//! The Model Context Protocol server of The Writer, and the operations behind it.
//!
//! `writer mcp` runs [`serve_stdio`]: an MCP server on standard input and output that
//! offers the operations of the command layer (`writer-agent`) as tools. It works on the
//! library directly and needs no running app; while the app is open the tools that read
//! work and the tools that change something answer with a tool error that says so.
//!
//! * [`protocol`]: the wire protocol, written from the specification
//!   (modelcontextprotocol.io, revision 2026-07-28 and the handshake revisions before
//!   it). It knows nothing about The Writer.
//! * [`tools`]: every operation with its name, description, argument schema and text. The
//!   `writer` command line tool runs its subcommands through the same table
//!   ([`Toolbox::run`]), so a command and its tool always do and say the same.
//! * [`args`]: the schemas and the check of arguments against them.
//! * [`render`]: the results as text.
//!
//! # Why the protocol is written by hand
//!
//! The official Rust SDK (`rmcp`) was weighed against it. This server answers one request
//! at a time from one client over a pipe, never sends a request of its own, and keeps no
//! state: the protocol it needs is a function from a line to a line, a few hundred lines
//! with their tests. The SDK would bring an async runtime, a schema generator and a macro
//! crate into a binary that is otherwise a synchronous command line tool which starts in a
//! few milliseconds, and its release cycle would decide when the revision of 2026-07-28
//! (no handshake, per-request metadata, `server/discover`) is spoken. Written by hand, both
//! eras are served by the same few functions and every rule the server follows is a test
//! in this crate.

pub mod args;
pub mod protocol;
pub mod render;
pub mod tools;

use std::io;

pub use protocol::{Server, ServerInfo};
pub use tools::{Failure, Reply, Toolbox};
pub use writer_agent::{ErrorKind, Item, Library};

/// The name of the server in the protocol (`serverInfo.name`) and the name agents
/// register it under.
pub const SERVER_NAME: &str = "the-writer";

/// What the server tells a model about itself (the `instructions` of the protocol).
pub const INSTRUCTIONS: &str = "The Writer is the user's writing app for macOS; its library holds their \
documents (books, chapters, posts, notes) in folders. These tools work on that library directly, whether or \
not the app is running. Documents and folders are named by id or by their path in the library \
(\"The Salt Road/Part one/Chapter 1\"; the last part alone is enough when nothing else has that name, and a \
name that fits several items fails with the list of them); every result carries ids, and an id is exact. Text goes in and comes out as Markdown. \
Start with list_library or search_library. To change part of a text use edit_document (an exact piece of \
text), replace_section or append_to_document rather than replace_document; to review a text without changing \
it, use add_comment. Before every change the text as it was is kept as a version (list_versions, \
restore_version), deleting means the trash, and dry_run reports what a tool would do. While The Writer is \
open, tools that read still work and tools that change something are refused: ask the user to quit the app, \
then call the tool again.";

/// The identity of this server.
pub fn server_info() -> ServerInfo {
    ServerInfo {
        name: SERVER_NAME.to_string(),
        title: "The Writer".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        instructions: INSTRUCTIONS.to_string(),
    }
}

/// An MCP server over the operations on `library`.
pub fn server(library: Library) -> Server<Toolbox> {
    Server::new(Toolbox::new(library), server_info())
}

/// Serves MCP on standard input and output until the input ends.
pub fn serve_stdio(library: Library) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    server(library).run(stdin.lock(), stdout.lock())
}
