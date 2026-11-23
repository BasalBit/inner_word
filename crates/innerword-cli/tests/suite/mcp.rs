//! `innerword mcp`: a scripted conversation with the built binary over real pipes.

use serde_json::{Value, json};

use super::harness::Sandbox;

const MODERN: &str = "2026-07-28";

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "test-client", "title": "Test Client", "version": "1" },
    })
}

#[test]
fn the_handshake_the_tool_list_and_a_clean_exit() {
    let sandbox = Sandbox::new();
    let mut client = sandbox.mcp();
    let hello = client.initialize("2025-06-18", "test-client");
    assert_eq!(hello["protocolVersion"], "2025-06-18");
    assert_eq!(hello["serverInfo"]["name"], "inner-word");
    assert_eq!(hello["serverInfo"]["title"], "Inner Word");
    assert_eq!(hello["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        hello["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    let instructions = hello["instructions"].as_str().unwrap();
    // The model is told the rule: while the app is open, the app makes the change and
    // the user sees it.
    assert!(
        instructions.contains(
            "While Inner Word is open, a change is carried out by the app itself: it appears \
             in the user's window at once and they can undo it there, so say what you changed."
        ),
        "{instructions}"
    );
    assert!(instructions.contains("app_is_open"), "{instructions}");
    assert!(!instructions.contains("are refused"), "{instructions}");

    assert_eq!(client.request("ping", json!({}))["result"], json!({}));

    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 41);
    let mut names: Vec<&str> = Vec::new();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        names.push(name);
        assert!(tool["description"].as_str().unwrap().len() > 30, "{name}");
        assert!(tool["title"].is_string(), "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
        let annotations = &tool["annotations"];
        assert!(annotations["readOnlyHint"].is_boolean(), "{name}");
        // Only adding a page reaches the web (it downloads what the page loads).
        assert_eq!(annotations["openWorldHint"], name == "add_page", "{name}");
        let reads = annotations["readOnlyHint"] == true;
        let properties = tool["inputSchema"]["properties"].as_object().unwrap();
        assert_eq!(
            properties.contains_key("dry_run"),
            !reads,
            "{name}: what changes something can be tried first"
        );
        if !reads {
            assert!(annotations["destructiveHint"].is_boolean(), "{name}");
        }
        // Nothing that erases for good.
        assert!(!properties.contains_key("permanently"), "{name}");
    }
    for expected in [
        "library_status",
        "list_library",
        "search_library",
        "recent_documents",
        "document_info",
        "folder_info",
        "list_trash",
        "read_document",
        "find_in_document",
        "create_document",
        "replace_document",
        "edit_document",
        "append_to_document",
        "insert_in_document",
        "replace_section",
        "rename_document",
        "move_document",
        "duplicate_document",
        "trash_document",
        "restore_document",
        "create_folder",
        "rename_folder",
        "move_folder",
        "trash_folder",
        "restore_folder",
        "list_comments",
        "add_comment",
        "reply_to_comment",
        "resolve_comment",
        "reopen_comment",
        "delete_comment",
        "list_versions",
        "read_version",
        "restore_version",
        "export",
    ] {
        assert!(names.contains(&expected), "{expected} is not offered");
    }
    for forbidden in ["delete_document", "delete_folder", "empty_trash"] {
        assert!(
            !names.contains(&forbidden),
            "{forbidden} must not be offered"
        );
        let response = client.request(
            "tools/call",
            json!({ "name": forbidden, "arguments": { "permanently": true } }),
        );
        assert_eq!(response["error"]["code"], -32602, "{response}");
        assert_eq!(
            response["error"]["message"],
            format!("Unknown tool: {forbidden}")
        );
    }
    let read_only: Vec<&str> = tools
        .iter()
        .filter(|tool| tool["annotations"]["readOnlyHint"] == true)
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        read_only,
        vec![
            "library_status",
            "list_library",
            "search_library",
            "recent_documents",
            "document_info",
            "folder_info",
            "list_trash",
            "read_document",
            "find_in_document",
            "list_comments",
            "list_versions",
            "read_version",
        ]
    );
    // The same list, in the same order, every time.
    assert_eq!(
        client.request("tools/list", json!({}))["result"]["tools"],
        listed["result"]["tools"]
    );
    // The server made no library by being asked what it can do.
    assert!(!sandbox.library().join("library.db").exists());
    client.close();
}

#[test]
fn malformed_input_and_unknown_methods_are_answered_and_the_server_goes_on() {
    let sandbox = Sandbox::new();
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "test-client");

    client.send_line("{this is not json");
    let parse = client.read();
    assert_eq!(parse["error"]["code"], -32700);
    assert!(parse.get("id").is_none());

    client.send_line("[]");
    assert_eq!(client.read()["error"]["code"], -32600);
    client.send_line("42");
    assert_eq!(client.read()["error"]["code"], -32600);
    client.send_line(r#"{"jsonrpc":"2.0","id":"x"}"#);
    let no_method = client.read();
    assert_eq!(no_method["error"]["code"], -32600);
    assert_eq!(no_method["id"], "x");

    for method in [
        "resources/list",
        "prompts/list",
        "logging/setLevel",
        "tools/destroy",
    ] {
        let response = client.request(method, json!({}));
        assert_eq!(response["error"]["code"], -32601, "{method}: {response}");
    }
    let unknown_tool = client.request(
        "tools/call",
        json!({ "name": "format_disk", "arguments": {} }),
    );
    assert_eq!(unknown_tool["error"]["code"], -32602);
    let no_params = client.request("tools/call", json!({}));
    assert_eq!(no_params["error"]["code"], -32602);

    // Wrong arguments are for the model to correct: a tool result, not a protocol error.
    let (message, error) =
        client.call_error("read_document", json!({ "doc": "x", "format": "pdf" }));
    assert!(
        message.contains("There is no argument \"doc\"."),
        "{message}"
    );
    assert!(message.contains("\"document\" is required."), "{message}");
    assert!(message.contains("\"format\" must be one of"), "{message}");
    assert_eq!(error["code"], "invalid");

    // A notification nobody knows, a blank line, a response to nothing: no answer, and the
    // next request is answered as usual.
    client.notify("notifications/whatever", json!({}));
    client.notify(
        "notifications/cancelled",
        json!({ "requestId": 99, "reason": "changed my mind" }),
    );
    client.send_line("");
    client.send_line(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#);
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));

    // A batch (revision 2025-03-26) is answered with a batch.
    client.send_line(
        r#"[{"jsonrpc":"2.0","id":"a","method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":"b","method":"nope"}]"#,
    );
    let batch = client.read();
    assert_eq!(batch.as_array().unwrap().len(), 2);
    assert_eq!(batch[0]["id"], "a");
    assert_eq!(batch[1]["error"]["code"], -32601);
    client.close();
}

#[test]
fn the_revision_without_a_handshake() {
    let sandbox = Sandbox::new();
    let mut client = sandbox.mcp();
    // No initialize: every request carries its version.
    let discover = client.request("server/discover", json!({ "_meta": modern_meta() }));
    let result = &discover["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"][0], MODERN);
    assert!(
        result["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2025-11-25"))
    );
    assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
    assert!(result["instructions"].is_string());
    assert!(result["ttlMs"].as_u64().is_some());
    assert_eq!(result["cacheScope"], "public");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "inner-word"
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );

    let listed = client.request("tools/list", json!({ "_meta": modern_meta() }));
    assert_eq!(listed["result"]["resultType"], "complete");
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 41);
    assert!(listed["result"]["ttlMs"].as_u64().is_some());
    assert_eq!(listed["result"]["cacheScope"], "public");

    let status = client.request(
        "tools/call",
        json!({ "name": "library_status", "arguments": {}, "_meta": modern_meta() }),
    );
    assert_eq!(status["result"]["resultType"], "complete");
    assert_eq!(status["result"]["isError"], false);
    assert_eq!(status["result"]["structuredContent"]["exists"], false);
    assert_eq!(
        status["result"]["structuredContent"]["data_dir"],
        sandbox.library().to_str().unwrap()
    );
    assert_eq!(
        status["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "inner-word"
    );

    let mut old = modern_meta();
    old["io.modelcontextprotocol/protocolVersion"] = json!("1999-12-31");
    let refused = client.request("tools/list", json!({ "_meta": old }));
    assert_eq!(refused["error"]["code"], -32022, "{refused}");
    assert_eq!(refused["error"]["data"]["requested"], "1999-12-31");
    assert_eq!(refused["error"]["data"]["supported"][0], MODERN);

    let incomplete = client.request(
        "tools/list",
        json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": MODERN } }),
    );
    assert_eq!(incomplete["error"]["code"], -32602);
    let ping = client.request("ping", json!({ "_meta": modern_meta() }));
    assert_eq!(
        ping["error"]["code"], -32601,
        "ping left the protocol with the handshake"
    );
    client.close();
}

#[test]
fn every_older_revision_is_spoken() {
    let sandbox = Sandbox::new();
    for version in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let mut client = sandbox.mcp();
        let hello = client.initialize(version, "old-client");
        assert_eq!(hello["protocolVersion"], version);
        let (_, status) = client.call_ok("library_status", json!({}));
        assert_eq!(status["app_is_open"], false);
        client.close();
    }
    let mut client = sandbox.mcp();
    let hello = client.initialize("2031-01-01", "new-client");
    assert_eq!(
        hello["protocolVersion"], "2025-11-25",
        "the newest handshake revision is offered"
    );
    client.close();
}

#[test]
fn the_server_takes_the_library_as_an_option() {
    let sandbox = Sandbox::new();
    let other = sandbox.root().join("another-library");
    let mut child = sandbox
        .command()
        .args(["--library", other.to_str().unwrap(), "mcp"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"library_status","arguments":{{}}}}}}"#
        )
        .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["result"]["structuredContent"]["data_dir"],
        other.to_str().unwrap()
    );
    assert!(!other.exists(), "asking about a library does not make one");
}
