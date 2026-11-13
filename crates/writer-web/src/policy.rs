//! Where a page may go, as data: the navigation policy of section 5 of the design
//! and the gate that lets only the reader open a link in the browser.

use crate::SCHEME;

/// How long after a real click or key of the reader a link may still be opened in the
/// browser. A script's `a.click()` looks to WebKit like the reader's click; it is
/// told apart only by time.
pub(crate) const READER_WINDOW_S: f64 = 1.0;

/// Whether the reader acted: a real `mouseUp:` or `keyDown:` of the view no more than
/// [`READER_WINDOW_S`] before `now` (both in seconds of the system's uptime, the clock
/// of `NSEvent.timestamp`), and WebKit's own word where it gives one
/// (`_isUserInitiated`).
pub(crate) fn reader_acted(
    last_event: Option<f64>,
    now: f64,
    user_initiated: Option<bool>,
) -> bool {
    if user_initiated == Some(false) {
        return false;
    }
    // An event stamped a little after `now` is the same moment (the clocks are one,
    // but a synthesized event may be stamped ahead).
    last_event.is_some_and(|at| now - at <= READER_WINDOW_S && at - now <= 0.25)
}

/// What becomes of a navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// A file of the page, or a document that can reach nothing.
    Allow,
    /// Cancelled, and counted among the blocked navigations.
    Cancel,
    /// Cancelled in the page and handed to the reader's browser.
    OpenOutside,
}

/// One navigation as the policy sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Navigation<'a> {
    pub url: &'a str,
    /// The host of the page's own files (`page-12`, `check`).
    pub own_host: &'a str,
    /// The navigation is of the main frame (or of a new window).
    pub main_frame: bool,
    /// WebKit calls it "link activated": a click on a link, or `enter` on one.
    pub link: bool,
    /// WebKit wants to download the target.
    pub download: bool,
    /// The reader acted ([`reader_acted`]).
    pub reader: bool,
}

/// The scheme of a URL, lowercased.
pub(crate) fn scheme_of(url: &str) -> String {
    url.split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether a URL is a link that may be handed to the reader's browser.
pub(crate) fn opens_outside(url: &str) -> bool {
    matches!(scheme_of(url).as_str(), "http" | "https" | "mailto")
}

/// The policy: the page's own files, `about:blank` and `about:srcdoc` load; a `data:`
/// or `blob:` document loads in a frame (it reaches nothing); a link to another site
/// that the reader opened goes to the browser; everything else is cancelled.
pub(crate) fn decide(navigation: &Navigation<'_>) -> Decision {
    if navigation.download {
        return Decision::Cancel;
    }
    let url = navigation.url;
    match scheme_of(url).as_str() {
        scheme if scheme == SCHEME => match crate::http::split_url(url) {
            Some((host, _)) if host == navigation.own_host.to_ascii_lowercase() => Decision::Allow,
            _ => Decision::Cancel,
        },
        "about" => {
            let rest = url[6..].split(['?', '#']).next().unwrap_or("");
            if rest == "blank" || rest == "srcdoc" {
                Decision::Allow
            } else {
                Decision::Cancel
            }
        }
        "data" | "blob" if !navigation.main_frame => Decision::Allow,
        "http" | "https" | "mailto" if navigation.link && navigation.reader => {
            Decision::OpenOutside
        }
        _ => Decision::Cancel,
    }
}

/// What becomes of a page's request for a new window (`window.open`, a link with
/// `target=_blank` that the policy let through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NewWindow {
    /// A file of the page: loaded in the same view.
    LoadHere,
    /// Another site, after the reader acted: the reader's browser.
    OpenOutside,
    /// Nothing opens.
    Nothing,
}

pub(crate) fn new_window(url: &str, own_host: &str, reader: bool) -> NewWindow {
    if scheme_of(url) == SCHEME {
        return match crate::http::split_url(url) {
            Some((host, _)) if host == own_host.to_ascii_lowercase() => NewWindow::LoadHere,
            _ => NewWindow::Nothing,
        };
    }
    if opens_outside(url) && reader {
        NewWindow::OpenOutside
    } else {
        NewWindow::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn navigation(url: &str) -> Navigation<'_> {
        Navigation {
            url,
            own_host: "page-12",
            main_frame: true,
            link: false,
            download: false,
            reader: false,
        }
    }

    #[test]
    fn the_gate_opens_for_one_second_after_the_reader() {
        assert!(!reader_acted(None, 100.0, None));
        assert!(reader_acted(Some(99.5), 100.0, None));
        assert!(reader_acted(Some(99.0), 100.0, None));
        assert!(!reader_acted(Some(98.9), 100.0, None));
        assert!(reader_acted(Some(100.1), 100.0, None));
        assert!(!reader_acted(Some(101.0), 100.0, None));
        // WebKit's word is asked where it gives one, and can only close the gate.
        assert!(reader_acted(Some(99.9), 100.0, Some(true)));
        assert!(!reader_acted(Some(99.9), 100.0, Some(false)));
        assert!(!reader_acted(None, 100.0, Some(true)));
    }

    #[test]
    fn the_page_goes_only_to_its_own_files() {
        assert_eq!(
            decide(&navigation("writer-page://page-12/chapter.html")),
            Decision::Allow
        );
        assert_eq!(
            decide(&navigation("writer-page://PAGE-12/index.html#top")),
            Decision::Allow
        );
        // Another page's files are another origin.
        assert_eq!(
            decide(&navigation("writer-page://page-13/index.html")),
            Decision::Cancel
        );
        assert_eq!(decide(&navigation("about:blank")), Decision::Allow);
        assert_eq!(decide(&navigation("about:srcdoc")), Decision::Allow);
        assert_eq!(decide(&navigation("about:config")), Decision::Cancel);
        assert_eq!(
            decide(&navigation("https://example.com/")),
            Decision::Cancel
        );
        assert_eq!(decide(&navigation("file:///etc/hosts")), Decision::Cancel);
        assert_eq!(decide(&navigation("ftp://example.com/a")), Decision::Cancel);
        assert_eq!(decide(&navigation("tel:+123")), Decision::Cancel);
        // A data: or blob: document in a frame reaches nothing; as the page itself it
        // would leave its own files.
        assert_eq!(decide(&navigation("data:text/html,x")), Decision::Cancel);
        let frame = Navigation {
            main_frame: false,
            ..navigation("data:text/html,x")
        };
        assert_eq!(decide(&frame), Decision::Allow);
        let frame = Navigation {
            main_frame: false,
            ..navigation("blob:writer-page://page-12/5d1e")
        };
        assert_eq!(decide(&frame), Decision::Allow);
    }

    #[test]
    fn a_link_opens_outside_only_when_the_reader_opened_it() {
        let link = Navigation {
            link: true,
            ..navigation("https://example.com/")
        };
        assert_eq!(decide(&link), Decision::Cancel);
        assert_eq!(
            decide(&Navigation {
                reader: true,
                ..link
            }),
            Decision::OpenOutside
        );
        assert_eq!(
            decide(&Navigation {
                reader: true,
                link: true,
                ..navigation("mailto:someone@example.com")
            }),
            Decision::OpenOutside
        );
        // The reader acted, but it is no link: a script's `location = ...`.
        assert_eq!(
            decide(&Navigation {
                reader: true,
                ..navigation("https://example.com/")
            }),
            Decision::Cancel
        );
        // A link to another kind of address is never opened.
        assert_eq!(
            decide(&Navigation {
                reader: true,
                link: true,
                ..navigation("file:///Users")
            }),
            Decision::Cancel
        );
        // A download is cancelled whatever it is.
        assert_eq!(
            decide(&Navigation {
                download: true,
                ..navigation("writer-page://page-12/data.csv")
            }),
            Decision::Cancel
        );
    }

    #[test]
    fn a_new_window_opens_here_outside_or_not_at_all() {
        assert_eq!(
            new_window("writer-page://page-12/b.html", "page-12", false),
            NewWindow::LoadHere
        );
        assert_eq!(
            new_window("writer-page://page-9/b.html", "page-12", true),
            NewWindow::Nothing
        );
        assert_eq!(
            new_window("https://example.com", "page-12", false),
            NewWindow::Nothing
        );
        assert_eq!(
            new_window("https://example.com", "page-12", true),
            NewWindow::OpenOutside
        );
        assert_eq!(
            new_window("javascript:alert(1)", "page-12", true),
            NewWindow::Nothing
        );
    }
}
