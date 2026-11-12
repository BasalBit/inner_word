//! The Model Context Protocol over a line stream, for a server that only has tools.
//!
//! Written from the specification at modelcontextprotocol.io (revision 2026-07-28 and the
//! handshake-based revisions before it), not from an SDK: the server answers one request
//! at a time, never sends a request of its own and holds no state between requests, so the
//! whole protocol is a function from one line to at most one line.
//!
//! # Two eras
//!
//! The server is "dual-era" in the words of the specification:
//!
//! * **Modern** (2026-07-28). There is no handshake. Every request names its protocol
//!   version and the client's capabilities in `params._meta`
//!   (`io.modelcontextprotocol/protocolVersion`, `io.modelcontextprotocol/clientCapabilities`).
//!   A version the server does not speak is answered with `-32022` and the list of
//!   versions it does speak; a missing capabilities field with `-32602`. `server/discover`
//!   reports versions, capabilities, identity and the instructions. Every result carries
//!   `resultType: "complete"` and the server's identity in `_meta`; `tools/list` and
//!   `server/discover` carry the caching hints `ttlMs` and `cacheScope`.
//! * **Legacy** (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05). The client opens with
//!   `initialize`; the server answers with the version asked for when it speaks it and its
//!   newest legacy version otherwise. `notifications/initialized` is taken without an
//!   answer, `ping` is answered with an empty result.
//!
//! The era is read from each request by itself: a request with a protocol version in
//! `_meta` is modern, one without is legacy. Nothing is remembered between requests, so
//! a legacy client that skips `initialize` is served too (the specification leaves that
//! to the server; it makes the server easy to try by hand).
//!
//! # Transport
//!
//! stdio: one JSON-RPC message per line, no embedded newlines, nothing else on standard
//! output. The server stops when its input ends. A JSON-RPC batch (an array of messages,
//! allowed by revision 2025-03-26 only) is answered with an array.
//!
//! # Errors
//!
//! Protocol errors are JSON-RPC errors: `-32700` for a line that is not JSON (without an
//! `id`, since none could be read; see below), `-32600` for a message that is not a request,
//! `-32601` for a method the server does not have, `-32602` for parameters it cannot use
//! (an unknown tool among them). Everything a tool itself reports, the refusal while the
//! app is open included, is a normal `tools/call` result with `isError: true`, so that the
//! model reads the message and can act on it.
//!
//! **An error for a message whose id could not be read has no `id` member, and not
//! `"id": null`.** The two specifications disagree here, and this server speaks MCP.
//! JSON-RPC 2.0 says of that case that the id "MUST be Null". The schema of MCP says
//! `JSONRPCErrorResponse { jsonrpc; id?: RequestId; error }` with
//! `RequestId = string | number`: the member may be left out and is never null, and a
//! client that checks messages against that schema (the SDKs do) takes `"id": null` for
//! a message it cannot read. Read again on 2026-11-12 in `schema/draft/schema.ts` of
//! modelcontextprotocol/modelcontextprotocol and in section 5 of jsonrpc.org/specification.

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value, json};

/// The modern protocol revision this server speaks.
pub const MODERN_VERSION: &str = "2026-07-28";

/// The handshake-based revisions this server speaks, newest first.
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// `_meta` key: the protocol version of a modern request.
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key: the client capabilities of a modern request.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key: the client's identity in a modern request.
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key: the server's identity in a modern result.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// JSON-RPC: the line is not JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: the message is not a request.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: the server does not have this method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the parameters cannot be used.
pub const INVALID_PARAMS: i64 = -32602;
/// MCP 2026-07-28: the protocol version of the request is not supported.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// How long a client may keep the tool list and the discovery result, in milliseconds.
/// Both are fixed for a build of the binary.
const CACHE_TTL_MS: u64 = 3_600_000;

/// What a tool does to the library, as the protocol's tool annotations say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Reads only (`readOnlyHint: true`).
    Read,
    /// Adds something and takes nothing away (`destructiveHint: false`).
    Add,
    /// Changes or removes something that was there (`destructiveHint: true`).
    Change,
}

/// One tool as `tools/list` describes it.
#[derive(Clone, Debug)]
pub struct ToolDef {
    /// The name a model calls it by.
    pub name: &'static str,
    /// A short name for a person.
    pub title: &'static str,
    /// What it does, written for a model.
    pub description: String,
    /// The JSON Schema (2020-12) of its arguments, an object schema.
    pub input_schema: Value,
    /// What it does to the library.
    pub effect: Effect,
    /// Whether calling it again with the same arguments changes nothing further.
    pub idempotent: bool,
}

impl ToolDef {
    fn to_json(&self) -> Value {
        let mut annotations = Map::new();
        annotations.insert("readOnlyHint".into(), json!(self.effect == Effect::Read));
        if self.effect != Effect::Read {
            annotations.insert(
                "destructiveHint".into(),
                json!(self.effect == Effect::Change),
            );
            annotations.insert("idempotentHint".into(), json!(self.idempotent));
        }
        // The library is on this Mac and nowhere else.
        annotations.insert("openWorldHint".into(), json!(false));
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": annotations,
        })
    }
}

/// What a tool call came to.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    /// The result as text a model reads. Complete by itself.
    pub text: String,
    /// The same result as data, a JSON object. Complete by itself.
    pub structured: Value,
    /// Whether the tool failed. The text then says why and what to do.
    pub is_error: bool,
}

impl ToolOutput {
    /// A result.
    pub fn ok(text: impl Into<String>, structured: Value) -> Self {
        Self {
            text: text.into(),
            structured,
            is_error: false,
        }
    }

    /// A failure the model should read.
    pub fn error(text: impl Into<String>, structured: Value) -> Self {
        Self {
            text: text.into(),
            structured,
            is_error: true,
        }
    }

    fn to_json(&self) -> Map<String, Value> {
        let mut result = Map::new();
        result.insert(
            "content".into(),
            json!([{ "type": "text", "text": self.text }]),
        );
        // Revisions from 2025-06-18 to 2025-11-25 want an object here.
        let structured = match &self.structured {
            Value::Object(_) => self.structured.clone(),
            Value::Null => json!({}),
            other => json!({ "result": other }),
        };
        result.insert("structuredContent".into(), structured);
        result.insert("isError".into(), json!(self.is_error));
        result
    }
}

/// The tools behind a [`Server`].
pub trait ToolHost {
    /// Every tool, in the order `tools/list` gives them. The order must not change.
    fn tools(&self) -> &[ToolDef];

    /// Calls the tool `name`. `None` when there is no such tool; a tool that fails
    /// returns an output with `is_error`. `client` is the name the client gave itself
    /// (its title, else its program name), when it gave one: good for a label, nothing
    /// else.
    fn call(
        &mut self,
        name: &str,
        arguments: &Map<String, Value>,
        client: Option<&str>,
    ) -> Option<ToolOutput>;
}

/// Who the server says it is.
#[derive(Clone, Debug)]
pub struct ServerInfo {
    /// The program name, `the-writer`.
    pub name: String,
    /// The name for a person, `The Writer`.
    pub title: String,
    /// The version of the app the binary belongs to.
    pub version: String,
    /// How to use the server, for the model (the `instructions` of the protocol).
    pub instructions: String,
}

impl ServerInfo {
    fn to_json(&self) -> Value {
        json!({ "name": self.name, "title": self.title, "version": self.version })
    }
}

/// An MCP server over a [`ToolHost`].
pub struct Server<H> {
    host: H,
    info: ServerInfo,
    /// The name the client gave in the handshake of a legacy revision. That handshake is
    /// the one thing the protocol ever asked a server to remember; a modern request
    /// names its client itself.
    handshake_client: Option<String>,
}

/// A JSON-RPC error on its way out.
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// The era of one request.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Legacy,
    Modern,
}

impl<H: ToolHost> Server<H> {
    /// A server that offers the tools of `host`.
    pub fn new(host: H, info: ServerInfo) -> Self {
        Self {
            host,
            info,
            handshake_client: None,
        }
    }

    /// The tools.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Every protocol version the server speaks, the modern one first.
    pub fn supported_versions() -> Vec<&'static str> {
        std::iter::once(MODERN_VERSION)
            .chain(LEGACY_VERSIONS)
            .collect()
    }

    /// Serves until `input` ends: one message per line in, at most one line out for each.
    /// Returns an error only when reading or writing fails (a closed pipe on the way out
    /// is the client going away and ends the loop quietly).
    pub fn run(&mut self, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
        for line in input.split(b'\n') {
            let line = line?;
            let Some(answer) = self.handle_bytes(&line) else {
                continue;
            };
            let written = output
                .write_all(answer.as_bytes())
                .and_then(|()| output.write_all(b"\n"))
                .and_then(|()| output.flush());
            match written {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Answers one line as it came off the wire (it need not be UTF-8).
    pub fn handle_bytes(&mut self, line: &[u8]) -> Option<String> {
        match std::str::from_utf8(line) {
            Ok(line) => self.handle_line(line),
            Err(_) => Some(
                error_response(
                    None,
                    RpcError::new(PARSE_ERROR, "Parse error: the message is not valid UTF-8"),
                )
                .to_string(),
            ),
        }
    }

    /// Answers one line. `None` when the line needs no answer (a notification, a blank
    /// line). The answer is a single line of JSON.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                return Some(
                    error_response(
                        None,
                        RpcError::new(PARSE_ERROR, format!("Parse error: {error}")),
                    )
                    .to_string(),
                );
            }
        };
        let answer = match message {
            Value::Array(messages) => {
                if messages.is_empty() {
                    Some(error_response(
                        None,
                        RpcError::new(INVALID_REQUEST, "Invalid request: an empty batch"),
                    ))
                } else {
                    let answers: Vec<Value> = messages
                        .into_iter()
                        .filter_map(|m| self.handle_message(m))
                        .collect();
                    (!answers.is_empty()).then_some(Value::Array(answers))
                }
            }
            message => self.handle_message(message),
        };
        answer.map(|answer| answer.to_string())
    }

    /// Answers one JSON-RPC message.
    fn handle_message(&mut self, message: Value) -> Option<Value> {
        let Value::Object(message) = message else {
            return Some(error_response(
                None,
                RpcError::new(
                    INVALID_REQUEST,
                    "Invalid request: a message is a JSON object",
                ),
            ));
        };
        let id = message.get("id");
        let Some(method) = message.get("method") else {
            // A response to a request of ours. The server sends none, so there is
            // nothing to match it with; a message that is neither is malformed.
            if message.contains_key("result") || message.contains_key("error") {
                return None;
            }
            return Some(error_response(
                valid_id(id),
                RpcError::new(INVALID_REQUEST, "Invalid request: no \"method\""),
            ));
        };
        let Some(method) = method.as_str() else {
            return Some(error_response(
                valid_id(id),
                RpcError::new(
                    INVALID_REQUEST,
                    "Invalid request: \"method\" must be a string",
                ),
            ));
        };
        let Some(id) = id else {
            // A notification: `notifications/initialized`, `notifications/cancelled`
            // (every request is finished before the next line is read, so there is
            // never one left to cancel) or one the server does not know. No answer.
            return None;
        };
        let Some(id) = valid_id(Some(id)) else {
            return Some(error_response(
                None,
                RpcError::new(
                    INVALID_REQUEST,
                    "Invalid request: \"id\" must be a string or a number",
                ),
            ));
        };
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error_response(
                Some(id),
                RpcError::new(
                    INVALID_REQUEST,
                    "Invalid request: \"jsonrpc\" must be \"2.0\"",
                ),
            ));
        }
        let empty = Map::new();
        let params = match message.get("params") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(params)) => params,
            Some(_) => {
                return Some(error_response(
                    Some(id),
                    RpcError::new(
                        INVALID_PARAMS,
                        "Invalid params: \"params\" must be an object",
                    ),
                ));
            }
        };
        Some(match self.handle_request(method, params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => error_response(Some(id), error),
        })
    }

    /// The era of a request and, for a modern one, whether the server can serve it.
    fn era(&self, method: &str, params: &Map<String, Value>) -> Result<Era, RpcError> {
        let meta = match params.get("_meta") {
            None | Some(Value::Null) => None,
            Some(Value::Object(meta)) => Some(meta),
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    "Invalid params: \"_meta\" must be an object",
                ));
            }
        };
        let Some(version) = meta.and_then(|meta| meta.get(META_PROTOCOL_VERSION)) else {
            if method == "server/discover" {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("Invalid params: params._meta has no \"{META_PROTOCOL_VERSION}\""),
                ));
            }
            return Ok(Era::Legacy);
        };
        let Some(version) = version.as_str() else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("Invalid params: \"{META_PROTOCOL_VERSION}\" must be a string"),
            ));
        };
        if version == MODERN_VERSION {
            let capabilities = meta.and_then(|meta| meta.get(META_CLIENT_CAPABILITIES));
            if !matches!(capabilities, Some(Value::Object(_))) {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!(
                        "Invalid params: params._meta needs \"{META_CLIENT_CAPABILITIES}\", an object"
                    ),
                ));
            }
            return Ok(Era::Modern);
        }
        if LEGACY_VERSIONS.contains(&version) {
            // A handshake-era version named per request: the server lists these
            // versions as supported, so it serves them, in their own shape.
            return Ok(if method == "server/discover" {
                Era::Modern
            } else {
                Era::Legacy
            });
        }
        Err(RpcError {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "Unsupported protocol version".into(),
            data: Some(json!({ "supported": Self::supported_versions(), "requested": version })),
        })
    }

    fn handle_request(
        &mut self,
        method: &str,
        params: &Map<String, Value>,
    ) -> Result<Value, RpcError> {
        let era = self.era(method, params)?;
        let mut result = match (method, era) {
            ("initialize", _) => {
                self.handshake_client = client_name(params.get("clientInfo"));
                return Ok(self.initialize(params));
            }
            ("ping", Era::Legacy) => Map::new(),
            ("server/discover", _) => self.discover(),
            ("tools/list", _) => self.list_tools(params, era)?,
            ("tools/call", _) => {
                let client = params
                    .get("_meta")
                    .and_then(|meta| client_name(meta.get(META_CLIENT_INFO)))
                    .or_else(|| self.handshake_client.clone());
                self.call_tool(params, client.as_deref())?
            }
            _ => {
                return Err(RpcError::new(
                    METHOD_NOT_FOUND,
                    format!("Method not found: {method}"),
                ));
            }
        };
        if era == Era::Modern {
            result.insert("resultType".into(), json!("complete"));
            result.insert(
                "_meta".into(),
                json!({ META_SERVER_INFO: self.info.to_json() }),
            );
        }
        Ok(Value::Object(result))
    }

    fn capabilities() -> Value {
        json!({ "tools": { "listChanged": false } })
    }

    /// The handshake of the legacy revisions.
    fn initialize(&self, params: &Map<String, Value>) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let version = requested
            .and_then(|requested| LEGACY_VERSIONS.iter().find(|v| **v == requested))
            .unwrap_or(&LEGACY_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": Self::capabilities(),
            "serverInfo": self.info.to_json(),
            "instructions": self.info.instructions,
        })
    }

    fn discover(&self) -> Map<String, Value> {
        let mut result = Map::new();
        result.insert(
            "supportedVersions".into(),
            json!(Self::supported_versions()),
        );
        result.insert("capabilities".into(), Self::capabilities());
        result.insert("instructions".into(), json!(self.info.instructions));
        result.insert("ttlMs".into(), json!(CACHE_TTL_MS));
        result.insert("cacheScope".into(), json!("public"));
        result
    }

    fn list_tools(
        &self,
        params: &Map<String, Value>,
        era: Era,
    ) -> Result<Map<String, Value>, RpcError> {
        // The whole list is one page, so the server never hands out a cursor.
        if let Some(cursor) = params.get("cursor").filter(|cursor| !cursor.is_null()) {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("Invalid params: unknown cursor {cursor}"),
            ));
        }
        let tools: Vec<Value> = self.host.tools().iter().map(ToolDef::to_json).collect();
        let mut result = Map::new();
        result.insert("tools".into(), Value::Array(tools));
        if era == Era::Modern {
            result.insert("ttlMs".into(), json!(CACHE_TTL_MS));
            result.insert("cacheScope".into(), json!("public"));
        }
        Ok(result)
    }

    fn call_tool(
        &mut self,
        params: &Map<String, Value>,
        client: Option<&str>,
    ) -> Result<Map<String, Value>, RpcError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "Invalid params: \"name\" must be the name of a tool",
            ));
        };
        let empty = Map::new();
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(arguments)) => arguments,
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    "Invalid params: \"arguments\" must be an object",
                ));
            }
        };
        match self.host.call(name, arguments, client) {
            Some(output) => Ok(output.to_json()),
            None => Err(RpcError::new(
                INVALID_PARAMS,
                format!("Unknown tool: {name}"),
            )),
        }
    }
}

/// The name of a client from its `Implementation` object: the title, else the name.
fn client_name(info: Option<&Value>) -> Option<String> {
    let info = info?.as_object()?;
    ["title", "name"]
        .iter()
        .filter_map(|key| info.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_string)
}

/// The id of a request when it is one the protocol allows: a string or a number.
fn valid_id(id: Option<&Value>) -> Option<Value> {
    id.filter(|id| id.is_string() || id.is_number()).cloned()
}

/// An error response. Without an id (one that could not be read) the member is left out,
/// as the protocol's schema has it (`id?: RequestId`; never null, although JSON-RPC 2.0
/// by itself asks for null there: see "Errors" at the top of this file).
fn error_response(id: Option<Value>, error: RpcError) -> Value {
    let mut body = Map::new();
    body.insert("code".into(), json!(error.code));
    body.insert("message".into(), json!(error.message));
    if let Some(data) = error.data {
        body.insert("data".into(), data);
    }
    let mut response = Map::new();
    response.insert("jsonrpc".into(), json!("2.0"));
    if let Some(id) = id {
        response.insert("id".into(), id);
    }
    response.insert("error".into(), Value::Object(body));
    Value::Object(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo {
        tools: Vec<ToolDef>,
        calls: usize,
        clients: Vec<Option<String>>,
    }

    impl Echo {
        fn new() -> Self {
            let tool = |name, effect, idempotent| ToolDef {
                name,
                title: "A tool",
                description: "Says what it was given.".into(),
                input_schema: json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
                effect,
                idempotent,
            };
            Self {
                tools: vec![
                    tool("echo", Effect::Read, true),
                    tool("add", Effect::Add, false),
                    tool("change", Effect::Change, true),
                ],
                calls: 0,
                clients: Vec::new(),
            }
        }
    }

    impl ToolHost for Echo {
        fn tools(&self) -> &[ToolDef] {
            &self.tools
        }

        fn call(
            &mut self,
            name: &str,
            arguments: &Map<String, Value>,
            client: Option<&str>,
        ) -> Option<ToolOutput> {
            self.calls += 1;
            self.clients.push(client.map(str::to_string));
            match name {
                "echo" => {
                    let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
                    Some(ToolOutput::ok(
                        format!("said: {text}"),
                        json!({ "text": text }),
                    ))
                }
                "change" => Some(ToolOutput::error(
                    "The Writer is open.",
                    json!({ "kind": "app_open" }),
                )),
                "add" => Some(ToolOutput::ok("three", json!(3))),
                _ => None,
            }
        }
    }

    fn server() -> Server<Echo> {
        Server::new(
            Echo::new(),
            ServerInfo {
                name: "the-writer".into(),
                title: "The Writer".into(),
                version: "9.9.9".into(),
                instructions: "Use the tools.".into(),
            },
        )
    }

    fn ask(server: &mut Server<Echo>, message: Value) -> Value {
        let answer = server.handle_line(&message.to_string()).expect("an answer");
        assert!(!answer.contains('\n'), "an answer is one line");
        serde_json::from_str(&answer).expect("the answer is JSON")
    }

    fn modern_meta() -> Value {
        json!({ META_PROTOCOL_VERSION: MODERN_VERSION, META_CLIENT_CAPABILITIES: {} })
    }

    #[test]
    fn the_handshake_echoes_a_version_it_speaks_and_offers_its_newest_otherwise() {
        let mut server = server();
        for version in LEGACY_VERSIONS {
            let answer = ask(
                &mut server,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": version, "capabilities": {},
                    "clientInfo": { "name": "t", "version": "1" } } }),
            );
            assert_eq!(answer["result"]["protocolVersion"], version);
            assert_eq!(answer["result"]["serverInfo"]["name"], "the-writer");
            assert_eq!(answer["result"]["serverInfo"]["version"], "9.9.9");
            assert_eq!(
                answer["result"]["capabilities"]["tools"]["listChanged"],
                false
            );
            assert_eq!(answer["result"]["instructions"], "Use the tools.");
            assert!(answer["result"].get("resultType").is_none());
        }
        for version in ["1999-01-01", MODERN_VERSION] {
            let answer = ask(
                &mut server,
                json!({ "jsonrpc": "2.0", "id": "a", "method": "initialize",
                    "params": { "protocolVersion": version } }),
            );
            assert_eq!(answer["id"], "a");
            assert_eq!(answer["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
        }
    }

    #[test]
    fn notifications_and_blank_lines_get_no_answer() {
        let mut server = server();
        assert_eq!(server.handle_line(""), None);
        assert_eq!(server.handle_line("   "), None);
        for method in [
            "notifications/initialized",
            "notifications/cancelled",
            "notifications/unknown",
        ] {
            let line = json!({ "jsonrpc": "2.0", "method": method, "params": { "requestId": 1 } });
            assert_eq!(server.handle_line(&line.to_string()), None);
        }
        // A response to a request the server never sent.
        assert_eq!(
            server.handle_line(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#),
            None
        );
    }

    #[test]
    fn legacy_requests_have_the_legacy_shape() {
        let mut server = server();
        let ping = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
        );
        assert_eq!(ping, json!({ "jsonrpc": "2.0", "id": 1, "result": {} }));

        let list = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        );
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert!(list["result"].get("resultType").is_none());
        assert!(list["result"].get("ttlMs").is_none());
        assert_eq!(tools[0]["name"], "echo");
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert!(tools[0]["annotations"].get("destructiveHint").is_none());
        assert_eq!(tools[0]["annotations"]["openWorldHint"], false);
        assert_eq!(tools[1]["annotations"]["readOnlyHint"], false);
        assert_eq!(tools[1]["annotations"]["destructiveHint"], false);
        assert_eq!(tools[1]["annotations"]["idempotentHint"], false);
        assert_eq!(tools[2]["annotations"]["destructiveHint"], true);
        assert_eq!(tools[2]["annotations"]["idempotentHint"], true);

        let call = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "echo", "arguments": { "text": "hi" } } }),
        );
        assert_eq!(
            call["result"]["content"],
            json!([{ "type": "text", "text": "said: hi" }])
        );
        assert_eq!(call["result"]["structuredContent"], json!({ "text": "hi" }));
        assert_eq!(call["result"]["isError"], false);
        assert!(call["result"].get("resultType").is_none());
    }

    #[test]
    fn a_tool_failure_is_a_result_and_structured_content_is_always_an_object() {
        let mut server = server();
        let refused = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "change" } }),
        );
        assert!(refused.get("error").is_none());
        assert_eq!(refused["result"]["isError"], true);
        assert_eq!(
            refused["result"]["content"][0]["text"],
            "The Writer is open."
        );
        assert_eq!(refused["result"]["structuredContent"]["kind"], "app_open");

        let number = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "add", "arguments": null } }),
        );
        assert_eq!(
            number["result"]["structuredContent"],
            json!({ "result": 3 })
        );
    }

    #[test]
    fn modern_requests_carry_their_version_and_get_the_modern_shape() {
        let mut server = server();
        let discover = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": { "_meta": modern_meta() } }),
        );
        let result = &discover["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"][0], MODERN_VERSION);
        assert_eq!(result["supportedVersions"].as_array().unwrap().len(), 5);
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(result["instructions"], "Use the tools.");
        assert_eq!(result["ttlMs"], 3_600_000);
        assert_eq!(result["cacheScope"], "public");
        assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "the-writer");
        assert_eq!(result["_meta"][META_SERVER_INFO]["version"], "9.9.9");

        let list = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": { "_meta": modern_meta() } }),
        );
        assert_eq!(list["result"]["resultType"], "complete");
        assert_eq!(list["result"]["ttlMs"], 3_600_000);
        assert_eq!(list["result"]["cacheScope"], "public");
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 3);
        assert_eq!(
            list["result"]["_meta"][META_SERVER_INFO]["title"],
            "The Writer"
        );

        let call = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "echo", "arguments": { "text": "x" }, "_meta": modern_meta() } }),
        );
        assert_eq!(call["result"]["resultType"], "complete");
        assert_eq!(call["result"]["content"][0]["text"], "said: x");
        assert_eq!(
            call["result"]["_meta"][META_SERVER_INFO]["name"],
            "the-writer"
        );

        // `ping` left the protocol with the handshake.
        let ping = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "ping", "params": { "_meta": modern_meta() } }),
        );
        assert_eq!(ping["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn a_version_the_server_does_not_speak_is_refused_with_the_list_of_those_it_does() {
        let mut server = server();
        let answer = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "_meta": {
                META_PROTOCOL_VERSION: "1900-01-01", META_CLIENT_CAPABILITIES: {} } } }),
        );
        assert_eq!(answer["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(answer["error"]["data"]["requested"], "1900-01-01");
        let supported = answer["error"]["data"]["supported"].as_array().unwrap();
        assert_eq!(supported[0], MODERN_VERSION);
        assert!(supported.contains(&json!("2025-11-25")));
        assert_eq!(server.host().calls, 0);
    }

    #[test]
    fn modern_requests_without_their_required_fields_are_malformed() {
        let mut server = server();
        let no_capabilities = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list",
                "params": { "_meta": { META_PROTOCOL_VERSION: MODERN_VERSION } } }),
        );
        assert_eq!(no_capabilities["error"]["code"], INVALID_PARAMS);
        let no_version = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "server/discover" }),
        );
        assert_eq!(no_version["error"]["code"], INVALID_PARAMS);
        let bad_version = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list",
                "params": { "_meta": { META_PROTOCOL_VERSION: 7 } } }),
        );
        assert_eq!(bad_version["error"]["code"], INVALID_PARAMS);
        let bad_meta = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": { "_meta": "x" } }),
        );
        assert_eq!(bad_meta["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn a_handshake_version_named_per_request_is_served_in_its_own_shape() {
        let mut server = server();
        let meta = json!({ META_PROTOCOL_VERSION: "2025-11-25" });
        let list = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "_meta": meta } }),
        );
        assert!(list["result"]["tools"].is_array());
        assert!(list["result"].get("resultType").is_none());
        let discover = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "server/discover", "params": { "_meta": meta } }),
        );
        assert_eq!(discover["result"]["supportedVersions"][0], MODERN_VERSION);
    }

    #[test]
    fn malformed_input_is_answered_with_the_right_error() {
        let mut server = server();
        let parse: Value = serde_json::from_str(&server.handle_line("{not json").unwrap()).unwrap();
        assert_eq!(parse["error"]["code"], PARSE_ERROR);
        // MCP's schema: `id?: RequestId`, a string or a number. Left out, never null
        // (JSON-RPC 2.0 alone would have null here; a client that checks against MCP's
        // schema cannot read that).
        assert!(
            parse.get("id").is_none(),
            "no id could be read, so none is sent"
        );
        assert!(parse.as_object().unwrap().keys().all(|key| key != "id"));
        assert_eq!(parse["jsonrpc"], "2.0");

        let utf8: Value =
            serde_json::from_str(&server.handle_bytes(&[0xff, 0xfe, b'{']).unwrap()).unwrap();
        assert_eq!(utf8["error"]["code"], PARSE_ERROR);

        for message in [
            json!(5),
            json!("tools/list"),
            json!({ "jsonrpc": "2.0", "id": 1 }),
            json!([]),
        ] {
            let answer = ask(&mut server, message.clone());
            assert_eq!(answer["error"]["code"], INVALID_REQUEST, "{message}");
        }
        let no_method_id = ask(&mut server, json!({ "jsonrpc": "2.0", "id": 1 }));
        assert_eq!(no_method_id["id"], 1);

        let null_id = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }),
        );
        assert_eq!(null_id["error"]["code"], INVALID_REQUEST);
        assert!(null_id.get("id").is_none());

        let method = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": 5 }),
        );
        assert_eq!(method["error"]["code"], INVALID_REQUEST);

        let version = ask(
            &mut server,
            json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
        );
        assert_eq!(version["error"]["code"], INVALID_REQUEST);
        assert_eq!(version["id"], 1);

        let params = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": [1] }),
        );
        assert_eq!(params["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn unknown_methods_tools_and_cursors() {
        let mut server = server();
        for method in [
            "resources/list",
            "prompts/list",
            "logging/setLevel",
            "nonsense",
        ] {
            let answer = ask(
                &mut server,
                json!({ "jsonrpc": "2.0", "id": 9, "method": method }),
            );
            assert_eq!(answer["error"]["code"], METHOD_NOT_FOUND, "{method}");
            assert_eq!(
                answer["error"]["message"],
                format!("Method not found: {method}")
            );
            assert_eq!(answer["id"], 9);
        }
        let tool = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "nope" } }),
        );
        assert_eq!(tool["error"]["code"], INVALID_PARAMS);
        assert_eq!(tool["error"]["message"], "Unknown tool: nope");
        let no_name = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call" }),
        );
        assert_eq!(no_name["error"]["code"], INVALID_PARAMS);
        let arguments = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "echo", "arguments": [] } }),
        );
        assert_eq!(arguments["error"]["code"], INVALID_PARAMS);
        let cursor = ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "cursor": "abc" } }),
        );
        assert_eq!(cursor["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn the_client_names_itself_in_the_handshake_or_in_each_modern_request() {
        let mut server = server();
        let call = |meta: Value| {
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "echo", "_meta": meta } })
        };
        ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "echo" } }),
        );
        ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": { "name": "claude-code", "title": "Claude Code", "version": "2" } } }),
        );
        ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "echo" } }),
        );
        let mut meta = modern_meta();
        meta[META_CLIENT_INFO] = json!({ "name": "codex", "version": "1" });
        ask(&mut server, call(meta));
        ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-11-25", "clientInfo": { "name": " ", "version": "2" } } }),
        );
        ask(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "echo" } }),
        );
        assert_eq!(
            server.host().clients,
            vec![
                None,
                Some("Claude Code".to_string()),
                Some("codex".to_string()),
                None
            ]
        );
    }

    #[test]
    fn a_batch_is_answered_with_a_batch() {
        let mut server = server();
        let batch = json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/initialized" },
            { "jsonrpc": "2.0", "id": 2, "method": "nonsense" },
        ]);
        let answer = ask(&mut server, batch);
        let answers = answer.as_array().unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0]["id"], 1);
        assert_eq!(answers[1]["error"]["code"], METHOD_NOT_FOUND);
        let only_notifications =
            json!([{ "jsonrpc": "2.0", "method": "notifications/initialized" }]);
        assert_eq!(server.handle_line(&only_notifications.to_string()), None);
    }

    #[test]
    fn run_reads_lines_until_the_input_ends() {
        let mut server = server();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "\r\n\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "garbage\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"a\nb"}}}"#,
        );
        let mut output = Vec::new();
        server.run(input.as_bytes(), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "{output}");
        assert!(output.ends_with('\n'));
        let last: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(last["result"]["content"][0]["text"], "said: a\nb");
    }
}
