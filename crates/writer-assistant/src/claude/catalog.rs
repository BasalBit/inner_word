//! The models and effort levels Claude Code offers.
//!
//! The list is not kept here: the program is asked. A process that is started with the
//! isolation flags and sent one `initialize` request (and no message: it costs the user
//! nothing and takes about a second) answers with the models of the user's account,
//! each with its display name, a description and the effort levels it takes
//! ([`ClaudeCode::refresh_catalog`](super::ClaudeCode)). The built-in table
//! ([`builtin_catalog`]) is only what the pickers show until that answer is there, and
//! when the program cannot be asked: it lists the aliases the program's `--model` help
//! names and the levels its `--effort` help names, as read on [`CATALOG_AS_OF`]. An
//! alias always means the newest model of its family, so the table does not go stale
//! with every model; it is wrong only when a family is added or retired.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::args::{base_command, probe_args};
use super::run::run_with_timeout;
use super::stream::{Line, control_request, parse_line};
use crate::backend::{Catalog, CatalogSource, EffortOption, ModelOption, Selection};

/// The day the built-in table was read from `claude --help` (2.1.322) and from
/// code.claude.com/docs/en/model-config.
pub const CATALOG_AS_OF: &str = "2026-11-12";

/// The effort levels in their order, lowest first, with their names.
const EFFORTS: [(&str, &str); 5] = [
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra high"),
    ("max", "Maximum"),
];

/// The label of the choice that names no model and no effort.
const DEFAULT_LABEL: &str = "Claude Code's default";

fn effort(id: &str) -> EffortOption {
    let name = EFFORTS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| {
            let mut chars = id.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        });
    EffortOption {
        id: id.to_string(),
        name,
    }
}

/// The defaults of the two modes for a list of models: the quick chat answers with the
/// fastest model at the lowest effort, the side chat with the balanced one at medium
/// effort. A family the list does not have falls back to the program's default.
fn defaults(models: &[ModelOption], efforts: &[EffortOption]) -> (Selection, Selection) {
    let pick = |wanted: &[&str], level: &str| {
        let model = wanted
            .iter()
            .find_map(|id| models.iter().find(|model| model.id == *id));
        match model {
            None => Selection::default(),
            Some(model) => {
                let takes = match &model.efforts {
                    Some(own) => own.iter().any(|own| own == level),
                    None => efforts.iter().any(|effort| effort.id == level),
                };
                if takes {
                    Selection::named(model.id.clone(), level)
                } else {
                    Selection {
                        model: crate::backend::ModelChoice::named(model.id.clone()),
                        effort: crate::backend::EffortChoice::AgentDefault,
                    }
                }
            }
        }
    };
    (pick(&["haiku"], "low"), pick(&["sonnet", "opus"], "medium"))
}

/// The table built into this version of the app (see the module documentation for
/// where it comes from and why it does not go stale quickly).
pub fn builtin_catalog() -> Catalog {
    let model = |id: &str, name: &str, description: &str| ModelOption {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        efforts: None,
    };
    let models = vec![
        model("haiku", "Haiku", "Fastest, for quick answers"),
        model("sonnet", "Sonnet", "Efficient, for everyday work"),
        model("opus", "Opus", "For complex work"),
        model("fable", "Fable", "For the toughest questions"),
    ];
    let efforts: Vec<EffortOption> = EFFORTS.iter().map(|(id, _)| effort(id)).collect();
    let (quick_default, side_default) = defaults(&models, &efforts);
    Catalog {
        models,
        efforts,
        quick_default,
        side_default,
        agent_default_label: DEFAULT_LABEL.to_string(),
        agent_default_effort_label: DEFAULT_LABEL.to_string(),
        source: CatalogSource::BuiltIn {
            as_of: CATALOG_AS_OF.to_string(),
        },
    }
}

/// The catalog from the program's answer to `initialize` (its `models` list).
///
/// The list has an entry `default` (what the program uses when it is not told; its
/// description names the model, which goes into the label of that choice), the aliases
/// (`haiku`, `sonnet`, `opus`, `fable`), and older models by their full names. The
/// pickers offer the aliases: an alias follows its family to the newest model, and a
/// short list is what a small picker wants. `None` when the answer has no usable list.
pub fn catalog_from_initialize(response: &Value, version: Option<String>) -> Option<Catalog> {
    let listed = response.get("models")?.as_array()?;
    let mut models = Vec::new();
    let mut levels: Vec<String> = Vec::new();
    let mut default_label = DEFAULT_LABEL.to_string();
    for entry in listed {
        let Some(id) = entry.get("value").and_then(Value::as_str) else {
            continue;
        };
        let name = entry
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .trim();
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if id == "default" {
            if !description.is_empty() && description.chars().count() <= 40 {
                default_label = format!("{DEFAULT_LABEL} ({description})");
            }
            continue;
        }
        let own_levels: Option<Vec<String>> =
            match entry.get("supportedEffortLevels").and_then(Value::as_array) {
                Some(list) => Some(
                    list.iter()
                        .filter_map(|level| level.as_str().map(str::to_string))
                        .collect(),
                ),
                None if entry.get("supportsEffort").and_then(Value::as_bool) == Some(false) => {
                    Some(Vec::new())
                }
                None => None,
            };
        for level in own_levels.iter().flatten() {
            if !levels.contains(level) {
                levels.push(level.clone());
            }
        }
        // A full model name is an older model kept for those who pin one.
        if id.starts_with("claude-") || id.contains('/') || id.contains(':') {
            continue;
        }
        models.push(ModelOption {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            efforts: own_levels,
        });
    }
    if models.is_empty() {
        return None;
    }
    // Fastest first, as the pickers read: the families in a fixed order, then whatever
    // the program lists that this table does not know yet.
    let rank = |id: &str| {
        ["haiku", "sonnet", "opus", "fable"]
            .iter()
            .position(|known| *known == id)
            .unwrap_or(usize::MAX)
    };
    models.sort_by_key(|model| rank(&model.id));
    let mut efforts: Vec<EffortOption> = EFFORTS
        .iter()
        .filter(|(id, _)| levels.iter().any(|level| level == id))
        .map(|(id, _)| effort(id))
        .collect();
    for level in &levels {
        if !efforts.iter().any(|known| &known.id == level) {
            efforts.push(effort(level));
        }
    }
    if efforts.is_empty() {
        efforts = EFFORTS.iter().map(|(id, _)| effort(id)).collect();
    }
    let (quick_default, side_default) = defaults(&models, &efforts);
    Some(Catalog {
        models,
        efforts,
        quick_default,
        side_default,
        agent_default_label: default_label,
        // The effort's default is the program's too, but it is no model.
        agent_default_effort_label: DEFAULT_LABEL.to_string(),
        source: CatalogSource::Agent { version },
    })
}

/// Asks the program what it offers: starts it with the isolation flags, sends one
/// `initialize` request and no message, and reads the answer. About a second; `None`
/// when the program does not answer in time, does not know the request or lists no
/// models.
pub fn probe(program: &Path, dir: Option<&Path>) -> Option<Catalog> {
    let mut command = base_command(program, dir);
    command.args(probe_args());
    let request = control_request("probe-1", "initialize", Value::Null) + "\n";
    let ran = run_with_timeout(command, Some(&request), Duration::from_secs(20))?;
    read_probe(&ran.stdout)
}

/// The catalog in the answer to the probe's request, among the lines the program
/// printed. Nothing of the account the answer also names is read.
pub fn read_probe(output: &str) -> Option<Catalog> {
    output.lines().find_map(|line| match parse_line(line) {
        Line::ControlResponse {
            ok: true, response, ..
        } => {
            let version = response
                .get("claude_code_version")
                .and_then(Value::as_str)
                .map(str::to_string);
            catalog_from_initialize(&response, version)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{EffortChoice, Mode, ModelChoice};

    /// The answer of Claude Code 2.1.322 to `initialize`, recorded 2026-11-12 with what
    /// identifies the account and the machine replaced.
    const INITIALIZE: &str = include_str!("../../tests/fixtures/initialize.jsonl");

    #[test]
    fn the_built_in_table_has_the_aliases_and_levels_of_the_programs_help() {
        let catalog = builtin_catalog();
        let ids: Vec<&str> = catalog
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["haiku", "sonnet", "opus", "fable"]);
        let levels: Vec<&str> = catalog
            .efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect();
        assert_eq!(levels, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(
            catalog.default_for(Mode::Quick),
            Selection::named("haiku", "low")
        );
        assert_eq!(
            catalog.default_for(Mode::Side),
            Selection::named("sonnet", "medium")
        );
        assert_eq!(catalog.agent_default_label, "Claude Code's default");
        assert!(matches!(catalog.source, CatalogSource::BuiltIn { .. }));
    }

    #[test]
    fn the_programs_own_list_becomes_the_catalog() {
        let catalog = read_probe(INITIALIZE).expect("the recorded answer lists models");
        let ids: Vec<&str> = catalog
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["haiku", "sonnet", "opus", "fable"]);
        // Names and descriptions are the program's own.
        let haiku = catalog.model("haiku").unwrap();
        assert_eq!(haiku.name, "Haiku 5.5");
        assert_eq!(haiku.description, "Fastest for quick answers");
        assert_eq!(
            haiku.efforts.as_deref(),
            Some(&["low", "medium", "high", "xhigh", "max"].map(String::from)[..])
        );
        assert_eq!(catalog.model_label(&ModelChoice::named("opus")), "Opus 5.5");
        // The default's description names the model behind it.
        assert_eq!(
            catalog.agent_default_label,
            "Claude Code's default (Fable 5.2)"
        );
        // The effort's default names no model.
        assert_eq!(
            catalog.effort_label(&crate::backend::EffortChoice::AgentDefault),
            "Claude Code's default"
        );
        assert_eq!(
            catalog.model_label(&ModelChoice::AgentDefault),
            "Claude Code's default (Fable 5.2)"
        );
        assert_eq!(
            catalog.model_label(&ModelChoice::AgentDefault),
            "Claude Code's default (Fable 5.2)"
        );
        let levels: Vec<&str> = catalog
            .efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect();
        assert_eq!(levels, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(
            catalog.effort_label(&EffortChoice::named("xhigh")),
            "Extra high"
        );
        assert_eq!(
            catalog.default_for(Mode::Quick),
            Selection::named("haiku", "low")
        );
        assert_eq!(
            catalog.default_for(Mode::Side),
            Selection::named("sonnet", "medium")
        );
        assert_eq!(
            catalog.source,
            CatalogSource::Agent {
                version: Some("2.1.322".to_string())
            }
        );
    }

    #[test]
    fn a_list_without_the_known_families_still_makes_a_catalog() {
        let response = serde_json::json!({"models": [
            {"value": "default", "displayName": "Default", "description": "Nova 1"},
            {"value": "nova", "displayName": "Nova 1", "description": "New", "supportsEffort": false},
            {"value": "claude-old-1", "displayName": "Old", "description": "Older"},
        ]});
        let catalog = catalog_from_initialize(&response, None).unwrap();
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].efforts, Some(Vec::new()));
        // No haiku and no sonnet: both modes use the program's default.
        assert_eq!(catalog.quick_default, Selection::default());
        assert_eq!(catalog.side_default, Selection::default());
        assert_eq!(
            catalog.agent_default_label,
            "Claude Code's default (Nova 1)"
        );
        // A model without effort levels is offered none.
        assert!(catalog.efforts_for(&ModelChoice::named("nova")).is_empty());
        assert_eq!(
            catalog.normalized(&Selection::named("nova", "high")),
            Selection {
                model: ModelChoice::named("nova"),
                effort: EffortChoice::AgentDefault
            }
        );
    }

    #[test]
    fn an_answer_without_models_is_no_catalog() {
        assert!(catalog_from_initialize(&serde_json::json!({}), None).is_none());
        assert!(catalog_from_initialize(&serde_json::json!({"models": []}), None).is_none());
        assert!(read_probe("").is_none());
        assert!(read_probe("not json\n{\"type\":\"system\"}\n").is_none());
        let refused = r#"{"type":"control_response","response":{"subtype":"error","request_id":"probe-1","error":"Unsupported control request subtype: initialize"}}"#;
        assert!(read_probe(refused).is_none());
    }
}
