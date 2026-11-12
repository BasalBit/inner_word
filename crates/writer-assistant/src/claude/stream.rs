//! The lines of the program's `stream-json` output, read into what the assistant
//! needs of them, and the lines written to its input.
//!
//! The shapes are those of Claude Code 2.1.322, read from its documentation
//! (code.claude.com/docs/en/headless and the Agent SDK reference, `SDKMessage`) and
//! confirmed against the program on 2026-11-12; the recordings are in `tests/fixtures`.
//! A line this module does not know is [`Line::Other`], and a line that is not JSON is
//! [`Line::NotJson`]: neither ends a conversation.

use serde_json::{Value, json};

use crate::backend::{AssistantError, Usage, one_line};

/// A tool call the model makes.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolUse {
    /// The id the result will carry.
    pub id: String,
    /// The tool as the program names it: `mcp__the-writer__read_document`.
    pub name: String,
    /// Its arguments (empty while they are still arriving).
    pub input: Value,
}

/// The result of a tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    /// The call it answers.
    pub tool_use_id: String,
    /// Whether the tool failed or was refused.
    pub is_error: bool,
    /// What the tool returned as data (`structuredContent`), or `null`.
    pub structured: Value,
}

/// How a turn ended, as the program's `result` line says it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultLine {
    /// `success`, `error_during_execution`, `error_max_turns`, ...
    pub subtype: String,
    /// Whether the turn failed.
    pub is_error: bool,
    /// The final text (also the error's text when `is_error`).
    pub text: Option<String>,
    /// The messages of a failed run.
    pub errors: Vec<String>,
    /// `completed`, `aborted_streaming`, `api_error`, ...
    pub terminal_reason: Option<String>,
    /// The HTTP status of the service's error, when there was one.
    pub api_error_status: Option<u16>,
    /// The tokens of this turn.
    pub input_tokens: Option<u64>,
    /// The tokens written in this turn.
    pub output_tokens: Option<u64>,
    /// The estimated cost of the whole process so far (it accumulates over the turns).
    pub total_cost_usd: Option<f64>,
    /// How long the turn took.
    pub duration_ms: Option<u64>,
}

impl ResultLine {
    /// Whether the turn ended because it was interrupted.
    pub fn was_aborted(&self) -> bool {
        self.terminal_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("aborted"))
    }
}

/// One line of the program's output.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// `system/init`: a turn begins. `model` is the model that answers, `tools` what
    /// the model is offered, `failed_servers` the MCP servers that did not start.
    Init {
        model: Option<String>,
        version: Option<String>,
        tools: Vec<String>,
        failed_servers: Vec<String>,
    },
    /// `system/api_retry`: a request failed and is tried again.
    Retry {
        attempt: u64,
        max: u64,
        /// The HTTP status; `None` when there was no answer at all (no connection).
        status: Option<u16>,
        error: String,
    },
    /// `system/status` with `compacting`: the program shortens a long conversation.
    Compacting,
    /// `stream_event/message_start`: the model begins a message.
    MessageStart,
    /// `stream_event/content_block_start` of a text block.
    TextStart,
    /// `stream_event/content_block_start` of a thinking block.
    ThinkingStart,
    /// `stream_event/content_block_start` of a tool call (its arguments follow).
    ToolStart(ToolUse),
    /// `stream_event/content_block_delta` with text.
    TextDelta(String),
    /// A complete message of the model: its text blocks and tool calls. `error` is the
    /// category when the message is the program's report of a failed request
    /// (`authentication_failed`, `rate_limit`, ...), and the text is then the report.
    Assistant {
        texts: Vec<String>,
        tools: Vec<ToolUse>,
        error: Option<String>,
        synthetic: bool,
    },
    /// A `user` line carrying the results of tool calls.
    ToolResults(Vec<ToolResult>),
    /// `rate_limit_event`: where the user's allowance stands.
    RateLimit {
        /// `allowed`, `allowed_warning`, `rejected`.
        status: String,
        /// When the window resets, in seconds since 1970.
        resets_at: Option<i64>,
        /// Which window: `five_hour`, `seven_day`, ...
        kind: Option<String>,
    },
    /// `result`: the turn is over.
    Result(ResultLine),
    /// `control_response`: the answer to a request this side made.
    ControlResponse {
        request_id: String,
        ok: bool,
        error: Option<String>,
        response: Value,
    },
    /// `control_request`: the program asks this side something.
    ControlRequest { request_id: String, subtype: String },
    /// A JSON line of a kind the assistant has no use for, or does not know.
    Other,
    /// A line that is not a JSON object.
    NotJson,
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn status_of(value: &Value, key: &str) -> Option<u16> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
}

fn tool_use(block: &Value) -> Option<ToolUse> {
    Some(ToolUse {
        id: text_of(block, "id")?,
        name: text_of(block, "name")?,
        input: block.get("input").cloned().unwrap_or(Value::Null),
    })
}

/// Reads one line of the program's output.
pub fn parse_line(line: &str) -> Line {
    let line = line.trim();
    if !line.starts_with('{') {
        return Line::NotJson;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Line::NotJson;
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
    match kind {
        "system" => match subtype {
            "init" => Line::Init {
                model: text_of(&value, "model"),
                version: text_of(&value, "claude_code_version"),
                tools: value
                    .get("tools")
                    .and_then(Value::as_array)
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|tool| tool.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                failed_servers: value
                    .get("mcp_servers")
                    .and_then(Value::as_array)
                    .map(|servers| {
                        servers
                            .iter()
                            .filter(|server| {
                                // "connected" and "pending" are fine; so is a state this
                                // version does not know.
                                matches!(
                                    server.get("status").and_then(Value::as_str),
                                    Some("failed") | Some("needs-auth") | Some("disabled")
                                )
                            })
                            .filter_map(|server| text_of(server, "name"))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "api_retry" => Line::Retry {
                attempt: value.get("attempt").and_then(Value::as_u64).unwrap_or(0),
                max: value
                    .get("max_retries")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                status: status_of(&value, "error_status"),
                error: text_of(&value, "error").unwrap_or_default(),
            },
            "status" if value.get("status").and_then(Value::as_str) == Some("compacting") => {
                Line::Compacting
            }
            _ => Line::Other,
        },
        "stream_event" => {
            // Text of a subagent is not the answer (there are no subagents here, but a
            // line that says it belongs to one is not taken for the answer).
            if value
                .get("parent_tool_use_id")
                .is_some_and(|id| !id.is_null())
            {
                return Line::Other;
            }
            let Some(event) = value.get("event") else {
                return Line::Other;
            };
            match event.get("type").and_then(Value::as_str).unwrap_or("") {
                "message_start" => Line::MessageStart,
                "content_block_start" => {
                    let block = event.get("content_block").unwrap_or(&Value::Null);
                    match block.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text" => Line::TextStart,
                        "thinking" | "redacted_thinking" => Line::ThinkingStart,
                        "tool_use" | "server_tool_use" | "mcp_tool_use" => {
                            tool_use(block).map(Line::ToolStart).unwrap_or(Line::Other)
                        }
                        _ => Line::Other,
                    }
                }
                "content_block_delta" => {
                    let delta = event.get("delta").unwrap_or(&Value::Null);
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => match text_of(delta, "text") {
                            Some(text) if !text.is_empty() => Line::TextDelta(text),
                            _ => Line::Other,
                        },
                        _ => Line::Other,
                    }
                }
                _ => Line::Other,
            }
        }
        "assistant" => {
            if value
                .get("parent_tool_use_id")
                .is_some_and(|id| !id.is_null())
            {
                return Line::Other;
            }
            let message = value.get("message").unwrap_or(&Value::Null);
            let mut texts = Vec::new();
            let mut tools = Vec::new();
            if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = text_of(block, "text") {
                                texts.push(text);
                            }
                        }
                        Some("tool_use") => tools.extend(tool_use(block)),
                        _ => {}
                    }
                }
            } else if let Some(text) = message.get("content").and_then(Value::as_str) {
                texts.push(text.to_string());
            }
            let synthetic = message.get("model").and_then(Value::as_str) == Some("<synthetic>")
                || value.get("is_api_error_message").and_then(Value::as_bool) == Some(true);
            Line::Assistant {
                texts,
                tools,
                error: text_of(&value, "error"),
                synthetic,
            }
        }
        "user" => {
            let mut results = Vec::new();
            let structured = value
                .get("tool_use_result")
                .and_then(|result| result.get("structuredContent"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(blocks) = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
            {
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result")
                        && let Some(id) = text_of(block, "tool_use_id")
                    {
                        results.push(ToolResult {
                            tool_use_id: id,
                            is_error: block.get("is_error").and_then(Value::as_bool) == Some(true),
                            structured: structured.clone(),
                        });
                    }
                }
            }
            if results.is_empty() {
                Line::Other
            } else {
                Line::ToolResults(results)
            }
        }
        "rate_limit_event" => {
            let info = value.get("rate_limit_info").unwrap_or(&Value::Null);
            Line::RateLimit {
                status: text_of(info, "status").unwrap_or_default(),
                resets_at: info.get("resetsAt").and_then(Value::as_i64),
                kind: text_of(info, "rateLimitType"),
            }
        }
        "result" => {
            let usage = value.get("usage").unwrap_or(&Value::Null);
            let count = |key: &str| usage.get(key).and_then(Value::as_u64);
            let input = match (
                count("input_tokens"),
                count("cache_creation_input_tokens"),
                count("cache_read_input_tokens"),
            ) {
                (None, None, None) => None,
                (plain, written, read) => {
                    Some(plain.unwrap_or(0) + written.unwrap_or(0) + read.unwrap_or(0))
                }
            };
            Line::Result(ResultLine {
                subtype: subtype.to_string(),
                is_error: value.get("is_error").and_then(Value::as_bool) == Some(true)
                    || subtype.starts_with("error"),
                text: text_of(&value, "result"),
                errors: value
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(|errors| {
                        errors
                            .iter()
                            .filter_map(|error| error.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                terminal_reason: text_of(&value, "terminal_reason"),
                api_error_status: status_of(&value, "api_error_status"),
                input_tokens: input,
                output_tokens: count("output_tokens"),
                total_cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
                duration_ms: value.get("duration_ms").and_then(Value::as_u64),
            })
        }
        "control_response" => {
            let response = value.get("response").unwrap_or(&Value::Null);
            let Some(request_id) = text_of(response, "request_id") else {
                return Line::Other;
            };
            Line::ControlResponse {
                request_id,
                ok: response.get("subtype").and_then(Value::as_str) == Some("success"),
                error: text_of(response, "error"),
                response: response.get("response").cloned().unwrap_or(Value::Null),
            }
        }
        "control_request" => {
            let Some(request_id) = text_of(&value, "request_id") else {
                return Line::Other;
            };
            Line::ControlRequest {
                request_id,
                subtype: value
                    .get("request")
                    .and_then(|request| text_of(request, "subtype"))
                    .unwrap_or_default(),
            }
        }
        _ => Line::Other,
    }
}

// ----- what is written to the program -------------------------------------------------

/// A user message, to be delivered exactly as written.
///
/// `client_composed` is what makes it so: without it the program expands every
/// `@path` in the text into the contents of that file (read from the disk, whatever
/// `--tools` says) and runs text that begins with `/` as a command. A writer's text
/// is full of both. Seen on Claude Code 2.1.322: with the field the message reaches
/// the model character for character.
pub fn user_message(text: &str) -> String {
    json!({
        "type": "user",
        "client_composed": true,
        "message": {
            "role": "user",
            "content": [{ "type": "text", "text": text }],
        },
        "parent_tool_use_id": null,
    })
    .to_string()
}

/// A request to the program itself: `initialize`, `interrupt`, `set_model`,
/// `apply_flag_settings`. `fields` are the other members of the request.
pub fn control_request(request_id: &str, subtype: &str, fields: Value) -> String {
    let mut request = json!({ "subtype": subtype });
    if let (Some(request), Some(fields)) = (request.as_object_mut(), fields.as_object()) {
        for (key, value) in fields {
            request.insert(key.clone(), value.clone());
        }
    }
    json!({ "type": "control_request", "request_id": request_id, "request": request }).to_string()
}

/// The answer to a request of the program this side has no part in: an error, so
/// that the program does not wait. A request to use a tool is answered with a refusal.
pub fn control_refusal(request_id: &str, subtype: &str) -> String {
    if subtype == "can_use_tool" {
        return json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "deny",
                    "message": "The writing assistant only reads; it cannot use this tool.",
                },
            },
        })
        .to_string();
    }
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": format!("The Writer does not answer {subtype} requests."),
        },
    })
    .to_string()
}

// ----- errors ---------------------------------------------------------------------------

/// Whether a report reads like a failed connection rather than an answer of the
/// service.
fn reads_like_no_connection(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "econnrefused",
        "enotfound",
        "etimedout",
        "econnreset",
        "eai_again",
        "ehostunreach",
        "enetunreach",
        "connection refused",
        "connection error",
        "connection reset",
        "connection closed",
        "unable to connect",
        "could not connect",
        "network",
        "fetch failed",
        "getaddrinfo",
        "socket hang up",
        "offline",
        "dns",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|sign| text.contains(sign))
}

/// Whether a report says that nobody is signed in.
fn reads_like_signed_out(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("not logged in")
        || text.contains("/login")
        || text.contains("failed to authenticate")
        || text.contains("authentication_error")
        || text.contains("invalid api key")
        || text.contains("oauth token has expired")
}

/// The program's report of a failed turn as one of the kinds the interface acts on.
///
/// `category` is the `error` of the program's message (`authentication_failed`,
/// `rate_limit`, `overloaded`, `server_error`, `model_not_found`, `invalid_request`,
/// `billing_error`, ..., from the documented list), `status` the HTTP status of the
/// service's answer, `text` the program's sentence, `resets_at` the reset time of a
/// `rate_limit_event` that came with it, and `no_answer` whether the program reported
/// retries that got no answer from the service at all.
pub fn classify(
    category: Option<&str>,
    status: Option<u16>,
    text: &str,
    resets_at: Option<i64>,
    no_answer: bool,
) -> AssistantError {
    let detail = one_line(text, 400);
    match category.unwrap_or("") {
        "authentication_failed" | "oauth_org_not_allowed" => return AssistantError::NotSignedIn,
        "rate_limit" | "billing_error" => {
            return AssistantError::UsageLimit { resets_at, detail };
        }
        "overloaded" => return AssistantError::Unavailable { detail },
        "model_not_found" | "invalid_request" => return AssistantError::Rejected { detail },
        _ => {}
    }
    match status {
        Some(401) | Some(403) => return AssistantError::NotSignedIn,
        Some(429) => return AssistantError::UsageLimit { resets_at, detail },
        Some(404) | Some(400) | Some(413) | Some(422) => {
            return AssistantError::Rejected { detail };
        }
        Some(status) if status >= 500 => return AssistantError::Unavailable { detail },
        _ => {}
    }
    if reads_like_signed_out(text) {
        return AssistantError::NotSignedIn;
    }
    if no_answer || reads_like_no_connection(text) {
        return AssistantError::Network { detail };
    }
    if category == Some("server_error") {
        return AssistantError::Unavailable { detail };
    }
    AssistantError::Other { detail }
}

/// The usage of a turn from its `result` line. `cost_before` is the process's total
/// cost before the turn: the program reports the total, not the turn's share.
pub fn usage_of(result: &ResultLine, cost_before: f64) -> Option<Usage> {
    let usage = Usage {
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        cost_usd: result
            .total_cost_usd
            .map(|total| (total - cost_before).max(0.0)),
        duration_ms: result.duration_ms,
    };
    (usage != Usage::default()).then_some(usage)
}
