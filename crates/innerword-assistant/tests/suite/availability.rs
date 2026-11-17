//! Whether Claude Code is there, which version, whether the user is signed in, and
//! which models it offers: asked of the stand-in program.

use innerword_assistant::claude::Search;
use innerword_assistant::{
    Availability, Backend, CatalogSource, ClaudeCode, Mode, Selection, SignIn,
};
use serde_json::json;

use super::support::{Stage, fixture};

#[test]
fn the_program_says_its_version_and_that_the_user_is_signed_in() {
    let stage = Stage::new(json!({"auth": "signed-in"}));
    let claude = stage.backend();
    assert_eq!(
        claude.cached_availability(),
        None,
        "nothing is asked before it is needed"
    );
    let availability = claude.availability();
    assert!(availability.is_installed());
    assert!(availability.is_ready());
    assert!(!availability.is_signed_out());
    assert_eq!(availability.version(), Some("2.1.322"));
    // The plan, and neither the address nor the organisation the status also carries.
    assert_eq!(availability.account(), Some("Claude Max plan"));
    assert_eq!(availability.installation().unwrap().program, stage.program);
    assert_eq!(claude.cached_availability(), Some(availability));
    // The two questions, and nothing else: no conversation was started to find out.
    assert_eq!(
        stage.launches(),
        vec![
            vec!["--version".to_string()],
            vec!["auth".to_string(), "status".to_string()]
        ]
    );
}

#[test]
fn a_signed_out_user_is_told_apart_from_a_missing_program() {
    let stage = Stage::new(json!({"auth": "signed-out"}));
    let availability = stage.backend().availability();
    assert!(availability.is_installed());
    assert!(availability.is_signed_out());
    assert!(!availability.is_ready());
    assert_eq!(availability.account(), None);

    let missing = ClaudeCode::with_program(stage.path().join("no such claude"));
    assert_eq!(missing.availability(), Availability::NotInstalled);
    assert!(!missing.availability().is_ready());
}

#[test]
fn a_program_that_cannot_say_is_taken_as_usable() {
    // An older version without `auth status`: the feature is there, a message will tell.
    let stage = Stage::new(json!({"auth": "garbage", "version": "Claude Code"}));
    let availability = stage.backend().availability();
    let installation = availability.installation().expect("installed");
    assert_eq!(installation.sign_in, SignIn::Unknown);
    assert_eq!(installation.version, None);
    assert!(availability.is_ready());
}

#[test]
fn the_answer_is_kept_until_it_is_asked_for_again() {
    let stage = Stage::new(json!({}));
    let claude = stage.backend();
    claude.availability();
    claude.availability();
    assert_eq!(
        stage.launches().len(),
        2,
        "one look: the version and the status"
    );
    // A clone shares what is known.
    assert!(claude.clone().cached_availability().is_some());
    claude.recheck();
    assert_eq!(stage.launches().len(), 4, "a second look only on request");
}

#[test]
fn the_program_is_found_by_a_search_of_folders() {
    let stage = Stage::new(json!({}));
    let search = Search {
        dirs: vec![
            stage.path().join("empty"),
            stage.program.parent().unwrap().to_path_buf(),
        ],
        shell: None,
    };
    let claude = ClaudeCode::with_search(search);
    assert_eq!(
        claude
            .availability()
            .installation()
            .map(|found| found.program.clone()),
        Some(stage.program.clone())
    );
    let nowhere = ClaudeCode::with_search(Search {
        dirs: vec![stage.path().join("empty")],
        shell: None,
    });
    assert_eq!(nowhere.availability(), Availability::NotInstalled);
}

#[test]
fn the_models_are_asked_of_the_program_and_no_message_is_sent() {
    let stage = Stage::new(json!({"initialize": fixture("initialize.jsonl")}));
    let claude = stage.backend();
    // Until it is asked, the built-in table.
    assert!(matches!(
        claude.catalog().source,
        CatalogSource::BuiltIn { .. }
    ));
    let catalog = claude.refresh_catalog();
    assert_eq!(
        catalog.source,
        CatalogSource::Agent {
            version: Some("2.1.322".to_string())
        }
    );
    let names: Vec<&str> = catalog
        .models
        .iter()
        .map(|model| model.name.as_str())
        .collect();
    assert_eq!(names, ["Haiku 5.5", "Sonnet 5.5", "Opus 5.5", "Fable 5.2"]);
    assert_eq!(
        catalog.agent_default_label,
        "Claude Code's default (Fable 5.2)"
    );
    assert_eq!(
        catalog.default_for(Mode::Quick),
        Selection::named("haiku", "low")
    );
    assert_eq!(
        catalog.default_for(Mode::Side),
        Selection::named("sonnet", "medium")
    );
    assert_eq!(claude.catalog(), catalog, "what was learned is kept");
    // The process that was asked got one request and no message, with the isolation
    // flags of every other process.
    assert_eq!(stage.requests(), ["initialize"]);
    assert!(stage.messages().is_empty());
    let probe = &stage.conversations()[0];
    for flag in [
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ] {
        assert!(probe.iter().any(|arg| arg == flag), "{flag}");
    }
}

#[test]
fn a_program_that_cannot_list_its_models_leaves_the_built_in_table() {
    let stage = Stage::new(json!({}));
    let claude = stage.backend();
    let catalog = claude.refresh_catalog();
    assert!(matches!(catalog.source, CatalogSource::BuiltIn { .. }));
    assert_eq!(
        catalog.default_for(Mode::Quick),
        Selection::named("haiku", "low")
    );

    // A signed-out program is not asked at all.
    let signed_out =
        Stage::new(json!({"auth": "signed-out", "initialize": fixture("initialize.jsonl")}));
    let catalog = signed_out.backend().refresh_catalog();
    assert!(matches!(catalog.source, CatalogSource::BuiltIn { .. }));
    assert!(signed_out.conversations().is_empty());
}

#[test]
fn a_catalog_kept_from_an_earlier_run_shows_until_the_program_is_asked() {
    let stage = Stage::new(json!({"initialize": fixture("initialize.jsonl")}));
    let earlier = {
        let claude = stage.backend();
        claude.refresh_catalog()
    };
    // The app stored it, and starts again.
    let stored = serde_json::to_string(&earlier).unwrap();
    let claude = stage.backend();
    claude.seed_catalog(serde_json::from_str(&stored).unwrap());
    assert_eq!(
        claude.catalog(),
        earlier,
        "the program's names, before it was asked"
    );
    // A second seed does not replace what is known, and a refresh does.
    claude.seed_catalog(innerword_assistant::fake_catalog());
    assert_eq!(claude.catalog(), earlier);
    assert_eq!(claude.refresh_catalog(), earlier);
}
