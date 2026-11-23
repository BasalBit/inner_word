//! The messages the page posts, as `bridge::parse` reads them.

use innerword_board_web::bridge::{Message, Status, parse};

#[test]
fn ready() {
    let m = parse(r#"{"v":1,"kind":"ready","excalidraw":"0.18.1","elements":14,"stamp":"3f2a9c1b5d6e7f80a1b2c3d4e5f60718"}"#).unwrap();
    match m {
        Message::Ready { excalidraw, elements, stamp } => {
            assert_eq!(excalidraw, "0.18.1");
            assert_eq!(elements, 14);
            assert_eq!(stamp, "3f2a9c1b5d6e7f80a1b2c3d4e5f60718");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_scene_stays_raw() {
    let scene = r##"{"type":"excalidraw","version":2,"source":"inner-word","elements":[{"id":"a","type":"rectangle","x":1}],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}}"##;
    let body = format!(r#"{{"v":1,"kind":"scene","rev":31,"scene":{scene}}}"#);
    match parse(&body).unwrap() {
        Message::Scene { rev, scene: raw } => {
            assert_eq!(rev, 31);
            assert_eq!(raw.get(), scene);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn scene_parts() {
    match parse(r#"{"v":1,"kind":"scenePart","rev":4,"index":1,"count":3,"text":"\"elements\":["}"#).unwrap() {
        Message::ScenePart { rev, index, count, text } => {
            assert_eq!((rev, index, count), (4, 1, 3));
            assert_eq!(text, "\"elements\":[");
        }
        other => panic!("{other:?}"),
    }
    assert!(parse(r#"{"v":1,"kind":"scenePart","rev":4,"index":3,"count":3,"text":""}"#).is_err());
    assert!(parse(r#"{"v":1,"kind":"scenePart","rev":4,"index":0,"count":0,"text":""}"#).is_err());
}

#[test]
fn a_file() {
    match parse(r#"{"v":1,"kind":"file","id":"6c2b","mimeType":"image/png","dataURL":"data:image/png;base64,iVBOR"}"#).unwrap() {
        Message::File { id, mime_type, data_url } => {
            assert_eq!(id, "6c2b");
            assert_eq!(mime_type, "image/png");
            assert_eq!(data_url, "data:image/png;base64,iVBOR");
        }
        other => panic!("{other:?}"),
    }
    assert!(parse(r#"{"v":1,"kind":"file","id":"","dataURL":"data:,"}"#).is_err());
    assert!(parse(r#"{"v":1,"kind":"file","id":"x"}"#).is_err());
}

#[test]
fn a_status_as_the_page_writes_it() {
    let m = parse(
        r#"{"v":1,"kind":"status","rev":31,"posted":30,"editing":false,"composing":true,"pointer":"down","zoom":1.25,"scrollX":-120.5,"scrollY":40,"elements":14,"selected":2,"tooLarge":false,"libraryWaiting":true}"#,
    )
    .unwrap();
    let Message::Status(status) = m else { panic!() };
    assert_eq!(
        status,
        Status {
            rev: 31,
            posted: 30,
            editing: false,
            composing: true,
            pointer_down: true,
            zoom: 1.25,
            scroll_x: -120.5,
            scroll_y: 40.0,
            elements: 14,
            selected: 2,
            too_large: false,
            library_waiting: true,
        }
    );
    assert!(status.holds_more());
}

#[test]
fn a_status_is_read_leniently() {
    // A field left out is its default; NaN (written null) is 0; an unknown pointer is up.
    let Message::Status(status) = parse(r#"{"v":1,"kind":"status","rev":3,"posted":3,"zoom":null,"pointer":"sideways","extra":1}"#).unwrap() else {
        panic!()
    };
    assert_eq!(status.zoom, 0.0);
    assert!(!status.pointer_down);
    assert!(!status.holds_more());
}

#[test]
fn what_holds_more() {
    let quiet = Status { rev: 5, posted: 5, ..Status::default() };
    assert!(!quiet.holds_more());
    assert!(Status { rev: 6, ..quiet.clone() }.holds_more());
    assert!(Status { editing: true, ..quiet.clone() }.holds_more());
    assert!(Status { pointer_down: true, ..quiet.clone() }.holds_more());
    // Composing alone holds nothing unposted (the agents' wait is the app's).
    assert!(!Status { composing: true, ..quiet.clone() }.holds_more());
    // A waiting change of the shape library is said apart: it is not the drawing's.
    assert!(!Status { library_waiting: true, ..quiet.clone() }.holds_more());
}

#[test]
fn a_waiting_library_change_is_in_the_status_the_glue_writes() {
    let status = Status { library_waiting: true, ..Status::default() };
    assert_eq!(serde_json::to_value(&status).unwrap()["libraryWaiting"], true);
    let Message::Status(old) = parse(r#"{"v":1,"kind":"status","rev":3,"posted":3}"#).unwrap() else {
        panic!()
    };
    assert!(!old.library_waiting, "a status without the field waits for nothing");
    let glue = std::fs::read_to_string(super::web().join("src/glue/autosave.js")).unwrap();
    assert!(glue.contains("libraryWaiting: libraryWaiting(),"));
    assert!(glue.contains("status.tooLarge, status.libraryWaiting].join"), "a change of it is posted at once");
}

#[test]
fn a_status_is_written_as_the_page_writes_it() {
    let status = Status { rev: 2, posted: 1, pointer_down: true, zoom: 1.0, too_large: true, ..Status::default() };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["pointer"], "down");
    assert_eq!(json["tooLarge"], true);
    assert_eq!(json["scrollX"], 0.0);
    let back: Status = serde_json::from_value(json).unwrap();
    assert_eq!(back, status);
}

#[test]
fn the_start_that_failed_is_said_in_the_words_of_the_rust_half() {
    let board = std::fs::read_to_string(super::web().join("src/glue/board.jsx")).unwrap();
    assert!(board.contains(&format!("reportError(`{}: ${{error?.message ?? error}}`)", innerword_board_web::bridge::START_FAILED)));
    // A board.json that is not a stored scene is a start that failed, not an empty canvas.
    assert!(board.contains(r#"if (!scene) throw new Error("board.json could not be read"#));
}

#[test]
fn library_help_and_error() {
    match parse(r#"{"v":1,"kind":"library","lrev":3,"items":[{"id":"i","status":"unpublished","elements":[],"created":1}]}"#).unwrap() {
        Message::Library { lrev, items, too_large } => {
            assert_eq!(lrev, 3);
            assert!(items.get().starts_with("[{"));
            assert!(!too_large);
        }
        other => panic!("{other:?}"),
    }
    assert!(parse(r#"{"v":1,"kind":"library","lrev":3,"items":{}}"#).is_err());
    // Longer than one message: the page says so, and its items are null, never a list that
    // could be stored as the library (as the glue writes it).
    let glue = std::fs::read_to_string(super::web().join("src/glue/library.js")).unwrap();
    assert!(glue.contains(r#"`{"v":1,"kind":"library","lrev":${library.lrev},"items":null,"tooLarge":true}`"#));
    match parse(r#"{"v":1,"kind":"library","lrev":4,"items":null,"tooLarge":true}"#).unwrap() {
        Message::Library { lrev, items, too_large } => {
            assert_eq!((lrev, items.get(), too_large), (4, "null", true));
            assert!(innerword_board::ShapeLibrary::from_view(items.get()).is_err(), "null is never a library");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse(r#"{"v":1,"kind":"library","lrev":4,"tooLarge":true}"#).unwrap(),
        Message::Library { too_large: true, .. }
    ));
    assert!(matches!(parse(r#"{"v":1,"kind":"help"}"#).unwrap(), Message::Help));
    match parse(r#"{"v":1,"kind":"error","message":"a promise was rejected: x"}"#).unwrap() {
        Message::Error { message } => assert_eq!(message, "a promise was rejected: x"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn what_is_refused() {
    for body in [
        "",
        "not json",
        "[1,2]",
        r#""a string""#,
        r#"{"kind":"help"}"#,
        r#"{"v":2,"kind":"help"}"#,
        r#"{"v":1}"#,
        r#"{"v":1,"kind":"open","url":"https://example.com"}"#,
        r#"{"v":1,"kind":"scene","scene":{}}"#,
        r#"{"v":1,"kind":"scene","rev":1}"#,
        r#"{"v":1,"kind":"scene","rev":1,"scene":[1]}"#,
        r#"{"v":1,"kind":"scene","rev":-1,"scene":{}}"#,
        r#"{"v":1,"kind":"library","items":[]}"#,
    ] {
        let error = parse(body).unwrap_err();
        assert!(error.contains("whiteboard"), "{body}: {error}");
    }
}

#[test]
fn a_lone_surrogate_is_mended_before_reading() {
    let body = r#"{"v":1,"kind":"error","message":"half \ud800 of a pair, and \\ud800 as text, and a whole 😀"}"#;
    match parse(body).unwrap() {
        Message::Error { message } => assert_eq!(message, "half \u{fffd} of a pair, and \\ud800 as text, and a whole \u{1f600}"),
        other => panic!("{other:?}"),
    }
    let scene = r#"{"v":1,"kind":"scene","rev":2,"scene":{"elements":[{"text":"\udc00"}]}}"#;
    match parse(scene).unwrap() {
        Message::Scene { scene, .. } => assert_eq!(scene.get(), concat!(r#"{"elements":[{"text":""#, "\\", r#"ufffd"}]}"#)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_message_written_reads_back() {
    let scene = serde_json::value::RawValue::from_string(r#"{"elements":[]}"#.to_string()).unwrap();
    let items = serde_json::value::RawValue::from_string("[]".to_string()).unwrap();
    let messages = [
        Message::Ready { excalidraw: "0.18.1".into(), elements: 2, stamp: "ab\"c".into() },
        Message::Scene { rev: 7, scene },
        Message::ScenePart { rev: 7, index: 0, count: 2, text: "{\"elem".into() },
        Message::File { id: "f".into(), mime_type: "image/png".into(), data_url: "data:image/png;base64,AA==".into() },
        Message::Status(Status { rev: 1, posted: 1, zoom: 2.0, ..Status::default() }),
        Message::Library { lrev: 1, items, too_large: false },
        Message::Library {
            lrev: 2,
            items: serde_json::value::RawValue::from_string("null".to_string()).unwrap(),
            too_large: true,
        },
        Message::Help,
        Message::Error { message: "line\nbreak".into() },
    ];
    for message in messages {
        let body = message.to_body();
        let back = parse(&body).unwrap_or_else(|e| panic!("{body}: {e}"));
        assert_eq!(back.to_body(), body);
    }
}
