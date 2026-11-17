//! The model and the effort of the two chats, as the settings keep them.
//!
//! The engine names its models and effort levels itself
//! (`innerword_assistant::Catalog`); the app stores a choice as that name and
//! knows nothing about any engine's models. Each chat (the quick chat, the
//! side chat) remembers a model and an effort of its own:
//!
//! | Stored | Means |
//! |---|---|
//! | nothing, or an empty string | not chosen yet: what the engine suggests for that chat (a fast model at low effort for the quick chat) |
//! | [`AGENT_DEFAULT`] | whatever the engine uses when it is not told (the user's own default in it) |
//! | anything else | that model or level, by the engine's name for it |
//!
//! The pickers in the chats and the rows in the settings write the same
//! keys, so what was chosen in one is what the other shows.

use innerword_assistant::{Catalog, EffortChoice, Mode, ModelChoice, Selection};

use crate::settings::{AssistantSettings, keys};

/// The stored name of "whatever the engine uses when it is not told". No
/// engine names a model or a level like this.
pub const AGENT_DEFAULT: &str = "@default";

/// The setting keys of the model and the effort of `mode`.
pub fn keys_for(mode: Mode) -> (&'static str, &'static str) {
    match mode {
        Mode::Quick => (keys::ASSISTANT_QUICK_MODEL, keys::ASSISTANT_QUICK_EFFORT),
        Mode::Side => (keys::ASSISTANT_SIDE_MODEL, keys::ASSISTANT_SIDE_EFFORT),
    }
}

/// What `settings` remember for `mode`, as stored.
fn stored(mode: Mode, settings: &AssistantSettings) -> (Option<&str>, Option<&str>) {
    match mode {
        Mode::Quick => (
            settings.quick_model.as_deref(),
            settings.quick_effort.as_deref(),
        ),
        Mode::Side => (
            settings.side_model.as_deref(),
            settings.side_effort.as_deref(),
        ),
    }
}

/// The model and effort the chat of `mode` asks with: what the settings
/// remember, and for what was never chosen the engine's suggestion for
/// that chat. An effort the chosen model does not take gives way to the
/// engine's default.
pub fn selection_for(mode: Mode, settings: &AssistantSettings, catalog: &Catalog) -> Selection {
    let suggested = catalog.default_for(mode);
    let (model, effort) = stored(mode, settings);
    let selection = Selection {
        model: match model {
            None => suggested.model,
            Some(AGENT_DEFAULT) => ModelChoice::AgentDefault,
            Some(id) => ModelChoice::named(id),
        },
        effort: match effort {
            None => suggested.effort,
            Some(AGENT_DEFAULT) => EffortChoice::AgentDefault,
            Some(id) => EffortChoice::named(id),
        },
    };
    catalog.normalized(&selection)
}

/// A model choice as it is stored.
pub fn stored_model(choice: &ModelChoice) -> String {
    choice.id().unwrap_or(AGENT_DEFAULT).to_string()
}

/// An effort choice as it is stored.
pub fn stored_effort(choice: &EffortChoice) -> String {
    choice.id().unwrap_or(AGENT_DEFAULT).to_string()
}

/// The models a row of the settings steps through: the engine's own
/// default first, then its models. A remembered model the engine no longer
/// lists (`current`) is kept in the list, after the default, so that the
/// row can show it and step away from it.
pub fn model_options(catalog: &Catalog, current: &ModelChoice) -> Vec<ModelChoice> {
    let mut options = vec![ModelChoice::AgentDefault];
    if let ModelChoice::Named(id) = current
        && catalog.model(id).is_none()
    {
        options.push(current.clone());
    }
    options.extend(
        catalog
            .models
            .iter()
            .map(|model| ModelChoice::named(model.id.clone())),
    );
    options
}

/// The effort levels a row of the settings steps through for `model`: the
/// engine's own default first, then the levels that model takes.
pub fn effort_options(catalog: &Catalog, model: &ModelChoice) -> Vec<EffortChoice> {
    std::iter::once(EffortChoice::AgentDefault)
        .chain(
            catalog
                .efforts_for(model)
                .into_iter()
                .map(|effort| EffortChoice::named(effort.id)),
        )
        .collect()
}
