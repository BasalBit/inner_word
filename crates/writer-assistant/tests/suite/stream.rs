//! The program's output lines, read from the recordings, and its errors as kinds.

use writer_assistant::AssistantError;
use writer_assistant::claude::stream::{
    Line, classify, control_refusal, control_request, parse_line, user_message,
};

use super::support::fixture;

fn lines(name: &str) -> Vec<Line> {
    std::fs::read_to_string(fixture(name))
        .unwrap()
        .lines()
        .map(parse_line)
        .collect()
}

#[test]
fn every_line_of_every_recording_is_read_without_complaint() {
    let folder = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut read = 0;
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            assert!(
                parse_line(line) != Line::NotJson,
                "{}: {line}",
                path.display()
            );
            read += 1;
        }
    }
    assert!(read > 200, "{read}");
}

#[test]
fn a_recorded_turn_reads_as_init_text_and_result() {
    let lines = lines("quick-turn.jsonl");
    assert!(
        matches!(&lines[0], Line::Init { model: Some(model), version: Some(version), tools, failed_servers } if model == "claude-haiku-5-5" && version == "2.1.322" && tools.is_empty() && failed_servers.is_empty())
    );
    assert!(lines.contains(&Line::MessageStart));
    assert!(lines.contains(&Line::TextStart));
    assert!(lines.contains(&Line::TextDelta("HERON".to_string())));
    assert!(lines.iter().any(|line| matches!(line, Line::Assistant { texts, error: None, synthetic: false, .. } if texts == &["HERON".to_string()])));
    assert!(lines.iter().any(|line| matches!(line, Line::RateLimit { status, resets_at: Some(_), kind: Some(kind) } if status == "allowed" && kind == "seven_day")));
    let Some(Line::Result(result)) = lines.last() else {
        panic!("the last line is the result");
    };
    assert!(!result.is_error);
    assert_eq!(result.subtype, "success");
    assert_eq!(result.text.as_deref(), Some("HERON"));
    assert_eq!(result.terminal_reason.as_deref(), Some("completed"));
    assert_eq!(
        result.input_tokens,
        Some(639),
        "plain, written to and read from the cache"
    );
    assert_eq!(result.output_tokens, Some(5));
    assert!(result.total_cost_usd.unwrap() > 0.0);
    assert!(!result.was_aborted());
}

#[test]
fn a_recorded_tool_call_reads_as_the_call_and_its_result() {
    let lines = lines("side-tool-turn.jsonl");
    assert!(
        matches!(&lines[0], Line::Init { tools, .. } if tools.len() == 12 && tools.iter().all(|tool| tool.starts_with("mcp__the-writer__")))
    );
    assert!(lines.iter().any(|line| matches!(line, Line::ToolStart(call) if call.name == "mcp__the-writer__read_document")));
    assert!(lines.iter().any(|line| matches!(
        line,
        Line::Assistant { tools, .. } if tools.len() == 1 && tools[0].input == serde_json::json!({"document": "doc:1"})
    )));
    let result = lines
        .iter()
        .find_map(|line| match line {
            Line::ToolResults(results) => Some(&results[0]),
            _ => None,
        })
        .expect("the tool answered");
    assert!(!result.is_error);
    assert_eq!(
        result.structured["document"]["title"], "Chapter 2",
        "the data the server returned"
    );
    assert!(lines.contains(&Line::ThinkingStart));
    // The withheld tool: its result is an error.
    let withheld = lines_of_results("withheld-tool-turn.jsonl");
    assert!(withheld[0]);
}

fn lines_of_results(name: &str) -> Vec<bool> {
    lines(name)
        .iter()
        .filter_map(|line| match line {
            Line::ToolResults(results) => Some(results[0].is_error),
            _ => None,
        })
        .collect()
}

#[test]
fn a_recorded_interrupt_reads_as_its_answer_and_an_aborted_result() {
    let lines = lines("interrupted-turn.jsonl");
    assert!(lines.iter().any(|line| matches!(line, Line::ControlResponse { request_id, ok: true, .. } if request_id == "int-1")));
    let Some(Line::Result(result)) = lines.last() else {
        panic!("the last line is the result");
    };
    assert!(result.was_aborted());
    assert!(result.is_error);
    assert_eq!(result.subtype, "error_during_execution");
    // The same in the middle of three turns of one process.
    let three = lines_results("three-turns.jsonl");
    assert_eq!(three, [false, true, false]);
}

fn lines_results(name: &str) -> Vec<bool> {
    lines(name)
        .iter()
        .filter_map(|line| match line {
            Line::Result(result) => Some(result.was_aborted()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_recorded_failures_read_as_reports_with_their_category_and_status() {
    for (name, category, status) in [
        ("signed-out-turn.jsonl", "authentication_failed", None),
        (
            "error-sign-in-expired.jsonl",
            "authentication_failed",
            Some(401),
        ),
        ("error-usage-limit.jsonl", "rate_limit", Some(429)),
        ("error-overloaded.jsonl", "server_error", Some(529)),
        ("error-server.jsonl", "server_error", Some(500)),
        ("error-model.jsonl", "model_not_found", Some(404)),
        ("error-no-connection.jsonl", "server_error", None),
        ("error-cut-stream.jsonl", "unknown", None),
    ] {
        let lines = lines(name);
        let reported = lines.iter().find_map(|line| match line {
            Line::Assistant {
                error: Some(error),
                synthetic,
                ..
            } => Some((error.as_str(), *synthetic)),
            _ => None,
        });
        assert_eq!(reported, Some((category, true)), "{name}");
        let Some(Line::Result(result)) = lines.last() else {
            panic!("{name}: the last line is the result");
        };
        assert!(result.is_error, "{name}");
        assert_eq!(result.api_error_status, status, "{name}");
        assert_eq!(
            result.terminal_reason.as_deref(),
            Some("api_error"),
            "{name}"
        );
    }
    let retries = lines("error-no-connection-retries.jsonl");
    assert!(retries.contains(&Line::Retry {
        attempt: 1,
        max: 4,
        status: None,
        error: "unknown".to_string()
    }));
    let retries = lines("error-overloaded-retries.jsonl");
    assert!(retries.contains(&Line::Retry {
        attempt: 4,
        max: 4,
        status: Some(529),
        error: "overloaded".to_string()
    }));
    let limit = lines("error-usage-limit.jsonl");
    assert!(limit.iter().any(|line| matches!(line, Line::RateLimit { status, resets_at: Some(1794501407), kind: Some(kind) } if status == "rejected" && kind == "five_hour")));
}

#[test]
fn what_is_not_a_line_of_the_protocol_is_told_apart() {
    assert_eq!(parse_line(""), Line::NotJson);
    assert_eq!(parse_line("npm WARN deprecated"), Line::NotJson);
    assert_eq!(parse_line("{ broken"), Line::NotJson);
    assert_eq!(parse_line("[1, 2, 3]"), Line::NotJson);
    assert_eq!(parse_line("{}"), Line::Other);
    assert_eq!(
        parse_line(r#"{"type":"a_kind_from_the_future","x":1}"#),
        Line::Other
    );
    assert_eq!(
        parse_line(r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":50}"#),
        Line::Other
    );
    assert_eq!(
        parse_line(r#"{"type":"system","subtype":"status","status":"requesting"}"#),
        Line::Other
    );
    assert_eq!(
        parse_line(r#"{"type":"system","subtype":"status","status":"compacting"}"#),
        Line::Compacting
    );
    assert_eq!(
        parse_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}"#
        ),
        Line::Other
    );
    // Text of a subagent is not the answer.
    assert_eq!(
        parse_line(
            r#"{"type":"stream_event","parent_tool_use_id":"toolu_1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"inner"}}}"#
        ),
        Line::Other
    );
    assert_eq!(
        parse_line(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#
        ),
        Line::Other
    );
    assert!(matches!(
        parse_line(r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#),
        Line::ControlRequest { request_id, subtype } if request_id == "r1" && subtype == "can_use_tool"
    ));
    assert!(matches!(
        parse_line(r#"{"type":"control_response","response":{"subtype":"error","request_id":"r2","error":"Unsupported control request subtype: x"}}"#),
        Line::ControlResponse { ok: false, error: Some(error), .. } if error.contains("Unsupported")
    ));
}

#[test]
fn what_is_written_to_the_program_has_the_shapes_it_reads() {
    let message: serde_json::Value =
        serde_json::from_str(&user_message("Is @notes.md right?\n\"Yes\"")).unwrap();
    assert_eq!(
        message,
        serde_json::json!({
            "type": "user",
            "client_composed": true,
            "message": {"role": "user", "content": [{"type": "text", "text": "Is @notes.md right?\n\"Yes\""}]},
            "parent_tool_use_id": null,
        })
    );
    assert!(
        !user_message("line one\nline two").contains('\n'),
        "one message is one line"
    );
    let request: serde_json::Value = serde_json::from_str(&control_request(
        "w-1",
        "set_model",
        serde_json::json!({"model": "sonnet"}),
    ))
    .unwrap();
    assert_eq!(
        request,
        serde_json::json!({"type": "control_request", "request_id": "w-1", "request": {"subtype": "set_model", "model": "sonnet"}})
    );
    let bare: serde_json::Value = serde_json::from_str(&control_request(
        "w-2",
        "interrupt",
        serde_json::Value::Null,
    ))
    .unwrap();
    assert_eq!(bare["request"], serde_json::json!({"subtype": "interrupt"}));
    // A question of the program is answered so that it does not wait: a tool is refused.
    let denied: serde_json::Value =
        serde_json::from_str(&control_refusal("r1", "can_use_tool")).unwrap();
    assert_eq!(denied["response"]["response"]["behavior"], "deny");
    assert_eq!(denied["response"]["request_id"], "r1");
    let other: serde_json::Value =
        serde_json::from_str(&control_refusal("r2", "hook_callback")).unwrap();
    assert_eq!(other["response"]["subtype"], "error");
}

#[test]
fn a_report_of_the_program_becomes_one_kind_of_error() {
    let kind = |category: Option<&str>, status: Option<u16>, text: &str, no_answer: bool| {
        classify(category, status, text, None, no_answer)
    };
    assert_eq!(
        kind(
            Some("authentication_failed"),
            None,
            "Not logged in · Please run /login",
            false
        ),
        AssistantError::NotSignedIn
    );
    assert_eq!(
        kind(Some("oauth_org_not_allowed"), Some(403), "x", false),
        AssistantError::NotSignedIn
    );
    assert_eq!(
        kind(None, Some(401), "x", false),
        AssistantError::NotSignedIn
    );
    assert_eq!(
        kind(None, None, "Invalid API key · Please run /login", false),
        AssistantError::NotSignedIn
    );
    assert!(matches!(
        classify(Some("rate_limit"), Some(429), "You've hit your 5-hour limit", Some(1_800_000_000), false),
        AssistantError::UsageLimit { resets_at: Some(1_800_000_000), detail } if detail == "You've hit your 5-hour limit"
    ));
    assert!(matches!(
        kind(
            Some("billing_error"),
            None,
            "Credit balance is too low",
            false
        ),
        AssistantError::UsageLimit {
            resets_at: None,
            ..
        }
    ));
    assert!(matches!(
        kind(None, Some(429), "x", false),
        AssistantError::UsageLimit { .. }
    ));
    assert!(matches!(
        kind(Some("overloaded"), Some(529), "Overloaded", false),
        AssistantError::Unavailable { .. }
    ));
    assert!(matches!(
        kind(
            Some("server_error"),
            Some(500),
            "Internal server error",
            false
        ),
        AssistantError::Unavailable { .. }
    ));
    assert!(matches!(
        kind(Some("server_error"), None, "something odd", false),
        AssistantError::Unavailable { .. }
    ));
    assert!(matches!(
        kind(
            Some("model_not_found"),
            Some(404),
            "issue with the selected model",
            false
        ),
        AssistantError::Rejected { .. }
    ));
    assert!(matches!(
        kind(Some("invalid_request"), Some(400), "bad", false),
        AssistantError::Rejected { .. }
    ));
    // No answer from the service at all is the network, whatever the category says.
    assert!(matches!(
        kind(
            Some("server_error"),
            None,
            "API Error: Connection refused (ECONNREFUSED)",
            false
        ),
        AssistantError::Network { .. }
    ));
    assert!(matches!(
        kind(
            Some("unknown"),
            None,
            "getaddrinfo ENOTFOUND api.anthropic.com",
            false
        ),
        AssistantError::Network { .. }
    ));
    assert!(matches!(
        kind(Some("server_error"), None, "API Error", true),
        AssistantError::Network { .. }
    ));
    assert!(
        matches!(kind(Some("unknown"), None, "something nobody foresaw", false), AssistantError::Other { detail } if detail == "something nobody foresaw")
    );
    assert!(matches!(
        kind(None, None, "", false),
        AssistantError::Other { .. }
    ));
    // The detail is one line and bounded.
    let AssistantError::Other { detail } =
        kind(None, None, &format!("a\nb\t{}", "x".repeat(2000)), false)
    else {
        panic!()
    };
    assert!(detail.starts_with("a b x") && detail.chars().count() <= 400);
}
