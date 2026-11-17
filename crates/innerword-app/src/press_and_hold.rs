//! Held keys with Vim mode on: repeat, not the accent menu.
//!
//! macOS answers a held letter key with its accent menu ("press and hold")
//! instead of repeating the key. That is wrong for Vim, where holding `j`
//! or `x` must repeat. The system setting behind it is the user default
//! `ApplePressAndHoldEnabled`, which an app can set for itself alone. Inner
//! Word sets it to `false` while Vim mode is on and removes its own value
//! when Vim mode is off, so the system-wide choice applies again.
//!
//! # When it takes effect
//!
//! The text input system (HIToolbox) reads the default **once per
//! process**, when the first key reaches it, and keeps the answer (seen in
//! its `IsOKToUsePressAndHold` check: a function-local flag that is filled
//! from `TBGetAppBooleanPref` the first time and only read afterwards). So:
//!
//! - what is set **at launch**, before any key is pressed, holds for that
//!   run of the app ([`sync`] is first called while the window is built);
//! - a change **while the app runs** is stored at once but only acts from
//!   the next launch. [`relaunch_needed`] is then true and the settings
//!   say so.
//!
//! In Normal and Visual mode a held key should repeat either way: GPUI
//! gives a printable key to the key interceptors first (`handle_key_event`
//! in its macOS window code), Vim takes it there, and the text input
//! system, which is what starts the accent menu, never sees it. This
//! follows from reading that code; it was not tried on a keyboard. The
//! default decides what a held letter does where text is typed (Insert
//! mode, the command palette, a rename field).
//!
//! # Who writes the default
//!
//! Only the real app: `main` calls [`manage_user_default`]. Tests and
//! scripted runs keep the bookkeeping (what is wanted, whether a relaunch
//! is needed) without touching the user's preferences.

use gpui::{App, Global};

/// The name of the user default.
pub const USER_DEFAULT: &str = "ApplePressAndHoldEnabled";

/// Environment variable: a scripted run (see [`crate::automation`]) writes
/// the user default only when this is set to a non-empty value.
pub const MANAGE_ENV: &str = "INNER_WORD_USER_DEFAULTS";

/// Where [`sync`] stores the choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Backend {
    /// Nowhere: bookkeeping only (tests, scripted runs).
    #[default]
    Off,
    /// The app's user defaults.
    UserDefaults,
    /// Nowhere, but everything else behaves as in the real app.
    Dry,
}

/// What the app knows about the accent menu on held keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressAndHold {
    /// Where the choice is stored.
    backend: Backend,
    /// Whether the app wanted held keys to repeat when it started: what
    /// this run of the app does.
    at_launch: Option<bool>,
    /// Whether the app wants held keys to repeat now: what the next run
    /// will do.
    wanted: Option<bool>,
}

impl Global for PressAndHold {}

impl PressAndHold {
    /// Whether held keys repeat in this run of the app (`None` before the
    /// first [`sync`]).
    pub fn repeats_now(&self) -> Option<bool> {
        self.at_launch
    }

    /// Whether held keys will repeat from the next launch on.
    pub fn repeats_next(&self) -> Option<bool> {
        self.wanted
    }

    /// Whether the stored choice differs from what this run of the app
    /// does.
    pub fn relaunch_needed(&self) -> bool {
        self.at_launch != self.wanted
    }

    /// Whether this process stores the choice, so that a relaunch acts on
    /// it.
    pub fn is_managed(&self) -> bool {
        self.backend != Backend::Off
    }

    /// Whether the user has to be told that the choice made in this run
    /// only acts after a relaunch.
    pub fn restart_note_due(&self) -> bool {
        self.is_managed() && self.relaunch_needed()
    }
}

/// From now on [`sync`] writes the user default. Called by `main` for a
/// normal launch.
pub fn manage_user_default(cx: &mut App) {
    cx.default_global::<PressAndHold>().backend = Backend::UserDefaults;
}

/// Like [`manage_user_default`] without writing anything: for tests of
/// what the app says about relaunching.
#[doc(hidden)]
pub fn manage_without_writing(cx: &mut App) {
    cx.default_global::<PressAndHold>().backend = Backend::Dry;
}

/// Records, and in the real app stores, whether held keys should repeat
/// (`true` while Vim mode is on). The first call is the launch state.
pub fn sync(key_repeat: bool, cx: &mut App) {
    let state = cx.default_global::<PressAndHold>();
    if state.at_launch.is_none() {
        state.at_launch = Some(key_repeat);
    }
    if state.wanted == Some(key_repeat) {
        return;
    }
    state.wanted = Some(key_repeat);
    if state.backend == Backend::UserDefaults {
        write_user_default(key_repeat);
    }
}

/// The current bookkeeping.
pub fn state(cx: &App) -> PressAndHold {
    cx.try_global::<PressAndHold>().copied().unwrap_or_default()
}

/// Whether the choice made in this run only acts after a relaunch.
pub fn relaunch_needed(cx: &App) -> bool {
    state(cx).relaunch_needed()
}

/// Forgets the launch state, as if the app had just started. For tests,
/// where several "launches" share one process.
#[doc(hidden)]
pub fn reset(cx: &mut App) {
    let backend = state(cx).backend;
    cx.set_global(PressAndHold {
        backend,
        ..PressAndHold::default()
    });
}

/// `key_repeat`: store `ApplePressAndHoldEnabled = false` for this app.
/// Otherwise remove the app's own value, so the system-wide one applies.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
fn write_user_default(key_repeat: bool) {
    use objc::{
        class, msg_send,
        runtime::{NO, Object},
        sel, sel_impl,
    };
    use std::ffi::CString;

    let Ok(key) = CString::new(USER_DEFAULT) else {
        return;
    };
    // SAFETY: plain Foundation calls on the main thread. The NSString is
    // autoreleased and only used within these calls; `standardUserDefaults`
    // is a shared object that outlives them.
    unsafe {
        let key: *mut Object = msg_send![class!(NSString), stringWithUTF8String: key.as_ptr()];
        let defaults: *mut Object = msg_send![class!(NSUserDefaults), standardUserDefaults];
        if key.is_null() || defaults.is_null() {
            return;
        }
        if key_repeat {
            let _: () = msg_send![defaults, setBool: NO forKey: key];
        } else {
            let _: () = msg_send![defaults, removeObjectForKey: key];
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn write_user_default(_key_repeat: bool) {}
