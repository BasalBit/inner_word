//! Which engine the assistant runs on, and whether it is there.
//!
//! The engine is a terminal agent behind `writer_assistant::Backend`
//! (Claude Code is the first). Which one the workspaces of this app use is
//! an app global, like the spell service:
//!
//! - `main` installs Claude Code ([`use_claude_code`]);
//! - a test installs `writer_assistant::FakeBackend` with [`set_backend`]
//!   before it opens a workspace, and **no test ever runs the real
//!   program** (a message spends the user's own allowance);
//! - a scripted run gets the fake backend when [`FAKE_ENV`] is set, and no
//!   assistant at all otherwise, so a script can never send anything;
//! - without a backend there is no assistant: nothing of it is shown.
//!
//! # Availability
//!
//! Whether the engine is installed and the user signed in is found out by
//! running its program, which takes a moment: [`Host::check`] does it on a
//! background thread, at launch (the workspace asks once) and every time
//! the settings are opened. The app never waits for it. Until the first
//! answer is in, the assistant is not shown ([`Host::availability`] is
//! `None`); when the answer is "not installed" it stays hidden, except for
//! one quiet line in the settings that says what it needs.

use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use writer_assistant::{Availability, Backend, BackendInfo, Catalog};

/// Environment variable of a scripted run: when set to a non-empty value
/// the app uses the engine crate's fake backend, which answers from a
/// script and never starts a program, so that the assistant can be looked
/// at (and captured) without spending anything. Its value chooses what the
/// fake says about itself:
///
/// | Value | The assistant is |
/// |---|---|
/// | `ready` (or `1`) | installed and signed in |
/// | `signed-out` | installed, not signed in |
/// | `missing` | not installed |
///
/// Only read while `THE_WRITER_AUTOMATION` is set.
pub const FAKE_ENV: &str = "THE_WRITER_ASSISTANT_FAKE";

/// The engine and what is known about it.
pub struct Host {
    backend: Arc<dyn Backend>,
    info: BackendInfo,
    availability: Option<Availability>,
    catalog: Catalog,
    /// How many checks have finished (tests wait for one).
    checks: usize,
    check: Option<Task<()>>,
}

/// What a [`Host`] tells its observers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// A check finished: [`Host::availability`] is fresh (it may or may not
    /// differ from before).
    Checked,
}

impl EventEmitter<HostEvent> for Host {}

struct HostGlobal(Entity<Host>);

impl Global for HostGlobal {}

/// Installs `backend` as the engine of the assistant for every workspace
/// opened from now on. Nothing is looked up yet: the first workspace asks
/// for a check.
pub fn set_backend(backend: Arc<dyn Backend>, cx: &mut App) {
    let host = cx.new(|_| Host::new(backend));
    cx.set_global(HostGlobal(host));
}

/// Removes the engine: workspaces opened from now on have no assistant.
pub fn clear_backend(cx: &mut App) {
    if cx.has_global::<HostGlobal>() {
        cx.remove_global::<HostGlobal>();
    }
}

/// Installs Claude Code, with the login the user has in it. Called by
/// `main` and by nothing else.
pub fn use_claude_code(cx: &mut App) {
    set_backend(Arc::new(writer_assistant::ClaudeCode::new()), cx);
}

/// The host of the installed engine, if there is one.
pub fn host(cx: &App) -> Option<Entity<Host>> {
    cx.try_global::<HostGlobal>().map(|global| global.0.clone())
}

impl Host {
    fn new(backend: Arc<dyn Backend>) -> Self {
        let info = backend.info();
        let catalog = backend.catalog();
        // A backend that already knows (a fake, or a second window) can
        // say so without being asked again.
        let availability = backend.cached_availability();
        Self {
            backend,
            info,
            availability,
            catalog,
            checks: 0,
            check: None,
        }
    }

    /// The engine.
    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// What the engine is called, and how to install it and sign in.
    pub fn info(&self) -> &BackendInfo {
        &self.info
    }

    /// Whether the engine is installed and the user signed in; `None`
    /// until the first check has answered.
    pub fn availability(&self) -> Option<&Availability> {
        self.availability.as_ref()
    }

    /// Whether the engine's program is on this Mac, as far as is known.
    pub fn is_installed(&self) -> bool {
        self.availability
            .as_ref()
            .is_some_and(Availability::is_installed)
    }

    /// Whether the engine is installed and says the user is not signed in.
    pub fn is_signed_out(&self) -> bool {
        self.availability
            .as_ref()
            .is_some_and(Availability::is_signed_out)
    }

    /// The models and effort levels the engine offers, with the defaults
    /// of the two chats.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// How many checks have finished.
    pub fn checks_finished(&self) -> usize {
        self.checks
    }

    /// Whether a check is running.
    pub fn is_checking(&self) -> bool {
        self.check.is_some()
    }

    /// Looks for the engine again, on a background thread: its program,
    /// its version, whether the user is signed in, and the models it
    /// offers. Returns at once; [`HostEvent::Checked`] follows. A check
    /// that is already running is not started twice.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if self.check.is_some() {
            return;
        }
        let backend = self.backend.clone();
        let look = cx.background_spawn(async move {
            let availability = backend.recheck();
            // The list of models is the program's to say, where it can;
            // it costs nothing and sends nothing.
            let catalog = if availability.is_installed() {
                backend.refresh_catalog()
            } else {
                backend.catalog()
            };
            (availability, catalog)
        });
        self.check = Some(cx.spawn(async move |this, cx| {
            let (availability, catalog) = look.await;
            this.update(cx, |this, cx| {
                this.check = None;
                this.checks += 1;
                this.availability = Some(availability);
                this.catalog = catalog;
                cx.emit(HostEvent::Checked);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The line the settings show about the engine: its name, its version
    /// and the account, or what is missing.
    pub fn status_line(&self) -> String {
        let name = &self.info.name;
        match &self.availability {
            None => format!("Looking for {name}…"),
            Some(Availability::NotInstalled) => format!("{name} is not installed."),
            Some(availability) => {
                let mut line = match availability.version() {
                    Some(version) => format!("{name} {version}"),
                    None => name.clone(),
                };
                if availability.is_signed_out() {
                    line.push_str(", not signed in");
                } else if let Some(account) = availability.account() {
                    line.push_str(", ");
                    line.push_str(account);
                }
                line
            }
        }
    }
}
