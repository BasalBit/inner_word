//! Capturing a GPUI window to a PNG for visual QA.
//!
//! GPUI 0.2.2 has no API to render a window into an image, so this asks the
//! window server for the window's pixels with `screencapture -l <window id>`.
//! That captures only this window (no other apps, no desktop), even when it is
//! partly covered, but not when it is minimized or on another Space. It needs
//! the Screen Recording permission for the process that launched the app (on
//! this machine the terminal already has it).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail};
use gpui::Window;

/// The macOS window number (`CGWindowID`) of a GPUI window.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
pub fn window_number(window: &Window) -> Result<u32> {
    use objc::{msg_send, runtime::Object, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // `Window::window_handle()` is an inherent method returning GPUI's
    // AnyWindowHandle; call the raw-window-handle trait method explicitly.
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|err| anyhow::anyhow!("no native window handle: {err}"))?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        bail!("not an AppKit window");
    };
    let ns_view = appkit.ns_view.as_ptr() as *mut Object;
    // SAFETY: `ns_view` is the live NSView backing this GPUI window; `window`
    // and `windowNumber` are plain AppKit getters called on the main thread.
    let number: isize = unsafe {
        let ns_window: *mut Object = msg_send![ns_view, window];
        if ns_window.is_null() {
            bail!("the view is not in a window yet");
        }
        msg_send![ns_window, windowNumber]
    };
    if number <= 0 {
        bail!("window is not on screen (window number {number})");
    }
    Ok(number as u32)
}

#[cfg(not(target_os = "macos"))]
pub fn window_number(_window: &Window) -> Result<u32> {
    bail!("window capture is only implemented on macOS")
}

/// Captures window `window_number` into `path` (PNG). Blocking: run it on the
/// background executor.
pub fn capture_window(window_number: u32, path: &Path) -> Result<PathBuf> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // -x: no sound, -o: no window shadow, -l: this window only.
    let output = Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{window_number}"))
        .arg("-t")
        .arg("png")
        .arg(path)
        .output()
        .context("running /usr/sbin/screencapture")?;
    if !output.status.success() {
        bail!(
            "screencapture failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let len = std::fs::metadata(path)
        .with_context(|| format!("screencapture wrote nothing at {}", path.display()))?
        .len();
    if len == 0 {
        bail!("screencapture wrote an empty file at {}", path.display());
    }
    Ok(path.to_path_buf())
}
