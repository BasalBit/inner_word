//! The items removed from WebKit's right-click menu, and the key a menu entry of the
//! owner shows.
//!
//! **Neither list is public API on macOS.** The SDK's WebKit headers declare none of
//! the `WKMenuItemIdentifier…` names; they are WebKit's own values, read in its
//! sources (`WKMenuItemIdentifiers.mm`). A later WebKit may rename one, and the item
//! then stays in the menu. It still does nothing harmful: the real block is the
//! navigation and download policy (an "Open Link in New Window" goes through
//! `webView:createWebViewWithConfiguration:…`, which answers nil; a download is
//! cancelled).

/// Removed by the item's `identifier`: the items that open a link or a resource
/// somewhere (a new window, the page itself, a download), reload, inspect, go full
/// screen or share.
pub const REMOVED: &[&str] = &[
    "WKMenuItemIdentifierOpenLink",
    "WKMenuItemIdentifierOpenLinkInNewWindow",
    "WKMenuItemIdentifierOpenImageInNewWindow",
    "WKMenuItemIdentifierOpenFrameInNewWindow",
    "WKMenuItemIdentifierOpenMediaInNewWindow",
    "WKMenuItemIdentifierDownloadLinkedFile",
    "WKMenuItemIdentifierDownloadImage",
    "WKMenuItemIdentifierDownloadMedia",
    "WKMenuItemIdentifierReload",
    "WKMenuItemIdentifierInspectElement",
    "WKMenuItemIdentifierToggleFullScreen",
    "WKMenuItemIdentifierToggleEnhancedFullScreen",
    "WKMenuItemIdentifierShareMenu",
];

/// Removed by the item's action selector: the same items as AppKit and WebKit's
/// older menus name their actions.
pub const REMOVED_ACTIONS: &[&str] = &[
    "openLinkInNewWindow:",
    "openImageInNewWindow:",
    "openFrameInNewWindow:",
    "openMediaInNewWindow:",
    "downloadLinkToDisk:",
    "downloadImageToDisk:",
    "downloadMediaToDisk:",
    "reload:",
    "reloadFromOrigin:",
    "inspectElement:",
    "toggleFullScreen:",
    "toggleEnhancedFullScreen:",
];

/// Whether an item of WebKit's menu goes, by its identifier or its action.
pub(crate) fn is_removed(identifier: Option<&str>, action: Option<&str>) -> bool {
    identifier.is_some_and(|identifier| REMOVED.contains(&identifier))
        || action.is_some_and(|action| REMOVED_ACTIONS.contains(&action))
}

/// The modifier flags of AppKit (`NSEventModifierFlag…`).
pub(crate) const SHIFT: usize = 1 << 17;
pub(crate) const CONTROL: usize = 1 << 18;
pub(crate) const OPTION: usize = 1 << 19;
pub(crate) const COMMAND: usize = 1 << 20;

/// A menu entry's title and, when it carries one after a tab, its key: the first
/// keystroke of the keymap's text (`"Find in Page…\tcmd-f"`, `"Print…\tcmd-alt-p"`)
/// as the key equivalent AppKit draws beside the item and its modifier mask. A key
/// that cannot be drawn is left out and the title stays.
pub(crate) fn title_and_key(text: &str) -> (String, Option<(String, usize)>) {
    let Some((title, keys)) = text.split_once('\t') else {
        return (text.to_string(), None);
    };
    let keystroke = keys.split_whitespace().next().unwrap_or("");
    (title.to_string(), key_equivalent(keystroke))
}

fn key_equivalent(keystroke: &str) -> Option<(String, usize)> {
    if keystroke.is_empty() {
        return None;
    }
    // `cmd--` is the minus key: the last part is the key even when it is "-".
    let (modifiers, key) = match keystroke.strip_suffix("--") {
        Some(modifiers) => (modifiers, "-"),
        None => match keystroke.rsplit_once('-') {
            Some((modifiers, key)) => (modifiers, key),
            None => ("", keystroke),
        },
    };
    let mut mask = 0;
    for modifier in modifiers.split('-').filter(|m| !m.is_empty()) {
        mask |= match modifier {
            "cmd" => COMMAND,
            "ctrl" => CONTROL,
            "alt" => OPTION,
            "shift" => SHIFT,
            _ => return None,
        };
    }
    let function = |n: u32| char::from_u32(0xF704 + n - 1).map(String::from);
    let key = match key {
        "enter" => Some("\r".to_string()),
        "escape" => Some("\u{1b}".to_string()),
        "tab" => Some("\t".to_string()),
        "space" => Some(" ".to_string()),
        "backspace" => Some("\u{8}".to_string()),
        "delete" => Some("\u{7f}".to_string()),
        "up" => Some("\u{F700}".to_string()),
        "down" => Some("\u{F701}".to_string()),
        "left" => Some("\u{F702}".to_string()),
        "right" => Some("\u{F703}".to_string()),
        "home" => Some("\u{F729}".to_string()),
        "end" => Some("\u{F72B}".to_string()),
        "pageup" => Some("\u{F72C}".to_string()),
        "pagedown" => Some("\u{F72D}".to_string()),
        name if name.len() > 1 && name.starts_with('f') => name[1..]
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=20).contains(n))
            .and_then(function),
        single if single.chars().count() == 1 => Some(single.to_lowercase()),
        _ => None,
    }?;
    Some((key, mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_takes_out_by_identifier_and_by_action() {
        assert!(is_removed(Some("WKMenuItemIdentifierOpenLink"), None));
        assert!(is_removed(
            Some("WKMenuItemIdentifierDownloadImage"),
            Some("x:")
        ));
        assert!(is_removed(Some("WKMenuItemIdentifierShareMenu"), None));
        assert!(is_removed(None, Some("openLinkInNewWindow:")));
        assert!(is_removed(Some(""), Some("reload:")));
        assert!(is_removed(None, Some("inspectElement:")));
        // What stays: Copy, Copy Link, Copy Image, Look Up, Back, Forward, Speech,
        // Paste, the media controls, and every item of another name.
        for kept in [
            "WKMenuItemIdentifierCopy",
            "WKMenuItemIdentifierCopyLink",
            "WKMenuItemIdentifierCopyImage",
            "WKMenuItemIdentifierLookUp",
            "WKMenuItemIdentifierGoBack",
            "WKMenuItemIdentifierGoForward",
            "WKMenuItemIdentifierSpeechMenu",
            "WKMenuItemIdentifierPaste",
            "WKMenuItemIdentifierShowHideMediaControls",
        ] {
            assert!(
                !is_removed(Some(kept), Some("forwardContextMenuAction:")),
                "{kept}"
            );
        }
        assert!(!is_removed(None, Some("copy:")));
        assert!(!is_removed(None, None));
    }

    #[test]
    fn a_title_may_carry_its_key() {
        assert_eq!(
            title_and_key("Reload Page"),
            ("Reload Page".to_string(), None)
        );
        assert_eq!(
            title_and_key("Find in Page…\tcmd-f"),
            (
                "Find in Page…".to_string(),
                Some(("f".to_string(), COMMAND))
            )
        );
        assert_eq!(
            title_and_key("Export…\tcmd-shift-s"),
            (
                "Export…".to_string(),
                Some(("s".to_string(), COMMAND | SHIFT))
            )
        );
        assert_eq!(
            title_and_key("Zoom Out\tcmd--"),
            ("Zoom Out".to_string(), Some(("-".to_string(), COMMAND)))
        );
        assert_eq!(
            title_and_key("Menu\tctrl-enter shift-f10"),
            ("Menu".to_string(), Some(("\r".to_string(), CONTROL)))
        );
        assert_eq!(
            title_and_key("Help\tf1"),
            ("Help".to_string(), Some(("\u{F704}".to_string(), 0)))
        );
        assert_eq!(
            title_and_key("Print…\tcmd-alt-p"),
            (
                "Print…".to_string(),
                Some(("p".to_string(), COMMAND | OPTION))
            )
        );
        // A key that cannot be drawn: the title alone.
        assert_eq!(title_and_key("Odd\thyper-x"), ("Odd".to_string(), None));
        assert_eq!(title_and_key("Odd\tcmd-wheel"), ("Odd".to_string(), None));
        assert_eq!(title_and_key("Odd\t"), ("Odd".to_string(), None));
    }
}
