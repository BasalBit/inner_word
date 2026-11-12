//! Two fingers pinched on the trackpad zoom the text.
//!
//! GPUI 0.2.2 delivers no magnify event: its window only turns key, mouse
//! and scroll events into its own. AppKit, though, lets an app look at
//! every event it is about to dispatch (`+[NSEvent
//! addLocalMonitorForEventsMatchingMask:handler:]`), and that is all this
//! is: a monitor for magnify events and nothing else, which reads what the
//! pinch says (how much, where) and hands the event on untouched. Scroll
//! events never come near it, so scrolling cannot be disturbed by it.
//!
//! The monitor runs inside AppKit's event dispatch, where GPUI must not be
//! entered. It only puts what it read into a channel; a task of the app
//! takes it from there and calls [`Workspace::pinch`], which zooms the
//! text around the pointer when the pointer is on the text.
//!
//! Only the real app watches ([`watch`], from `main`). The way from a
//! pinch to the zoom ([`Pinch::factor`], [`Workspace::pinch`]) is tested;
//! the monitor itself can only be tried with fingers.

use std::sync::mpsc::{Receiver, channel};

use gpui::{App, WindowHandle, point, px};

use crate::workspace::Workspace;

/// What a magnify event says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    /// The window the fingers were over, by its number.
    pub window: isize,
    /// How much the pinch asks for since the last event: the change of
    /// magnification, a small number, positive when the fingers part.
    pub magnification: f32,
    /// Where the pointer is, in the window's own coordinates: from its
    /// left edge and from the top of its content.
    pub x: f32,
    /// See [`Pinch::x`].
    pub y: f32,
}

impl Pinch {
    /// The factor a magnification stands for: what the zoom is multiplied
    /// by. One event never more than doubles or halves it.
    pub fn factor(magnification: f32) -> f32 {
        if !magnification.is_finite() {
            return 1.;
        }
        (1. + magnification).clamp(0.5, 2.)
    }
}

/// Several events that arrived together are one pinch: the product of
/// their factors, at the place of the last one. `None` for no events.
pub fn combined(pinches: &[Pinch]) -> Option<(f32, f32, f32)> {
    let last = pinches.last()?;
    let factor: f32 = pinches
        .iter()
        .map(|pinch| Pinch::factor(pinch.magnification))
        .product();
    Some((factor, last.x, last.y))
}

/// Has pinches on the trackpad over `window` zoom its text, from now
/// until the app quits. Called by `main` once the main window is open.
pub fn watch(window: WindowHandle<Workspace>, cx: &mut App) {
    let number = window
        .update(cx, |_, window, _| crate::screenshot::window_number(window))
        .ok()
        .and_then(Result::ok);
    let Some(number) = number else {
        // No native window to watch (it is not on screen): no pinch.
        return;
    };
    let (sender, receiver) = channel::<Pinch>();
    if !native::monitor_magnify(sender) {
        return;
    }
    let number = number as isize;
    cx.spawn(async move |cx| {
        let mut receiver = receiver;
        loop {
            // Waiting for the next pinch occupies a background thread, as
            // waiting for a signal does (`crate::signals`).
            let (pinches, back) = cx
                .background_executor()
                .spawn(async move { next(receiver) })
                .await;
            let Some(pinches) = pinches else {
                break;
            };
            receiver = back;
            let here: Vec<Pinch> = pinches
                .into_iter()
                .filter(|pinch| pinch.window == number)
                .collect();
            let Some((factor, x, y)) = combined(&here) else {
                continue;
            };
            let alive = window.update(cx, |workspace, _, cx| {
                workspace.pinch(factor, point(px(x), px(y)), cx);
            });
            if alive.is_err() {
                // The window is gone.
                break;
            }
        }
    })
    .detach();
}

/// Blocks until a pinch arrives, and takes every other one that is
/// already waiting with it. `None` when nobody will send any more.
fn next(receiver: Receiver<Pinch>) -> (Option<Vec<Pinch>>, Receiver<Pinch>) {
    let Ok(first) = receiver.recv() else {
        return (None, receiver);
    };
    let mut pinches = vec![first];
    while let Ok(more) = receiver.try_recv() {
        pinches.push(more);
    }
    (Some(pinches), receiver)
}

#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
mod native {
    use std::sync::mpsc::Sender;

    use block::ConcreteBlock;
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};

    use super::Pinch;

    /// `NSEventTypeMagnify`.
    const MAGNIFY: usize = 30;
    /// `NSEventMaskMagnify`.
    const MAGNIFY_MASK: u64 = 1 << MAGNIFY;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsRect {
        origin: NsPoint,
        width: f64,
        height: f64,
    }

    /// Installs the monitor. Every magnify event the app is about to
    /// dispatch is read into `sender` and handed on unchanged. Returns
    /// whether AppKit took the monitor.
    pub(super) fn monitor_magnify(sender: Sender<Pinch>) -> bool {
        let handler = ConcreteBlock::new(move |event: *mut Object| -> *mut Object {
            if let Some(pinch) = read(event) {
                // Nobody listening any more is fine.
                let _ = sender.send(pinch);
            }
            event
        });
        // The block is copied to the heap; AppKit keeps it with the
        // monitor, which lives as long as the app does.
        let handler = handler.copy();
        // SAFETY: a class method of AppKit, called on the main thread with
        // a block of the type it documents (`NSEvent *(^)(NSEvent *)`).
        let monitor: *mut Object = unsafe {
            msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: MAGNIFY_MASK
                                                                    handler: &*handler]
        };
        !monitor.is_null()
    }

    /// What a magnify event says. `None` for anything else, and for an
    /// event that has no window.
    fn read(event: *mut Object) -> Option<Pinch> {
        if event.is_null() {
            return None;
        }
        // SAFETY: `event` is the live `NSEvent` AppKit handed the monitor,
        // on the main thread; these are its plain getters. `magnification`
        // is only asked of a magnify event (it raises for others).
        unsafe {
            let kind: usize = msg_send![event, type];
            if kind != MAGNIFY {
                return None;
            }
            let window: *mut Object = msg_send![event, window];
            if window.is_null() {
                return None;
            }
            let number: isize = msg_send![window, windowNumber];
            let magnification: f64 = msg_send![event, magnification];
            let location: NsPoint = msg_send![event, locationInWindow];
            let view: *mut Object = msg_send![window, contentView];
            if view.is_null() {
                return None;
            }
            let frame: NsRect = msg_send![view, frame];
            Some(Pinch {
                window: number,
                magnification: magnification as f32,
                x: location.x as f32,
                // AppKit counts from the bottom, the app from the top.
                y: (frame.height - location.y) as f32,
            })
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use std::sync::mpsc::Sender;

    use super::Pinch;

    pub(super) fn monitor_magnify(_: Sender<Pinch>) -> bool {
        false
    }
}
