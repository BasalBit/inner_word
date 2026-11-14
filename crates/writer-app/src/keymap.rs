//! The keymap as the app's own thing: the defaults every crate bound, the
//! user's changes to them, and the keymap that is installed.
//!
//! Every crate binds its keys in its own `init`, in the order
//! [`crate::init`] calls them. When they are all bound, [`take_defaults`]
//! reads the assembled keymap, in order, and keeps it as the defaults.
//! From then on the keymap GPUI has is always one this module built:
//!
//! 1. the bindings that only give menu bar items their key
//!    ([`crate::menus::menu_key_bindings`], computed from what follows);
//! 2. the defaults in their order, each one as it was bound, replaced by
//!    the user's key where the user changed it, or left out where the user
//!    removed it. A changed key stands where its default stood, so every
//!    precedence fact of the keymap holds as before: a binding without a
//!    context still ranks like the deepest context, and among the bindings
//!    at the focused element's own level the later one still wins.
//!
//! A binding is rebuilt from its parts with [`gpui::KeyBinding::load`]: the
//! action (a copy), the key context (the same predicate), no key
//! equivalents, the same mapper `KeyBinding::new` uses. A test holds that a
//! binding rebuilt this way is the binding it was.
//!
//! # The user's changes
//!
//! What a change is, what would collide and what is refused is
//! `writer-keymap`'s (no GPUI): this module hands it the defaults
//! ([`writer_keymap::Defaults`], one entry per binding, with the commands
//! of the palette that have no key), describes the window to it
//! ([`places`]), and binds what it says to bind
//! ([`writer_keymap::Effective::bindings`]).
//!
//! [`set_changes`] is the one way a change gets in: it keeps the changes,
//! installs the keymap they give, and builds the menu bar again. (Binding
//! keys refreshes every window.) The workspace calls it with what the
//! library has stored when it opens, and with every change the user makes,
//! which it then stores (`workspace/shortcuts.rs`); tests call it directly.
//!
//! # What is not handed over
//!
//! The two keys of the message window that stands in for the workspace
//! when the library cannot be opened ([`crate::window`]): the changes are
//! stored in the library, so that window always runs on the defaults, and
//! its keys are not listed anywhere. A binding with a key `writer-keymap`
//! does not read would not be handed over either (there is none). Both
//! are bound as they are, where they stood.

use std::rc::Rc;

use gpui::{
    Action, App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, KeyContext,
    Keymap,
};
use writer_keymap::{
    Changes, DefaultBinding, Defaults, Effective, EffectiveBinding, Places, StoredError, Target,
};
use writer_ui::KeyHint;

use crate::menus;

/// See the [module docs](self).
pub struct AppKeymap {
    /// What every crate bound in `init`, in order.
    defaults: Vec<KeyBinding>,
    /// For each binding handed over to the model (its index there), the
    /// index of the binding in `defaults`.
    handed: Vec<usize>,
    /// The defaults, the places of the window and the user's changes.
    model: writer_keymap::Keymap,
    /// What is installed: the bindings and the state of every command.
    effective: Effective,
}

impl Global for AppKeymap {}

/// Where a key that is given to a command without one applies, by the
/// namespace of the command's action: the commands of the workspace and
/// the app work wherever the keyboard is, those of the text where the
/// keys of the text work. A namespace that is not named works anywhere.
const HOME_CONTEXTS: [(&str, Option<&str>); 6] = [
    ("workspace", None),
    ("app", None),
    ("editor", Some(writer_editor::actions::KEY_CONTEXT)),
    ("spell", Some(writer_editor::actions::KEY_CONTEXT)),
    ("vim_mode", Some(writer_editor::actions::KEY_CONTEXT)),
    (
        "editor_tools",
        Some(writer_editor_tools::actions::KEY_CONTEXT),
    ),
];

/// The key context a key for the command `action` applies in when the
/// command has no key by default.
pub fn home_context(action: &str) -> Option<&'static str> {
    let namespace = action.split("::").next().unwrap_or(action);
    HOME_CONTEXTS
        .iter()
        .find(|(name, _)| *name == namespace)
        .and_then(|(_, context)| *context)
}

/// What `writer-keymap` is told about the places of the window: where
/// text is typed, which key context can lie inside which, which never has
/// the keyboard itself (the keyboard is always in something inside it),
/// and which is only around another (the tools around the text). It
/// judges from this whether a key would stop text from being typed and
/// whether two commands on one key get in each other's way. A new key
/// context of a crate goes in here.
///
/// What the keymap is told besides, with every change of a key: the rows
/// of the list and the names of their places ([`refresh_listed`]). Two
/// rows under one place name are in the same place, whatever their key
/// contexts are.
///
/// # What is not exact
///
/// The library has the keyboard itself (on a row) or in its filter field.
/// Its key context without `!TextField` (the arrows, the page keys,
/// `tab`: "wherever the keyboard is in the library") is ranked as if the
/// keyboard were always on it. Against a command of the window on the
/// same key that is right on a row (the later binding wins) and wrong in
/// the filter field, where GPUI ranks the binding without a key context
/// like the deepest one and the library's lies one level out: there the
/// window's command gets a key the field does not bind itself. So the
/// sentence "where both apply the library's command keeps it" is true of
/// the rows only. To say it exactly the verdict would need a kind of its
/// own ("keeps it on the rows, loses it in the field"), which nothing
/// that words a verdict has. Whether such a key is *taken* does not rest
/// on it for any default key: each of them is also a key of a list or a
/// dialog that never has the keyboard itself, which a key of the window
/// does take (`tests/suite/keybindings.rs`,
/// `a_key_of_the_window_against_a_key_of_the_whole_library`, holds both
/// what GPUI does and this).
pub fn places() -> Places {
    use writer_assistant_quick::actions::KEY_CONTEXT as QUICK_CHAT;
    use writer_assistant_ui::actions as chat;
    use writer_comments as comments;
    use writer_editor::actions::KEY_CONTEXT as EDITOR;
    use writer_editor_tools::actions as tools;
    use writer_library::KEY_CONTEXT as LIBRARY;
    use writer_ui::{
        button, confirm, focus, modal, picker, segmented, text_field::KEY_CONTEXT as FIELD, toggle,
    };

    // An open page has fields of its own: text is typed there, so no key of
    // a page command can take a letter from the page.
    let mut places = Places::new()
        .typing(EDITOR)
        .typing(FIELD)
        .typing(crate::pages::PAGE_VIEW);
    // The find strip of a page is the find bar, inside the page.
    places = places.inside(tools::FIND_BAR_CONTEXT, crate::pages::PAGE_VIEW);
    // The tools are only around the text: "Find" and "Add or edit link"
    // are bound on their key context, and are commands of the text to
    // whoever presses their keys (the list has them "In the text"). A
    // command of the text on one of their keys takes it from them: the
    // editor lies deeper, and its binding is the one that runs there.
    places = places.around(tools::KEY_CONTEXT, EDITOR);
    // What the tools open around the text.
    for inner in [
        tools::FIND_BAR_CONTEXT,
        tools::LINK_CONTEXT,
        picker::KEY_CONTEXT,
    ] {
        places = places.inside(inner, tools::KEY_CONTEXT);
    }
    // Where a text field can be.
    for outer in [
        tools::FIND_BAR_CONTEXT,
        tools::LINK_CONTEXT,
        picker::KEY_CONTEXT,
        LIBRARY,
        crate::settings_view::KEY_CONTEXT,
    ] {
        places = places.inside(FIELD, outer);
    }
    // Where an editor can be besides the text: the field of a comment and
    // the composers of the assistant's chats.
    for outer in [
        comments::COMPOSER_CONTEXT,
        chat::COMPOSER_CONTEXT,
        chat::PANEL_CONTEXT,
        QUICK_CHAT,
    ] {
        places = places.inside(EDITOR, outer);
    }
    places = places
        .inside(comments::COMPOSER_CONTEXT, comments::PANEL_CONTEXT)
        .inside(comments::THREADS_CONTEXT, comments::PANEL_CONTEXT);
    for outer in [chat::PANEL_CONTEXT, QUICK_CHAT] {
        places = places
            .inside(chat::COMPOSER_CONTEXT, outer)
            .inside(chat::MESSAGES_CONTEXT, outer);
    }
    places = places
        .inside(chat::HEADER_CONTEXT, chat::PANEL_CONTEXT)
        .inside(chat::CONTEXT_ROW_CONTEXT, chat::PANEL_CONTEXT);
    // The modals.
    for inner in [
        picker::KEY_CONTEXT,
        confirm::KEY_CONTEXT,
        crate::settings_view::KEY_CONTEXT,
        crate::version_history::KEY_CONTEXT,
        crate::agents_view::KEY_CONTEXT,
    ] {
        places = places.inside(inner, modal::KEY_CONTEXT);
    }
    places = places.inside(
        crate::settings_view::ROW_KEY_CONTEXT,
        crate::settings_view::KEY_CONTEXT,
    );
    // The list of keyboard shortcuts: its query field, and the question it
    // asks about a key.
    places = places
        .inside(writer_keymap_ui::KEY_CONTEXT, modal::KEY_CONTEXT)
        .inside(FIELD, writer_keymap_ui::KEY_CONTEXT)
        .inside(
            writer_keymap_ui::CHOICES_KEY_CONTEXT,
            writer_keymap_ui::KEY_CONTEXT,
        )
        .inside(
            writer_keymap_ui::RECORDING_KEY_CONTEXT,
            writer_keymap_ui::KEY_CONTEXT,
        );
    // The small controls, wherever a dialog or a panel has them.
    for inner in [
        button::KEY_CONTEXT,
        toggle::KEY_CONTEXT,
        segmented::KEY_CONTEXT,
    ] {
        for outer in [
            confirm::KEY_CONTEXT,
            focus::KEY_CONTEXT,
            tools::FIND_BAR_CONTEXT,
        ] {
            places = places.inside(inner, outer);
        }
    }
    // Never the keyboard's own place: it is in a field, a row, an editor
    // or a button inside them.
    for container in [
        tools::FIND_BAR_CONTEXT,
        tools::LINK_CONTEXT,
        picker::KEY_CONTEXT,
        modal::KEY_CONTEXT,
        confirm::KEY_CONTEXT,
        crate::settings_view::KEY_CONTEXT,
        writer_keymap_ui::KEY_CONTEXT,
        comments::PANEL_CONTEXT,
        comments::COMPOSER_CONTEXT,
        chat::PANEL_CONTEXT,
        chat::COMPOSER_CONTEXT,
        QUICK_CHAT,
    ] {
        places = places.container(container);
    }
    places
}

/// Whether `binding` is left out of what the user can change (see the
/// [module docs](self)).
fn is_kept_as_it_is(binding: &KeyBinding) -> bool {
    context_of(binding).as_deref() == Some(crate::window::ERROR_WINDOW_KEY_CONTEXT)
}

/// The command in its place that `binding` is a key of, as the model
/// names it.
pub fn target_of(binding: &KeyBinding) -> Target {
    Target::new(binding.action().name(), context_of(binding).as_deref())
}

/// Takes the keymap as it is assembled right now as the defaults and
/// installs it again, rebuilt. Called once, in [`crate::init`], when
/// every crate has bound its keys and registered its commands, before the
/// menu bar is made.
pub fn take_defaults(cx: &mut App) {
    let defaults: Vec<KeyBinding> = {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap.bindings().cloned().collect()
    };
    let mut handed = Vec::with_capacity(defaults.len());
    let mut model_defaults = Defaults::new();
    for (index, binding) in defaults.iter().enumerate() {
        if is_kept_as_it_is(binding) {
            continue;
        }
        let input = binding.action_input();
        let Ok(default) = DefaultBinding::new(
            binding.action().name(),
            input.as_ref().map(|input| input.as_ref()),
            context_of(binding).as_deref(),
            &keys_of(binding),
        ) else {
            continue;
        };
        model_defaults.push(default);
        handed.push(index);
    }
    // The commands without a key: the user can give them one.
    if let Some(registry) = cx.try_global::<crate::commands::CommandRegistry>() {
        for command in registry.commands() {
            let name = command.action_name();
            if !model_defaults.knows_command(&writer_keymap::Command::new(name)) {
                model_defaults.add_keyless(Target::new(name, home_context(name)));
            }
        }
    }
    let model = writer_keymap::Keymap::new(model_defaults, places());
    let effective = model.effective();
    cx.set_global(AppKeymap {
        defaults,
        handed,
        model,
        effective,
    });
    refresh_listed(cx);
    install(cx);
}

/// Tells the keymap how it is listed right now (`writer_keymap::Listed`),
/// so that what it says about a key is what the list shows:
///
/// - **which commands are not there**: those of a part of the app that is
///   not shown ([`crate::shortcuts::hidden_targets`]; today the
///   assistant's, while it is not shown). Their keys count for nothing: no
///   verdict names them, no row is warned about them, an import counts no
///   collision with them. They stay bound, and the user's changes to them
///   stay stored;
/// - **the rows** ([`crate::shortcuts::listed_rows`]): the key contexts
///   that are one command with one key to the user, under the name of
///   their place. A key is judged for its whole row, a command it
///   collides with is named once, and two rows under one place name are
///   in the same place ("taken", never "another place").
///
/// Nothing is installed by this. Called when the defaults are taken, with
/// every change of a key (the rows follow the keys), and whenever a
/// category of commands is hidden or shown
/// ([`crate::commands::set_category_hidden`]).
pub fn refresh_listed(cx: &mut App) {
    if !cx.has_global::<AppKeymap>() {
        return;
    }
    let (hidden, rows) = crate::shortcuts::listed(cx);
    let model = &mut cx.global_mut::<AppKeymap>().model;
    model.set_absent(hidden);
    model.set_rows(rows);
}

/// The default bindings, in the order the crates bound them.
pub fn defaults(cx: &App) -> &[KeyBinding] {
    cx.try_global::<AppKeymap>()
        .map(|keymap| keymap.defaults.as_slice())
        .unwrap_or_default()
}

/// The keymap as data: the defaults, the places and the user's changes.
/// `None` before [`take_defaults`] (an app that was not set up with
/// [`crate::init`]).
pub fn model(cx: &App) -> Option<&writer_keymap::Keymap> {
    cx.try_global::<AppKeymap>().map(|keymap| &keymap.model)
}

/// What is installed: every binding that runs, and every command in its
/// place with the keys it has now and by default.
pub fn effective(cx: &App) -> Option<&Effective> {
    cx.try_global::<AppKeymap>().map(|keymap| &keymap.effective)
}

/// The user's changes, as they are installed.
pub fn changes(cx: &App) -> Changes {
    model(cx)
        .map(|model| model.changes().clone())
        .unwrap_or_default()
}

/// How many commands in a key context have other keys than their
/// defaults. A command bound in several key contexts of one place counts
/// once for each: what the user is told is
/// [`crate::workspace::changed_command_count`], which counts as the list
/// does.
pub fn changed_count(cx: &App) -> usize {
    effective(cx).map_or(0, |effective| effective.changed().count())
}

/// Puts `changes` in the place of the user's changes, installs the keymap
/// they give and builds the menu bar again, so that everything that shows
/// a key or obeys one follows at once. Returns the changes as they were
/// (for Undo). Nothing is installed when they are the same.
///
/// This does not store anything: the workspace does, for the changes the
/// user makes.
pub fn set_changes(changes: Changes, cx: &mut App) -> Changes {
    let Some(keymap) = cx.try_global::<AppKeymap>() else {
        return Changes::new();
    };
    if *keymap.model.changes() == changes {
        return changes;
    }
    let keymap = cx.global_mut::<AppKeymap>();
    let before = keymap.model.set_changes(changes);
    keymap.effective = keymap.model.effective();
    refresh_listed(cx);
    install(cx);
    menus::install(cx);
    before
}

/// [`set_changes`] with the value the library has stored (`None`: nothing
/// is stored, the default keys). A value that cannot be read gives the
/// default keys and the error; the caller says so and leaves what is
/// stored alone.
pub fn set_stored_changes(stored: Option<&str>, cx: &mut App) -> Option<StoredError> {
    match stored.map(Changes::from_stored) {
        None => {
            set_changes(Changes::new(), cx);
            None
        }
        Some(Ok(changes)) => {
            set_changes(changes, cx);
            None
        }
        Some(Err(error)) => {
            set_changes(Changes::new(), cx);
            Some(error)
        }
    }
}

/// The action of the command named `name`: a copy of the one a default
/// binding or a registered command has, or else one built by its name.
pub fn action_named(name: &str, cx: &App) -> Option<Box<dyn Action>> {
    if let Some(binding) = defaults(cx)
        .iter()
        .find(|binding| binding.action().name() == name)
    {
        return Some(binding.action().boxed_clone());
    }
    if let Some(command) = cx
        .try_global::<crate::commands::CommandRegistry>()
        .and_then(|registry| {
            registry
                .commands()
                .iter()
                .find(|command| command.action_name() == name)
        })
    {
        return Some(command.action());
    }
    cx.build_action(name, None).ok()
}

/// The GPUI binding for one binding of the model. `None` for a key GPUI
/// cannot read or a command it cannot build, which leaves that key out.
fn gpui_binding(keymap: &AppKeymap, binding: &EffectiveBinding, cx: &App) -> Option<KeyBinding> {
    match binding.source {
        // The action and the key context of the default it stands for.
        Some(source) => {
            let default = keymap.defaults.get(*keymap.handed.get(source)?)?;
            rebound(default, &binding.spelling)
        }
        // A key for a command that has none by default.
        None => {
            let action = action_named(&binding.target.command.action, cx)?;
            let predicate = match binding.target.context.text() {
                Some(text) => Some(Rc::new(KeyBindingContextPredicate::parse(text).ok()?)),
                None => None,
            };
            bind(&binding.spelling, action.as_ref(), predicate, None, None)
        }
    }
}

/// The bindings that run, in order: what the model says to bind, with the
/// bindings that were not handed over to it where they stood. Every one
/// is built from its parts, changed or not, so there is one way a binding
/// gets into the keymap.
fn effective_bindings(keymap: &AppKeymap, cx: &App) -> Vec<KeyBinding> {
    // The bindings the model does not know, by their place in the defaults.
    let mut kept: Vec<usize> = (0..keymap.defaults.len())
        .filter(|index| !keymap.handed.contains(index))
        .rev()
        .collect();
    let mut bindings = Vec::with_capacity(keymap.defaults.len());
    let as_it_is = |index: usize| {
        let binding = &keymap.defaults[index];
        rebound(binding, &keys_of(binding)).unwrap_or_else(|| binding.clone())
    };
    for binding in keymap.effective.bindings() {
        if let Some(position) = binding
            .source
            .and_then(|source| keymap.handed.get(source).copied())
        {
            while kept.last().is_some_and(|index| *index < position) {
                bindings.extend(kept.pop().map(as_it_is));
            }
        }
        bindings.extend(gpui_binding(keymap, binding, cx));
    }
    while let Some(index) = kept.pop() {
        bindings.push(as_it_is(index));
    }
    bindings
}

/// Clears GPUI's keymap and binds it again: the menu bar's own bindings
/// first, then the bindings that run. Binding keys refreshes every
/// window.
fn install(cx: &mut App) {
    let effective = cx
        .try_global::<AppKeymap>()
        .map(|keymap| effective_bindings(keymap, cx))
        .unwrap_or_default();
    let for_menus = menus::menu_key_bindings(&Keymap::new(effective.clone()));
    cx.clear_key_bindings();
    cx.bind_keys(for_menus.into_iter().chain(effective));
}

/// What installing the keymap costs, part by part ([`measure_install`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct InstallCost {
    /// Working out what to bind from the defaults and the changes.
    pub effective: std::time::Duration,
    /// Building every GPUI binding from its parts, and the menu bar's.
    pub build: std::time::Duration,
    /// Clearing GPUI's keymap and binding it again.
    pub bind: std::time::Duration,
    /// Building the native menu bar again.
    pub menus: std::time::Duration,
    /// How many bindings were bound.
    pub bindings: usize,
}

/// Installs the keymap as it is once more and says what each part cost:
/// what [`set_changes`] does for a change, without a change. For the
/// `keyscost` step of a scripted run.
pub fn measure_install(cx: &mut App) -> Option<InstallCost> {
    use std::time::Instant;
    let start = Instant::now();
    let effective = cx.try_global::<AppKeymap>()?.model.effective();
    let after_effective = Instant::now();
    cx.global_mut::<AppKeymap>().effective = effective;
    let effective = effective_bindings(cx.global::<AppKeymap>(), cx);
    let for_menus = menus::menu_key_bindings(&Keymap::new(effective.clone()));
    let bindings = effective.len() + for_menus.len();
    let after_build = Instant::now();
    cx.clear_key_bindings();
    cx.bind_keys(for_menus.into_iter().chain(effective));
    let after_bind = Instant::now();
    menus::install(cx);
    let after_menus = Instant::now();
    Some(InstallCost {
        effective: after_effective - start,
        build: after_build - after_effective,
        bind: after_bind - after_build,
        menus: after_menus - after_bind,
        bindings,
    })
}

/// The keys of a binding as text GPUI parses again (`cmd-shift-p`, and
/// `ctrl-x ctrl-s` for a sequence).
pub fn keys_of(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|key| key.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key context of a binding as text (`None`: it has none).
pub fn context_of(binding: &KeyBinding) -> Option<String> {
    binding.predicate().map(|predicate| predicate.to_string())
}

/// The key the app shows for `action` in one place of the window: where
/// the key contexts `place` hold (one name, or several from the outside in:
/// `"FindBar > TextField"`). Among the bindings of the action that apply
/// there, one bound for a key context comes before one that works anywhere,
/// and the one bound last before the others, as everywhere a key is shown.
/// `None` when the action has no key there (the user may have taken it
/// away): whatever names the key then leaves it out.
///
/// For the texts that name a key outside a list of commands: the footer
/// of a modal, a tooltip, a sentence. They are drawn from the keymap each
/// time, so they follow a change at once.
pub fn hint_in(action: &dyn Action, place: &str, cx: &App) -> Option<KeyHint> {
    let stack: Vec<KeyContext> = place
        .split('>')
        .filter_map(|name| KeyContext::parse(name.trim()).ok())
        .collect();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut anywhere = None;
    let mut here = None;
    for binding in keymap.bindings_for_action(action) {
        match binding.predicate() {
            None => anywhere = Some(binding),
            Some(predicate) if predicate.depth_of(&stack).is_some() => here = Some(binding),
            Some(_) => {}
        }
    }
    here.or(anywhere).map(KeyHint::from_binding)
}

/// Every key of `action` in one place, in the order they are bound, each
/// once: `"↩ or Space"`. For a sentence that says which keys do something.
/// `None` when the action has no key there.
pub fn keys_sentence_in(action: &dyn Action, place: &str, cx: &App) -> Option<String> {
    let stack: Vec<KeyContext> = place
        .split('>')
        .filter_map(|name| KeyContext::parse(name.trim()).ok())
        .collect();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut keys: Vec<String> = Vec::new();
    for binding in keymap.bindings_for_action(action) {
        let applies = binding
            .predicate()
            .is_some_and(|predicate| predicate.depth_of(&stack).is_some());
        let text = KeyHint::from_binding(binding).text();
        if applies && !keys.contains(&text) {
            keys.push(text);
        }
    }
    (!keys.is_empty()).then(|| keys.join(" or "))
}

/// The keys of several actions in one place, as one text: `"↑ ↓"` for
/// "previous" and "next". An action without a key there is left out;
/// `None` when none of them has one.
pub fn keys_text_in(actions: &[&dyn Action], place: &str, cx: &App) -> Option<String> {
    let keys: Vec<String> = actions
        .iter()
        .filter_map(|action| hint_in(*action, place, cx))
        .map(|hint| hint.text())
        .collect();
    (!keys.is_empty()).then(|| keys.join(" "))
}

/// `binding` on other keys: the same action, key context, meta index and
/// action input. `None` when `keys` is not a key GPUI can read.
pub fn rebound(binding: &KeyBinding, keys: &str) -> Option<KeyBinding> {
    bind(
        keys,
        binding.action(),
        binding.predicate(),
        binding.meta(),
        binding.action_input(),
    )
}

/// A binding made from its parts, the way `KeyBinding::new` makes one.
pub fn bind(
    keys: &str,
    action: &dyn Action,
    predicate: Option<Rc<KeyBindingContextPredicate>>,
    meta: Option<gpui::KeyBindingMetaIndex>,
    input: Option<gpui::SharedString>,
) -> Option<KeyBinding> {
    // No key at all is not a binding.
    keys.split_whitespace().next()?;
    let mut binding = KeyBinding::load(
        keys,
        action.boxed_clone(),
        predicate,
        false,
        input,
        &DummyKeyboardMapper,
    )
    .ok()?;
    if let Some(meta) = meta {
        binding.set_meta(meta);
    }
    Some(binding)
}
