//! What the installed program says of itself: its version (`claude --version`) and
//! whether the user is signed in (`claude auth status`, which prints JSON).

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::args::base_command;
use super::run::run_with_timeout;
use crate::backend::SignIn;

/// How long each of the two questions may take.
const PATIENCE: Duration = Duration::from_secs(15);

/// The oldest version the assistant runs on: the first that delivers a message exactly
/// as written (`client_composed`), without expanding `@file` mentions or running text
/// that begins with `/` as a command. Source: the Agent SDK reference at
/// code.claude.com/docs/en/agent-sdk/typescript (`SDKUserMessage`), read 2026-11-12.
pub const MINIMUM_VERSION: &str = "2.1.248";

/// The version the program prints: "2.1.322" from "2.1.322 (Claude Code)".
pub fn version(program: &Path) -> Option<String> {
    let mut command = base_command(program, None);
    command.arg("--version");
    let ran = run_with_timeout(command, None, PATIENCE)?;
    parse_version(&ran.stdout)
}

/// The first thing in `output` that reads as a version number.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|word| word.trim_start_matches('v'))
        .find(|word| {
            let mut parts = word.split('.');
            let numeric = |part: Option<&str>| {
                part.is_some_and(|part| {
                    !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())
                })
            };
            numeric(parts.next()) && numeric(parts.next())
        })
        .map(|word| {
            word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_string()
        })
}

/// A version as numbers, for comparing: "2.1.322" is `[2, 1, 322]`. What follows the
/// numbers ("-beta.1") is ignored.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map_while(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u64>().ok()
        })
        .collect()
}

/// Whether `version` is at least `minimum`.
pub fn is_at_least(version: &str, minimum: &str) -> bool {
    let (have, need) = (numbers(version), numbers(minimum));
    for index in 0..need.len().max(have.len()) {
        let (have, need) = (
            have.get(index).copied().unwrap_or(0),
            need.get(index).copied().unwrap_or(0),
        );
        if have != need {
            return have > need;
        }
    }
    true
}

/// Whether the user is signed in, as `claude auth status` says.
pub fn sign_in(program: &Path) -> SignIn {
    let mut command = base_command(program, None);
    command.args(["auth", "status"]);
    match run_with_timeout(command, None, PATIENCE) {
        Some(ran) => parse_auth_status(&ran.stdout),
        None => SignIn::Unknown,
    }
}

/// Reads the JSON of `claude auth status`. Anything that is not that JSON (an older
/// version without the command, an error) is [`SignIn::Unknown`]: the feature stays
/// usable and a message will tell.
pub fn parse_auth_status(output: &str) -> SignIn {
    let start = output.find('{');
    let end = output.rfind('}');
    let (Some(start), Some(end)) = (start, end) else {
        return SignIn::Unknown;
    };
    if end < start {
        return SignIn::Unknown;
    }
    let Ok(status) = serde_json::from_str::<Value>(&output[start..=end]) else {
        return SignIn::Unknown;
    };
    match status.get("loggedIn").and_then(Value::as_bool) {
        Some(true) => SignIn::SignedIn {
            account: account_line(&status),
        },
        Some(false) => SignIn::SignedOut,
        None => SignIn::Unknown,
    }
}

/// A short line about the account for a person: the plan, or how the program is
/// signed in. Never the e-mail address and never the organisation's name (which is
/// often the address again).
pub fn account_line(status: &Value) -> Option<String> {
    let text = |key: &str| {
        status
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    match text("apiProvider") {
        Some("bedrock") => return Some("Amazon Bedrock".to_string()),
        Some("vertex") => return Some("Google Cloud".to_string()),
        Some("foundry") => return Some("Microsoft Foundry".to_string()),
        _ => {}
    }
    if let Some(plan) = text("subscriptionType").and_then(plan_name) {
        return Some(plan);
    }
    match text("authMethod") {
        Some("claude.ai") => Some("Claude subscription".to_string()),
        Some(method) if method.contains("api") || method.contains("key") => {
            Some("API key".to_string())
        }
        Some(method) if method.contains("console") => Some("Anthropic Console account".to_string()),
        Some(method) if method.contains("token") => Some("Access token".to_string()),
        _ => None,
    }
}

/// "Claude Max plan" from "max", "Claude Ultra plan" from "Claude Ultra". A value with
/// an `@` in it is never shown.
pub fn plan_name(subscription: &str) -> Option<String> {
    let plan = subscription.trim();
    if plan.is_empty() || plan.contains('@') || plan.chars().count() > 40 {
        return None;
    }
    let lower = plan.to_lowercase();
    let bare = lower
        .strip_prefix("claude ")
        .unwrap_or(&lower)
        .trim_end_matches(" plan")
        .replace('_', " ");
    if bare.is_empty() || bare == "none" {
        return None;
    }
    let mut name = String::from("Claude");
    for word in bare.split_whitespace() {
        name.push(' ');
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.extend(first.to_uppercase());
            name.push_str(chars.as_str());
        }
    }
    name.push_str(" plan");
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_what_the_program_prints() {
        assert_eq!(
            parse_version("2.1.322 (Claude Code)\n").as_deref(),
            Some("2.1.322")
        );
        assert_eq!(
            parse_version("claude v2.2.0-beta.1").as_deref(),
            Some("2.2.0-beta.1")
        );
        assert_eq!(parse_version("Claude Code\n3.0\n").as_deref(), Some("3.0"));
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(is_at_least("2.1.322", MINIMUM_VERSION));
        assert!(is_at_least("2.1.248", MINIMUM_VERSION));
        assert!(!is_at_least("2.1.247", MINIMUM_VERSION));
        assert!(!is_at_least("2.0.999", MINIMUM_VERSION));
        assert!(is_at_least("2.2", MINIMUM_VERSION));
        assert!(is_at_least("3.0.0", MINIMUM_VERSION));
        assert!(is_at_least("2.1.248-beta.3", MINIMUM_VERSION));
        assert!(!is_at_least("1.9.9999", MINIMUM_VERSION));
    }

    /// The output of `claude auth status` of Claude Code 2.1.322, signed in with a
    /// claude.ai subscription, recorded 2026-11-12 with what identifies the account
    /// replaced (the plan too: it reads "pro").
    const SIGNED_IN: &str = include_str!("../../tests/fixtures/auth-status-signed-in.json");
    /// The same with an empty configuration directory (exit code 1).
    const SIGNED_OUT: &str = include_str!("../../tests/fixtures/auth-status-signed-out.json");

    #[test]
    fn the_sign_in_state_is_read_from_the_recorded_status() {
        assert_eq!(
            parse_auth_status(SIGNED_IN),
            SignIn::SignedIn {
                account: Some("Claude Pro plan".to_string())
            }
        );
        assert_eq!(parse_auth_status(SIGNED_OUT), SignIn::SignedOut);
    }

    #[test]
    fn the_account_line_never_carries_an_address() {
        let SignIn::SignedIn { account } = parse_auth_status(SIGNED_IN) else {
            panic!("signed in");
        };
        let line = account.unwrap();
        assert!(!line.contains('@'), "{line}");
        assert!(!line.contains("example"), "{line}");
        // A plan field that holds an address is not shown; the method is.
        let odd = serde_json::json!({
            "loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "someone@example.com"
        });
        assert_eq!(account_line(&odd).as_deref(), Some("Claude subscription"));
    }

    #[test]
    fn other_ways_of_signing_in_have_their_own_line() {
        let line = |json: serde_json::Value| account_line(&json);
        assert_eq!(
            line(serde_json::json!({"authMethod": "claude.ai", "subscriptionType": "pro"}))
                .as_deref(),
            Some("Claude Pro plan")
        );
        assert_eq!(
            line(serde_json::json!({"subscriptionType": "Claude Ultra"})).as_deref(),
            Some("Claude Ultra plan")
        );
        assert_eq!(
            line(serde_json::json!({"subscriptionType": "max_20x"})).as_deref(),
            Some("Claude Max 20x plan")
        );
        assert_eq!(
            line(serde_json::json!({"authMethod": "api_key"})).as_deref(),
            Some("API key")
        );
        assert_eq!(
            line(serde_json::json!({"authMethod": "claude.ai", "apiProvider": "bedrock"}))
                .as_deref(),
            Some("Amazon Bedrock")
        );
        assert_eq!(
            line(serde_json::json!({"authMethod": "something new"})),
            None
        );
    }

    #[test]
    fn what_is_not_the_status_is_unknown() {
        assert_eq!(parse_auth_status(""), SignIn::Unknown);
        assert_eq!(
            parse_auth_status("error: unknown command 'auth'"),
            SignIn::Unknown
        );
        assert_eq!(parse_auth_status("{ not json }"), SignIn::Unknown);
        assert_eq!(
            parse_auth_status("{\"authMethod\": \"claude.ai\"}"),
            SignIn::Unknown
        );
        // Text around the JSON (a warning before it) does not matter.
        assert_eq!(
            parse_auth_status("warning: something\n{\"loggedIn\": false}\n"),
            SignIn::SignedOut
        );
    }
}
