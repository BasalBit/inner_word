//! The tests cannot reach the user's library or home: the guard, tested.

use std::process::Command;

use super::harness::{Sandbox, inside, real_places};

#[test]
fn a_process_started_by_the_harness_sees_only_the_sandbox() {
    let sandbox = Sandbox::new();
    let mut env = Command::new("/usr/bin/env");
    sandbox.isolate(&mut env);
    let output = env.output().expect("env runs");
    let printed = String::from_utf8(output.stdout).unwrap();
    let mut names: Vec<&str> = printed
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name)
        .collect();
    names.sort_unstable();
    // Nothing of the test's own environment: not its HOME, not a data directory somebody
    // exported, not a PATH with the user's tools.
    assert_eq!(
        names,
        vec!["HOME", "INNER_WORD_DATA_DIR", "PATH"],
        "{printed}"
    );
    let root = sandbox.root();
    for line in printed.lines() {
        if let Some(value) = line
            .strip_prefix("HOME=")
            .or_else(|| line.strip_prefix("INNER_WORD_DATA_DIR="))
        {
            assert!(
                inside(std::path::Path::new(value), &root),
                "{line} is outside the sandbox"
            );
        }
    }
}

#[test]
fn the_library_a_test_works_on_is_the_one_in_its_sandbox() {
    let sandbox = Sandbox::new();
    let status = sandbox.json(&["status"]);
    let data_dir = std::path::PathBuf::from(status["data_dir"].as_str().unwrap());
    assert_eq!(data_dir, sandbox.library());
    assert!(inside(&data_dir, &sandbox.root()));
    for real in real_places() {
        assert!(
            !data_dir.starts_with(&real),
            "{} is the user's own",
            data_dir.display()
        );
    }
}

#[test]
fn without_the_variable_the_default_library_is_still_inside_the_temporary_home() {
    // Should a test ever forget both the variable and the option, the tool falls back to
    // "the user's library": with the home of the sandbox that is a folder in the sandbox.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env_remove("INNER_WORD_DATA_DIR")
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let data_dir = std::path::PathBuf::from(status["data_dir"].as_str().unwrap());
    assert_eq!(
        data_dir,
        sandbox
            .home()
            .join("Library/Application Support/Inner Word")
    );
    assert!(inside(&sandbox.home(), &sandbox.root()));
    assert_eq!(status["exists"], false, "a library nobody made");

    // And a write there lands in the sandbox, nowhere else.
    let created = sandbox
        .command()
        .env_remove("INNER_WORD_DATA_DIR")
        .args(["create", "Guard", "--content", "text", "--json"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(data_dir.join("library.db").is_file());
}

#[test]
fn the_option_names_the_library_and_wins_over_the_variable() {
    let sandbox = Sandbox::new();
    let other = sandbox.root().join("other-library");
    let status = sandbox.json(&["--library", other.to_str().unwrap(), "status"]);
    assert_eq!(status["data_dir"], other.to_str().unwrap());
    let same = sandbox.json(&["status", &format!("--library={}", other.display())]);
    assert_eq!(same["data_dir"], other.to_str().unwrap());
}

#[test]
fn the_guard_knows_the_real_places_and_keeps_away_from_them() {
    // The test process itself has the user's HOME: that is how the guard knows what to
    // stay away from.
    let places = real_places();
    assert!(
        places
            .iter()
            .any(|place| place.ends_with("Library/Application Support/Inner Word")),
        "the guard does not know where the real library is: {places:?}"
    );
    assert!(places.iter().any(|place| place.ends_with(".claude")));
    let sandbox = Sandbox::new();
    sandbox.assert_isolated();
    for place in &places {
        assert!(
            !inside(place, &sandbox.root()),
            "{} is in the sandbox",
            place.display()
        );
        assert!(!inside(&sandbox.library(), place));
        assert!(!inside(&sandbox.home(), place));
    }
}
