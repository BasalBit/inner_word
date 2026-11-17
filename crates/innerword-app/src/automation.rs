//! Debug automation hook: drive the running app from a tiny script.
//!
//! When `INNER_WORD_AUTOMATION` is set, the app runs the script against its
//! main window after startup, dispatching keystrokes and text through GPUI
//! exactly as if they were typed (bindings first, then the focused input
//! handler). Progress and errors go to stderr, prefixed with `[automation]`.
//!
//! # Grammar
//!
//! ```text
//! script  := step ((';' | newline) step)*
//! step    := name [':' argument]          leading/trailing whitespace ignored
//! comment := '#' ... end of line          only at the start of a line
//! ```
//!
//! | step                  | effect                                                       |
//! |-----------------------|--------------------------------------------------------------|
//! | `wait:<ms>`           | sleep                                                        |
//! | `keys:<keystrokes>`   | GPUI keystrokes separated by spaces: `keys:cmd-shift-p escape`; the key `;` is written `\;` (`keys:cmd-\;`) |
//! | `type:<text>`         | types each character (escapes: `\;` `\n` `\t` `\s` `\\`)     |
//! | `action:<name>`       | dispatches a registered action by name: `action:app::Quit`   |
//! | `menu:<name>`         | opens a menu of the workspace at its control: `title`, `save`, `statistics`, `page`, `limit`, `zoom` |
//! | `choose:<label>`      | chooses the item with that label in the open context menu, as a click on it would |
//! | `menukey:<title>`     | prints the key the native menu bar has beside the item with that title (`menukey:Bold`), read from AppKit's own menu |
//! | `bar[:<button>]`      | shows the format bar over the selection, as a selection made with the mouse does; with a button (`bold`, `italic`, `highlight`, `turn-into`, `more`, `extra-0` ...) presses that button of the bar |
//! | `shortcut:<action>[@<place>]=<keys>` | gives a command its keys without the list: `shortcut:editor::ToggleBold=cmd-alt-b`; several keys are separated by a bar (`cmd-alt-b\|f5`), `none` leaves it without a key, `default` puts it back; `@In the find bar` names the place for a command that is listed in several |
//! | `shortcuts:<what>`    | the keyboard shortcuts: `shortcuts:import:<path>` reads a shortcuts file and shows what it would change in the list (as choosing it in the open panel does), `shortcuts:press:<label>` presses a control of the open list by its label (`Import…`, `Export…`, `Reset All…`, `Changed Only`, `Close`, or a choice of its open question: `Replace My Changes`, `Use ⌥⌘B`, `Cancel`), `shortcuts:reset` puts every key back without asking, `shortcuts:undo` undoes the last import or reset |
//! | `fake:<what>`         | the fake engine of the assistant (`INNER_WORD_ASSISTANT_FAKE`, see [`crate::assistant::fake`]): `fake:hold` holds its events back, `fake:advance:<n>` lets the next `n` through (an answer half written), `fake:release` lets everything through from now on |
//! | `agents:<n>[:<ms>]`   | waits until agents have made `n` changes through this app since it opened (the `innerword` tool, run from outside while the script waits), for at most `ms` milliseconds (10000 unless given); fails when they did not come |
//! | `agents:on`, `agents:off` | sets the switch "Agents can make changes while Inner Word is open", as its button in the Agents panel does |
//! | `zoom:<what>`         | the zoom of the text: `in`, `out`, `actual` (100 %), `fit` (the width), `page` (a whole page), `life` (the size of the paper), `auto` (what a page opens at, "Automatic"; page view only), a percentage (`zoom:125`), `by:<factor>` (what a pinch at the middle of the text does), `pinch:<magnification>` (hands the monitor AppKit calls a stand-in for a magnify event there) and `sideways:<pixels>` (scrolls a sheet that is wider than its room); prints the zoom afterwards |
//! | `pagefit:<font>\|<size>\|<line spacing>\|<a4 or letter>\|<file>` | lays the text of the file out as one paragraph in page view at those settings (2.54 cm margins) and prints what the page holds: the rows of the paragraph, the characters in a full row (mean, least, most), the rows on the first page |
//! | `print:pdf:<path>`    | makes the exact PDF "Print…" would hand to the print panel (the open document laid out as page view lays it out, whatever the window shows), writes it to the path and prints what was made: `print: 12 pages, 595.28 x 841.89 pt, 5214 rows, 0 rows off, 3 images, fonts: .SFNS-Regular, Charter-Bold, Charter-Roman, made in 0.04 s`, and a second line `print: missing fonts: …` when the document asks for a font this Mac does not have. `rows off` counts the rows the PDF sets in another face than the layout measured: 0 when the paper shows what page view shows |
//! | `print:job:<path>`    | the same PDF through the real macOS print operation as a save job, both panels off: what "Save as PDF" in the print panel would write. Everything of "Print…" but the panel. Prints `print: job saved to <path>` or why it was not |
//! | `export:wait[:<ms>]`  | waits until the PDF export that was chosen (the picker's "PDF file", `action:workspace::ExportPdf`) has written its file into `INNER_WORD_EXPORT_DIR`, for at most `ms` milliseconds (60000 unless given), and prints what was made: `export: “Novel” written to <path>: 31 pages, 595.28 x 841.89 pt, 1204 rows, 0 rows off, 0 images, fonts: …, made in 0.21 s`; fails with the reason when nothing was written (cancelled, another job under way) or none ended in time. Each export is told once |
//! | `image:<path>`        | inserts the image file at the caret, as "Insert > Image…" does once the file is chosen in the open panel (which a scripted run never opens); waits until the picture is stored and in the text and prints `image: inserted <id> 1600x1067 at block 3`, or the sentence that says why the file could not be used |
//! | `image:select:<n>`    | puts the caret on the n-th image of the document (from 1): the image is selected |
//! | `page:make:<folder>[\|<entry>]` | stores a folder (or file) that is offline already as a page in the folder selected in the library, as it is (nothing fetched, no WebKit check: QA only), and opens it |
//! | `page:import:<path>`  | adds a file or folder as a page, as File > Import Page… does once it is chosen (what it names on the web fetched, the check), in the folder selected in the library; waits until it is done and prints a line per page (`page: added page:12 “Orbitarium” 31 files 1.0 MB`, its warnings under it, or `page: not added: <the sentence>`) and the summary |
//! | `page:wait[:<ms>]`    | waits until the open page reports that it loaded (fails after `ms`, 10 s unless given) |
//! | `page:text`           | prints the open page's text as the check read it, its length first |
//! | `page:find:<query>`   | opens the find strip of the open page with that query |
//! | `page:js:<script>`    | evaluates a script in the open page and prints the answer |
//! | `page:menu`           | opens the page's context menu, as `ctrl-enter` does |
//! | `resize:<w>x<h>`      | resizes the window content                                   |
//! | `theme:<mode>`        | `light`, `dark` or `system`                                  |
//! | `screenshot:<path>`   | captures only this window to a PNG (waits for a fresh frame) |
//! | `log:<text>`          | prints the text                                              |
//! | `info`                | prints window bounds, scale factor, appearance, theme, focus |
//! | `state`               | prints the workspace state: document, save state, modal, open context menu, focus, printing (`print=[idle]`, `print=[preparing 40%]` while the pages of a print are made, `print=[panel]` while the job is in the print panel), the PDF export (`export=[idle]`, `export=[preparing 40%]` while its pages are made, `export=[writing]` while the file is written), images (`images=N`: how many the document holds, and while the caret is on one `image=[block=3 width=50% natural=1600x1067@144 align=center ready]`: its block counted from 0, its width as a share of the text column, its pixel size and the density its file declares, its alignment, and `ready`, `loading`, `missing` or `broken` for its picture, then `alt="..."` when it has a description), spelling (on or off, the language and whose it is, marked words, blocks still to check), the zoom of the view (`zoom=[page 177% fit-width life=177%]`: the view, the percentage it is drawn at, `fit-width`, `fit-page` or `fit-width-up-to-N` when it follows the window, `sideways=<offset>/<most>` while a sheet is wider than its room, life size on this display), the keyboard shortcuts (how many commands have other keys than their defaults and, while the list is open, what it shows: its mode, the selected row with its keys, the open question with its choices), the open page (`web=[id=12 state=loaded zoom=100% overlay=none\|waiting\|snapshot\|holes:N focus=page\|find\|elsewhere responder=web\|gpui\|none find="q":found missing=0]`, or `web=[none]`), the pages being added (`adding=[2/3 "Orbitarium" fetching]` with `fetching`, `checking` or `storing`, or `adding=[idle]`), agents that write while the app is open (`live=[channel=open\|closed switch=on\|off waiting=N changes=N last="..." run=N offers=Undo\|Open\|nothing cost=[save .. ms, change .. ms, window .. ms]]`: whether the channel is open, the owner's switch, requests that wait, how many changed something, what the note about the last of them says, how many it counts, what its button offers, and what the last change cost on the main thread: saving the open document first, the change in the library, bringing the window up to date), status bar |
//! | `dump`                | prints the start of the text in the editor                   |
//! | `dump:<path>`         | writes the whole text in the editor to a file, its blocks one to a line as the document model joins them: what the writer has on screen, to hold against what the library has |
//! | `book:<blocks>`       | creates a generated document of that many blocks and opens it |
//! | `savecost[:<rounds>]` | times each part of a save of the open document on the main thread |
//! | `limitcost[:<rounds>]` | times the character-limit count: whole text against block by block |
//! | `keyscost[:<rounds>]` | times a change of a keyboard shortcut (the key of Bold, there and back): the whole change as the list makes it (install, menu bar, store), the frame after it, and the parts of installing the keymap |
//! | `typecost:<text>`     | types the text at a writer's pace (a key every 80 ms) and times every key: its dispatch and the frame after it, and how long the main thread kept a 4 ms timer waiting between the keys |
//! | `keycost:<keystrokes>` | the same for keystrokes (`keycost:pagedown pagedown`)       |
//! | `stall:<ms>`          | watches the main thread for that long without pressing anything (after `book:`, during the background pass of the spell checker, while events are posted from outside): how long a 4 ms timer was kept waiting, and what the frames cost |
//! | `quit`                | quits the app (runs the quit hooks)                          |
//!
//! `INNER_WORD_AUTOMATION=@/path/to/script.txt` reads the script from a file.
//! A step that fails is reported and the script continues; `quit` still runs.
//!
//! A scripted run stays in the background: the app does not activate
//! itself, so it never takes the keyboard from the person at the machine
//! (whose typing would otherwise land in the window under test). The window
//! is still on screen and is captured as it is, with the traffic lights of
//! an inactive window. Set `INNER_WORD_AUTOMATION_FOREGROUND=1` for a normal
//! activation.
//!
//! `INNER_WORD_SEED_DEMO=1` (see [`crate::welcome`]) fills an empty library
//! with a few folders and documents before the window opens.
//!
//! `INNER_WORD_EXPORT_DIR=<dir>`: file exports of a scripted run are written
//! there under the suggested name, without the native save panel (which a
//! script cannot drive).
//!
//! No scripted run opens the print panel. The command itself
//! (`action:workspace::Print`) writes "<title>.pdf" into
//! `INNER_WORD_EXPORT_DIR`, or says in a toast that a scripted run has no
//! panel; `action:workspace::CancelPrint` right after it, on a book, stops
//! the pages being made (`state` then has `print=[idle]` and nothing was
//! written).
//!
//! A PDF export (`keys:cmd-shift-s; type:pdf; keys:enter`, or
//! `action:workspace::ExportPdf`) writes "<title>.pdf" into
//! `INNER_WORD_EXPORT_DIR` without the save panel; its pages are made in
//! slices as a print's are (`export=[preparing 40%]`), so `export:wait`
//! waits for the file. `action:workspace::CancelPdfExport` stops it.
//!
//! While the window is completely covered by other windows macOS stops
//! GPUI's display link and nothing is drawn. The script therefore draws a
//! frame itself after every keystroke and before every capture
//! ([`crate::screenshot::force_frame`]), so a covered window behaves and
//! looks as a visible one.
//!
//! There are no mouse steps: `Window::dispatch_event` is public in gpui 0.2.2
//! but returns a crate-private type, so it cannot be called from outside gpui.
//! Context menus open from the keyboard (`keys:ctrl-enter` in the text, the
//! library, the comment list or a text field); the menus that belong to the
//! title and the status bar, which the keyboard does not reach, open with
//! `menu:`. `choose:` picks an item of whichever menu is open. The format
//! bar, which only a selection made with the mouse brings up, is shown with
//! `bar`.
//!
//! Example:
//!
//! ```sh
//! INNER_WORD_DATA_DIR=$(mktemp -d) \
//! INNER_WORD_AUTOMATION='wait:800; type:Hello world; keys:cmd-a; screenshot:/tmp/tw.png; quit' \
//! cargo run -p innerword-app
//! ```

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers, px, size};
use innerword_ui::theme::{self, ActiveTheme, ThemeMode};

use crate::{screenshot, workspace::Workspace};

/// How long the step `image:<path>` waits for the picture to be stored and
/// to arrive in the text (a photograph of a hundred megabytes is converted
/// on the way).
const IMAGE_PATIENCE: Duration = Duration::from_secs(60);

/// Environment variable holding the script (or `@path`).
pub const AUTOMATION_ENV: &str = "INNER_WORD_AUTOMATION";

/// Environment variable: when set to a non-empty value, a scripted run
/// brings the app to the front like a normal launch. By default it does
/// not, so the script never takes keyboard focus from the person at the
/// machine (their typing would otherwise land in the app under test).
pub const FOREGROUND_ENV: &str = "INNER_WORD_AUTOMATION_FOREGROUND";

/// Environment variable: a directory that file exports of a scripted run
/// are written to, under the suggested file name, instead of asking in the
/// native save panel (which a script cannot drive). Only read while
/// [`AUTOMATION_ENV`] is set.
pub const EXPORT_DIR_ENV: &str = "INNER_WORD_EXPORT_DIR";

/// The directory of [`EXPORT_DIR_ENV`], in a scripted run.
pub fn export_directory() -> Option<PathBuf> {
    std::env::var_os(AUTOMATION_ENV).filter(|script| !script.is_empty())?;
    std::env::var_os(EXPORT_DIR_ENV)
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
}

/// Whether [`FOREGROUND_ENV`] asks for the app to come to the front.
pub fn foreground_requested() -> bool {
    std::env::var_os(FOREGROUND_ENV).is_some_and(|value| !value.is_empty())
}

/// One parsed step.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(Duration),
    Keys(Vec<Keystroke>),
    Type(String),
    Action(String),
    Menu(crate::workspace::MenuKind),
    Choose(String),
    MenuKey(String),
    Bar(Option<String>),
    /// Wait for this many changes by agents, at most this long.
    Agents(usize, Duration),
    /// Set the switch "Agents can make changes while Inner Word is open".
    AgentsSwitch(bool),
    Shortcut {
        /// The name of the command's action.
        action: String,
        /// The place the command is listed in, where it has several.
        place: Option<String>,
        /// The keys it gets; `None` for its defaults.
        keys: Option<Vec<innerword_keymap::KeySequence>>,
    },
    Shortcuts(ShortcutsStep),
    Fake(FakeStep),
    Resize(f32, f32),
    Theme(ThemeMode),
    Screenshot(PathBuf),
    Log(String),
    Info,
    State,
    Dump,
    Text(PathBuf),
    Book(usize),
    SaveCost(usize),
    LimitCost(usize),
    KeysCost(usize),
    TypeCost(String),
    KeyCost(Vec<Keystroke>),
    Stall(Duration),
    /// What the zoom of the text is to become (`Workspace::zoom_step`).
    Zoom(String),
    /// Lay a text out on a page and say what the page holds.
    PageFit(crate::workspace::PageFit),
    Print(PrintStep),
    /// Wait until the PDF export under way (or the one that ended last) is
    /// over, at most this long, and say what came of it (`export:wait`).
    ExportWait(Duration),
    Image(ImageStep),
    Page(PageStep),
    Quit,
}

/// What a `page:` step does.
#[derive(Clone, Debug, PartialEq)]
pub enum PageStep {
    /// Store this folder (or file), offline already, as a page in the
    /// folder selected in the library, and open it (`page:make:<path>[|<entry>]`).
    Make(PathBuf, Option<String>),
    /// Add this file or folder as a page, as File > Import Page… does once
    /// it is chosen (fetching what it names on the web, the check), in the
    /// folder selected in the library; wait until it is done and log one
    /// line per page (`page:import:<path>`).
    Import(PathBuf),
    /// Wait until the open page reports that it loaded, at most this long.
    Wait(Duration),
    /// Log the open page's text, its length first.
    Text,
    /// Open the find strip with this query.
    Find(String),
    /// Evaluate this script in the open page and log the answer.
    Js(String),
    /// Open the page's context menu, as `ctrl-enter` does.
    Menu,
}

/// How long `page:wait` waits unless the step says otherwise.
pub const PAGE_WAIT: Duration = Duration::from_secs(10);

/// How long `export:wait` waits unless the step says otherwise: a book.
pub const EXPORT_WAIT: Duration = Duration::from_secs(60);

/// What an `image:` step does.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageStep {
    /// Insert this file at the caret.
    Insert(PathBuf),
    /// Put the caret on the n-th image of the document, counted from 1.
    Select(usize),
}

/// What a `print:` step does with the pages of the open document.
#[derive(Clone, Debug, PartialEq)]
pub enum PrintStep {
    /// Write the PDF "Print…" would hand over to this file.
    Pdf(PathBuf),
    /// Run the PDF through the print operation as a save job into this
    /// file.
    Job(PathBuf),
}

/// What a `shortcuts:` step does.
#[derive(Clone, Debug, PartialEq)]
pub enum ShortcutsStep {
    /// Read a shortcuts file and show what importing it would change.
    Import(PathBuf),
    /// Press a control of the open list by its label.
    Press(String),
    /// Put every key back to its default, without asking.
    Reset,
    /// Undo the last import or reset.
    Undo,
}

/// What a `fake:` step does to the fake engine of the assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeStep {
    /// Hold every event back.
    Hold,
    /// Let the next events through.
    Advance(usize),
    /// Stop holding.
    Release,
}

/// A parsed script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub steps: Vec<Step>,
}

impl Script {
    /// Reads `INNER_WORD_AUTOMATION`. `Ok(None)` when unset or empty.
    pub fn from_env() -> Result<Option<Self>> {
        let Some(raw) = std::env::var_os(AUTOMATION_ENV) else {
            return Ok(None);
        };
        let raw = raw
            .into_string()
            .map_err(|_| anyhow!("{AUTOMATION_ENV} is not valid UTF-8"))?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let source = match raw.trim().strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path)
                .with_context(|| format!("reading automation script {path}"))?,
            None => raw,
        };
        Self::parse(&source).map(Some)
    }

    /// Parses the grammar described in the module docs.
    pub fn parse(source: &str) -> Result<Self> {
        let mut steps = Vec::new();
        for (ix, raw_step) in split_steps(source).into_iter().enumerate() {
            let step = raw_step.trim();
            if step.is_empty() {
                continue;
            }
            let (name, arg) = match step.split_once(':') {
                Some((name, arg)) => (name.trim(), arg.trim()),
                None => (step, ""),
            };
            let parsed = parse_step(name, arg)
                .with_context(|| format!("automation step {} ({step:?})", ix + 1))?;
            steps.push(parsed);
        }
        Ok(Self { steps })
    }
}

/// Splits on unescaped `;` and newlines, drops `#` comment lines, and keeps
/// escape sequences for [`unescape`].
fn split_steps(source: &str) -> Vec<String> {
    let mut steps = Vec::new();
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut current = String::new();
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    current.push('\\');
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ';' => steps.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            }
        }
        steps.push(current);
    }
    steps
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('s') => out.push(' '),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn parse_step(name: &str, arg: &str) -> Result<Step> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "wait" | "sleep" => Step::Wait(Duration::from_millis(
            arg.parse().context("expected milliseconds")?,
        )),
        "keys" | "key" => {
            if arg.is_empty() {
                bail!("expected keystrokes");
            }
            // A key that is the step separator is written escaped:
            // `keys:cmd-\;` is the next misspelling.
            let keys = arg
                .split_whitespace()
                .map(|key| Keystroke::parse(&unescape(key)).map_err(|err| anyhow!("{err}")))
                .collect::<Result<Vec<_>>>()?;
            Step::Keys(keys)
        }
        "type" | "text" => Step::Type(unescape(arg)),
        "action" => {
            if arg.is_empty() {
                bail!("expected an action name");
            }
            Step::Action(arg.to_string())
        }
        "menu" => Step::Menu(
            crate::workspace::MenuKind::parse(arg)
                .ok_or_else(|| anyhow!("expected one of: title, save, statistics, page, limit"))?,
        ),
        "choose" => {
            if arg.is_empty() {
                bail!("expected the label of a menu item");
            }
            Step::Choose(unescape(arg))
        }
        "menukey" => {
            if arg.is_empty() {
                bail!("expected the title of a menu bar item");
            }
            Step::MenuKey(unescape(arg))
        }
        "bar" => Step::Bar((!arg.is_empty()).then(|| arg.to_string())),
        "agents" if arg.trim() == "on" => Step::AgentsSwitch(true),
        "agents" if arg.trim() == "off" => Step::AgentsSwitch(false),
        "agents" => {
            let (count, patience) = match arg.split_once(':') {
                Some((count, patience)) => (
                    count,
                    Duration::from_millis(
                        patience.trim().parse().context("expected milliseconds")?,
                    ),
                ),
                None => (arg, Duration::from_secs(10)),
            };
            Step::Agents(
                count
                    .trim()
                    .parse()
                    .context("expected a number of changes")?,
                patience,
            )
        }
        "shortcut" => {
            let (command, keys) = arg
                .split_once('=')
                .ok_or_else(|| anyhow!("expected <action>=<keys>"))?;
            let (action, place) = match command.split_once('@') {
                Some((action, place)) => (action.trim(), Some(place.trim().to_string())),
                None => (command.trim(), None),
            };
            if action.is_empty() {
                bail!("expected the name of an action");
            }
            let keys = unescape(keys.trim());
            let keys = match keys.as_str() {
                "default" => None,
                "none" | "" => Some(Vec::new()),
                keys => Some(
                    keys.split('|')
                        .map(|key| {
                            innerword_keymap::KeySequence::parse(key.trim())
                                .map_err(|error| anyhow!("{error}"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            };
            Step::Shortcut {
                action: action.to_string(),
                place,
                keys,
            }
        }
        "shortcuts" => {
            let (what, rest) = arg.split_once(':').unwrap_or((arg, ""));
            Step::Shortcuts(match what.trim() {
                "import" => {
                    if rest.trim().is_empty() {
                        bail!("expected shortcuts:import:<path>");
                    }
                    ShortcutsStep::Import(PathBuf::from(unescape(rest.trim())))
                }
                "press" => {
                    if rest.trim().is_empty() {
                        bail!("expected shortcuts:press:<label>");
                    }
                    ShortcutsStep::Press(unescape(rest.trim()))
                }
                "reset" => ShortcutsStep::Reset,
                "undo" => ShortcutsStep::Undo,
                other => bail!(
                    "unknown shortcuts step {other:?} (import:<path>, press:<label>, reset, undo)"
                ),
            })
        }
        "fake" => {
            let (what, count) = arg.split_once(':').unwrap_or((arg, ""));
            Step::Fake(match what.trim() {
                "hold" => FakeStep::Hold,
                "release" => FakeStep::Release,
                "advance" => FakeStep::Advance(
                    count
                        .trim()
                        .parse()
                        .context("expected fake:advance:<events>")?,
                ),
                other => bail!("unknown fake step {other:?} (hold, advance:<n>, release)"),
            })
        }
        "resize" => {
            let (w, h) = arg
                .split_once(['x', 'X'])
                .ok_or_else(|| anyhow!("expected <width>x<height>"))?;
            Step::Resize(w.trim().parse()?, h.trim().parse()?)
        }
        "theme" => Step::Theme(
            ThemeMode::parse(arg).ok_or_else(|| anyhow!("expected light, dark or system"))?,
        ),
        "screenshot" | "capture" => {
            if arg.is_empty() {
                bail!("expected a file path");
            }
            Step::Screenshot(PathBuf::from(unescape(arg)))
        }
        "log" | "echo" => Step::Log(unescape(arg)),
        "info" => Step::Info,
        "state" => Step::State,
        "dump" => {
            if arg.is_empty() {
                Step::Dump
            } else {
                Step::Text(PathBuf::from(unescape(arg)))
            }
        }
        "book" => Step::Book(arg.parse().context("expected a number of blocks")?),
        "savecost" => Step::SaveCost(if arg.is_empty() {
            5
        } else {
            arg.parse().context("expected a number of rounds")?
        }),
        "limitcost" => Step::LimitCost(if arg.is_empty() {
            5
        } else {
            arg.parse().context("expected a number of rounds")?
        }),
        "keyscost" => Step::KeysCost(if arg.is_empty() {
            7
        } else {
            arg.parse().context("expected a number of rounds")?
        }),
        "typecost" => {
            let text = unescape(arg);
            if text.is_empty() {
                bail!("expected text to type");
            }
            Step::TypeCost(text)
        }
        "keycost" => {
            if arg.is_empty() {
                bail!("expected keystrokes");
            }
            let keys = arg
                .split_whitespace()
                .map(|key| Keystroke::parse(&unescape(key)).map_err(|err| anyhow!("{err}")))
                .collect::<Result<Vec<_>>>()?;
            Step::KeyCost(keys)
        }
        "pagefit" => Step::PageFit(crate::workspace::PageFit::parse(arg)?),
        "print" => {
            let (what, path) = arg
                .split_once(':')
                .map(|(what, path)| (what.trim(), unescape(path.trim())))
                .unwrap_or((arg, String::new()));
            if path.is_empty() {
                bail!("expected pdf:<path> or job:<path>");
            }
            match what {
                "pdf" => Step::Print(PrintStep::Pdf(PathBuf::from(path))),
                "job" => Step::Print(PrintStep::Job(PathBuf::from(path))),
                other => bail!("unknown print step {other:?}: expected pdf:<path> or job:<path>"),
            }
        }
        "export" => {
            let (what, rest) = arg.split_once(':').unwrap_or((arg, ""));
            if what.trim() != "wait" {
                bail!("unknown export step {what:?}: expected wait[:<ms>]");
            }
            Step::ExportWait(if rest.trim().is_empty() {
                EXPORT_WAIT
            } else {
                Duration::from_millis(rest.trim().parse().context("expected milliseconds")?)
            })
        }
        "image" => {
            if let Some(number) = arg.strip_prefix("select:") {
                let number: usize = number
                    .trim()
                    .parse()
                    .context("expected image:select:<n>, counted from 1")?;
                if number == 0 {
                    bail!("images are counted from 1");
                }
                Step::Image(ImageStep::Select(number))
            } else if arg.is_empty() {
                bail!("expected the path of an image file, or select:<n>");
            } else {
                Step::Image(ImageStep::Insert(PathBuf::from(unescape(arg))))
            }
        }
        "page" => Step::Page(parse_page_step(arg)?),
        "zoom" => {
            if arg.is_empty() {
                bail!("expected a zoom: in, out, actual, fit, page, life, auto, a percentage");
            }
            Step::Zoom(arg.to_string())
        }
        "stall" => Step::Stall(Duration::from_millis(
            arg.parse().context("expected milliseconds")?,
        )),
        "quit" | "exit" => Step::Quit,
        other => bail!("unknown step {other:?}"),
    })
}

/// The steps of `page:` (`make:<path>[|<entry>]`, `wait[:<ms>]`, `text`,
/// `find:<query>`, `js:<script>`, `menu`).
fn parse_page_step(arg: &str) -> Result<PageStep> {
    let (what, rest) = arg.split_once(':').unwrap_or((arg, ""));
    Ok(match what.trim() {
        "make" => {
            let rest = unescape(rest.trim());
            let (path, entry) = match rest.split_once('|') {
                Some((path, entry)) => (path.trim().to_string(), Some(entry.trim().to_string())),
                None => (rest.clone(), None),
            };
            if path.is_empty() {
                bail!("expected page:make:<folder>[|<entry>]");
            }
            PageStep::Make(PathBuf::from(path), entry.filter(|entry| !entry.is_empty()))
        }
        "import" => {
            let path = unescape(rest.trim());
            if path.is_empty() {
                bail!("expected page:import:<file or folder>");
            }
            PageStep::Import(PathBuf::from(path))
        }
        "wait" => PageStep::Wait(if rest.trim().is_empty() {
            PAGE_WAIT
        } else {
            Duration::from_millis(rest.trim().parse().context("expected milliseconds")?)
        }),
        "text" => PageStep::Text,
        "find" => PageStep::Find(unescape(rest)),
        "js" => {
            if rest.is_empty() {
                bail!("expected page:js:<script>");
            }
            PageStep::Js(unescape(rest))
        }
        "menu" => PageStep::Menu,
        other => bail!(
            "unknown page step {other:?}: expected make:<folder>, import:<path>, wait, text, \
             find:<query>, js:<script> or menu"
        ),
    })
}

/// The keystroke GPUI would see when typing `ch` on a US layout.
pub fn keystroke_for_char(ch: char) -> Keystroke {
    match ch {
        '\n' | '\r' => Keystroke::parse("enter").expect("valid keystroke"),
        '\t' => Keystroke::parse("tab").expect("valid keystroke"),
        ' ' => Keystroke::parse("space").expect("valid keystroke"),
        _ => {
            let shift = ch.is_uppercase();
            Keystroke {
                modifiers: Modifiers {
                    shift,
                    ..Default::default()
                },
                key: ch.to_lowercase().collect(),
                key_char: Some(ch.to_string()),
            }
        }
    }
}

/// Starts running `script` against `window`. Returns immediately.
pub fn run(script: Script, window: AnyWindowHandle, cx: &mut App) {
    eprintln!("[automation] running {} steps", script.steps.len());
    cx.spawn(async move |cx| {
        // The view to draw through while the window is covered (see
        // `screenshot::force_frame`).
        let view = window
            .update(cx, |_, window, _| screenshot::native_view(window).ok())
            .ok()
            .flatten();
        for (ix, step) in script.steps.into_iter().enumerate() {
            let label = format!("{step:?}");
            match execute(step, window, view, cx).await {
                Ok(StepOutcome::Continue) => {}
                Ok(StepOutcome::Quit) => {
                    eprintln!("[automation] step {}: quit", ix + 1);
                    cx.update(|cx| cx.quit()).ok();
                    return;
                }
                Err(err) => eprintln!("[automation] step {} {label} failed: {err:#}", ix + 1),
            }
        }
        eprintln!("[automation] done");
    })
    .detach();
}

enum StepOutcome {
    Continue,
    Quit,
}

async fn execute(
    step: Step,
    window: AnyWindowHandle,
    view: Option<screenshot::NativeView>,
    cx: &mut AsyncApp,
) -> Result<StepOutcome> {
    match step {
        Step::Wait(duration) => cx.background_executor().timer(duration).await,
        Step::Keys(keys) => {
            for keystroke in keys {
                let text = keystroke.unparse();
                let handled =
                    window.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx))?;
                eprintln!(
                    "[automation] keys {text}: {}",
                    if handled { "handled" } else { "not handled" }
                );
                // A key a script presses comes without a release: the
                // list of shortcuts would take the same key pressed again
                // for a held one.
                release_key_in_shortcuts_list(window, cx);
                frame_pause(window, view, cx).await;
            }
        }
        Step::Type(text) => {
            // One update per character, like real typing: effects (notify,
            // redraw) are flushed between keystrokes, and a frame is drawn
            // after each, as it is between the key presses of a person.
            for ch in text.chars() {
                window.update(cx, |_, window, cx| {
                    window.dispatch_keystroke(keystroke_for_char(ch), cx);
                })?;
                draw_frame(window, view, cx);
            }
            eprintln!("[automation] typed {} characters", text.chars().count());
            frame_pause(window, view, cx).await;
        }
        Step::Action(name) => {
            window.update(cx, |_, window, cx| -> Result<()> {
                let action = cx
                    .build_action(&name, None)
                    .map_err(|err| anyhow!("{err}"))?;
                window.dispatch_action(action, cx);
                Ok(())
            })??;
            eprintln!("[automation] dispatched {name}");
            frame_pause(window, view, cx).await;
        }
        Step::Menu(kind) => {
            let opened = workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                workspace.open_menu(kind, None, window, cx)
            })?;
            if !opened {
                bail!("the {} menu has nothing to open on", kind.name());
            }
            eprintln!("[automation] menu {} opened", kind.name());
            frame_pause(window, view, cx).await;
        }
        Step::Choose(label) => {
            let chosen = window.update(cx, |_, window, cx| {
                innerword_ui::ContextMenu::choose(&label, window, cx)
            })?;
            if !chosen {
                bail!("no open menu has an item {label:?} that can be chosen");
            }
            eprintln!("[automation] chose {label:?}");
            frame_pause(window, view, cx).await;
        }
        Step::MenuKey(title) => {
            // The menu bar is built in an effect of the change before it.
            frame_pause(window, view, cx).await;
            match cx.update(|_| crate::menus::native_key_of(&title))? {
                Some(key) if key.is_empty() => {
                    eprintln!("[automation] menu key {title:?}: none")
                }
                Some(key) => eprintln!("[automation] menu key {title:?}: {key}"),
                None => bail!("the menu bar has no item {title:?}"),
            }
        }
        Step::Bar(button) => {
            let done = workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                workspace
                    .tools()
                    .clone()
                    .update(cx, |tools, cx| match &button {
                        None => tools.show_format_bar(window, cx),
                        Some(button) => {
                            let id = format!("format-bar-{button}");
                            tools.press_format_bar_button(&id, window, cx)
                        }
                    })
            })?;
            match (done, &button) {
                (true, None) => eprintln!("[automation] format bar shown"),
                (true, Some(button)) => eprintln!("[automation] format bar: pressed {button}"),
                (false, None) => {
                    bail!("the format bar has nothing to come up over (no selection, or it is off)")
                }
                (false, Some(button)) => {
                    bail!("the format bar is not showing a button {button:?} that can be pressed")
                }
            }
            frame_pause(window, view, cx).await;
        }
        Step::Shortcut {
            action,
            place,
            keys,
        } => {
            workspace_handle(window, cx)?
                .update(cx, |workspace, window, cx| {
                    workspace.set_keys_of_command(
                        &action,
                        place.as_deref(),
                        keys.as_deref(),
                        window,
                        cx,
                    )
                })?
                .map_err(|error| anyhow!("{error}"))?;
            let now = match &keys {
                None => "its default keys".to_string(),
                Some(keys) if keys.is_empty() => "no key".to_string(),
                Some(keys) => keys
                    .iter()
                    .map(|key| key.to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
            };
            eprintln!("[automation] shortcut {action}: {now}");
            frame_pause(window, view, cx).await;
        }
        Step::Shortcuts(step) => {
            let workspace = workspace_handle(window, cx)?;
            match step {
                ShortcutsStep::Import(path) => {
                    let path = absolute(path);
                    // What the step did, and not what it was asked for: a
                    // file that is not there was not read (the app says so
                    // in a toast, as it does for whoever chose the file).
                    let there = std::fs::metadata(&path).map(|found| found.is_file());
                    workspace.update(cx, |workspace, window, cx| {
                        workspace.import_shortcuts_file(&path, window, cx)
                    })?;
                    eprintln!("[automation] {}", import_log(&path, there));
                }
                ShortcutsStep::Press(label) => {
                    let pressed = workspace.update(cx, |workspace, window, cx| {
                        workspace
                            .shortcuts_list(cx)
                            .map(|list| list.update(cx, |list, cx| list.press(&label, window, cx)))
                    })?;
                    match pressed {
                        Some(true) => eprintln!("[automation] shortcuts: pressed {label:?}"),
                        Some(false) => bail!("the list has nothing {label:?} to press now"),
                        None => bail!("the list of keyboard shortcuts is not open"),
                    }
                }
                ShortcutsStep::Reset => {
                    workspace.update(cx, |workspace, window, cx| {
                        workspace.reset_all_shortcuts_now(window, cx)
                    })?;
                    eprintln!("[automation] shortcuts: every key is the default");
                }
                ShortcutsStep::Undo => {
                    workspace.update(cx, |workspace, window, cx| {
                        workspace.undo_shortcuts_change(window, cx)
                    })?;
                    eprintln!("[automation] shortcuts: undone");
                }
            }
            frame_pause(window, view, cx).await;
        }
        Step::Fake(step) => {
            let fake = cx
                .update(|cx| crate::assistant::fake::installed(cx))?
                .ok_or_else(|| {
                    anyhow!(
                        "this run has no fake engine: set {}",
                        crate::assistant::FAKE_ENV
                    )
                })?;
            match step {
                FakeStep::Hold => {
                    fake.hold();
                    eprintln!("[automation] fake engine: holding its events");
                }
                FakeStep::Advance(count) => {
                    let delivered = fake.advance(count);
                    eprintln!(
                        "[automation] fake engine: let {delivered} of {count} events through"
                    );
                }
                FakeStep::Release => {
                    fake.release();
                    eprintln!("[automation] fake engine: released");
                }
            }
            frame_pause(window, view, cx).await;
        }
        Step::Resize(width, height) => {
            window.update(cx, |_, window, _| {
                window.resize(size(px(width), px(height)))
            })?;
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
        }
        Step::Theme(mode) => {
            cx.update(|cx| theme::set_theme_mode(mode, cx))?;
            frame_pause(window, view, cx).await;
        }
        Step::Screenshot(path) => {
            // Make sure the latest state is on screen before capturing.
            window.update(cx, |_, window, _| window.refresh())?;
            frame_pause(window, view, cx).await;
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            draw_frame(window, view, cx);
            let number = window.update(cx, |_, window, _| screenshot::window_number(window))??;
            let path = absolute(path);
            let task = cx
                .background_executor()
                .spawn(async move { screenshot::capture_window(number, &path) });
            let written = task.await?;
            eprintln!(
                "[automation] screenshot of window {number} written to {}",
                written.display()
            );
        }
        Step::Log(text) => eprintln!("[automation] {text}"),
        Step::Info => {
            let info = window.update(cx, |_, window, cx| {
                format!(
                    "window bounds {:?}, viewport {:?}, scale {}, appearance {:?}, active {}, \
                     focused {}, theme {} ({})",
                    window.bounds(),
                    window.viewport_size(),
                    window.scale_factor(),
                    window.appearance(),
                    window.is_window_active(),
                    window.focused(cx).is_some(),
                    cx.theme().name,
                    theme::theme_mode(cx).as_str(),
                )
            })?;
            eprintln!("[automation] {info}");
        }
        Step::Agents(count, patience) => {
            let workspace = workspace_handle(window, cx)?;
            let started = Instant::now();
            loop {
                let made =
                    workspace.update(cx, |workspace, _, _| workspace.agent_change_count())?;
                if made >= count {
                    break;
                }
                if started.elapsed() >= patience {
                    bail!(
                        "agents made {made} of {count} changes in {} ms",
                        patience.as_millis()
                    );
                }
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
            }
        }
        Step::ExportWait(patience) => {
            let started = Instant::now();
            let outcome = loop {
                let outcome = workspace_handle(window, cx)?
                    .update(cx, |workspace, _, _| workspace.take_pdf_export_report())?;
                if let Some(outcome) = outcome {
                    break outcome;
                }
                if started.elapsed() >= patience {
                    let doing = workspace_handle(window, cx)?
                        .update(cx, |workspace, _, _| workspace.describe_pdf_export())?;
                    bail!(
                        "export: no PDF export ended within {} ms (export=[{doing}])",
                        patience.as_millis()
                    );
                }
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
            };
            match outcome {
                Ok(line) => eprintln!("[automation] export: {line}"),
                Err(why) => bail!("export: nothing was written: {why}"),
            }
            frame_pause(window, view, cx).await;
        }
        Step::AgentsSwitch(on) => {
            workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                workspace.set_agents_may_change(on, cx);
            })?;
            eprintln!(
                "[automation] agents can {} while Inner Word is open",
                if on { "make changes" } else { "only read" }
            );
        }
        Step::State => {
            let state = workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                describe_state(workspace, window, cx)
            })?;
            eprintln!("[automation] state: {state}");
        }
        Step::Dump => {
            let text = workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                let text = workspace.editor().read(cx).document().plain_text();
                let mut shown: String = text.chars().take(TEXT_DUMP_CHARS).collect();
                if text.chars().count() > TEXT_DUMP_CHARS {
                    shown.push('…');
                }
                shown
            })?;
            eprintln!("[automation] text: {text:?}");
        }
        Step::Text(path) => {
            let text = workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                workspace.editor().read(cx).document().plain_text()
            })?;
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
            eprintln!(
                "[automation] text: {} characters written to {}",
                text.chars().count(),
                path.display()
            );
        }
        Step::Book(blocks) => {
            let started = Instant::now();
            let title = workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                workspace.open_generated_book(blocks, window, cx)
            })??;
            let opened = started.elapsed();
            draw_frame(window, view, cx);
            let drawn = started.elapsed();
            eprintln!(
                "[automation] book: opened {title:?} with {blocks} blocks in {:.1} ms, first frame \
                 after {:.1} ms",
                opened.as_secs_f64() * 1000.,
                drawn.as_secs_f64() * 1000.
            );
            // How long until the text on screen has been checked for
            // spelling: when the marks of what one sees are there.
            if visible_text_checked(window, cx).is_some() {
                let limit = Duration::from_secs(3);
                while visible_text_checked(window, cx) == Some(false) && started.elapsed() < limit {
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                }
                match visible_text_checked(window, cx) {
                    Some(true) => eprintln!(
                        "[automation] book: the text on screen was checked for spelling {:.1} ms \
                         after the document was asked for",
                        started.elapsed().as_secs_f64() * 1000.
                    ),
                    _ => eprintln!(
                        "[automation] book: the text on screen was not checked for spelling \
                         within {} s",
                        limit.as_secs()
                    ),
                }
                draw_frame(window, view, cx);
            }
            frame_pause(window, view, cx).await;
        }
        Step::SaveCost(rounds) => {
            let report = workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                workspace.measure_save_cost(rounds, cx)
            })?;
            for line in report {
                eprintln!("[automation] savecost: {line}");
            }
        }
        Step::LimitCost(rounds) => {
            let report = workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                workspace.measure_limit_cost(rounds, cx)
            })?;
            for line in report {
                eprintln!("[automation] limitcost: {line}");
            }
        }
        Step::KeysCost(rounds) => {
            for line in measure_keys_cost(rounds.max(1), window, view, cx).await? {
                eprintln!("[automation] keyscost: {line}");
            }
        }
        Step::TypeCost(text) => {
            let keys = text.chars().map(keystroke_for_char).collect();
            for line in measure_keys(keys, window, view, cx).await? {
                eprintln!("[automation] typecost: {line}");
            }
        }
        Step::KeyCost(keys) => {
            for line in measure_keys(keys, window, view, cx).await? {
                eprintln!("[automation] keycost: {line}");
            }
        }
        Step::PageFit(fit) => {
            let text = workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                workspace.open_page_fit(&fit, window, cx)
            })??;
            // The page is laid out by the frame that draws it.
            frame_pause(window, view, cx).await;
            let report = workspace_handle(window, cx)?
                .update(cx, |workspace, _, cx| workspace.report_page_fit(&text, cx))??;
            for line in report {
                eprintln!("[automation] pagefit: {line}");
            }
            frame_pause(window, view, cx).await;
        }
        Step::Print(PrintStep::Pdf(path))
            if workspace_handle(window, cx)?
                .update(cx, |workspace, _, _| workspace.open_page_id().is_some())? =>
        {
            // A page: WebKit's print of the view, from this task and
            // outside any update.
            let started = Instant::now();
            let answer = workspace_handle(window, cx)?
                .update(cx, |workspace, _, _| workspace.page_print_job())?
                .ok_or_else(|| anyhow!("no page is open"))?;
            let printed = answer
                .await
                .ok_or_else(|| anyhow!("the page was not printed"))?
                .map_err(|error| anyhow!("the page was not printed: {error}"))?;
            let path = absolute(path);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, &printed.bytes)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!(
                "[automation] print: {} pages, {:.2} x {:.2} pt (a page, WebKit), made in {:.2} s",
                printed.pages,
                printed.paper.0,
                printed.paper.1,
                started.elapsed().as_secs_f64()
            );
            eprintln!(
                "[automation] print: {} bytes written to {}",
                printed.bytes.len(),
                path.display()
            );
            frame_pause(window, view, cx).await;
        }
        Step::Print(step) => {
            let started = Instant::now();
            let job = workspace_handle(window, cx)?
                .update(cx, |workspace, window, cx| workspace.print_job(window, cx))?
                .ok_or_else(|| anyhow!("no document is open"))?;
            let made = started.elapsed();
            match step {
                PrintStep::Pdf(path) => {
                    let path = absolute(path);
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    std::fs::write(&path, job.pdf.bytes())
                        .with_context(|| format!("writing {}", path.display()))?;
                    for line in crate::printing::report_lines(&job, made) {
                        eprintln!("[automation] print: {line}");
                    }
                    eprintln!(
                        "[automation] print: {} bytes written to {}",
                        job.pdf.len(),
                        path.display()
                    );
                }
                PrintStep::Job(path) => {
                    let path = absolute(path);
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    // The print operation, from this task of the main
                    // thread and outside any update: AppKit runs it to
                    // its end before this returns.
                    match innerword_print::run_save_job(&job.request(), &path) {
                        Ok(()) => {
                            eprintln!("[automation] print: job saved to {}", path.display())
                        }
                        Err(error) => bail!("the print job was not saved: {error}"),
                    }
                }
            }
            frame_pause(window, view, cx).await;
        }
        Step::Page(step) => {
            run_page_step(step, window, cx).await?;
            frame_pause(window, view, cx).await;
        }
        Step::Image(ImageStep::Insert(path)) => {
            let path = absolute(path);
            let workspace = workspace_handle(window, cx)?;
            // What is there before: the images of the document, and the
            // notes at the bottom (a file that cannot be used says so in
            // one).
            let (images, notes) =
                workspace.update(cx, |workspace, window, cx| -> Result<_> {
                    if workspace.open_document_id().is_none() {
                        bail!("no document is open");
                    }
                    let images = workspace.editor().read(cx).document().images().count();
                    let notes = workspace.toast_layer().read(cx).ids();
                    workspace.editor().clone().update(cx, |editor, cx| {
                        editor.insert_image_files(vec![path.clone()], window, cx)
                    });
                    Ok((images, notes))
                })??;
            let started = Instant::now();
            loop {
                let said = workspace.update(cx, |workspace, _, cx| {
                    let editor = workspace.editor().read(cx);
                    if editor.document().images().count() > images {
                        return Some(match editor.selected_image() {
                            Some((block, image)) => format!(
                                "inserted {} {}x{} at block {block}",
                                image.id, image.pixels.0, image.pixels.1
                            ),
                            None => "inserted".to_string(),
                        });
                    }
                    let layer = workspace.toast_layer().read(cx);
                    layer
                        .ids()
                        .into_iter()
                        .zip(layer.toasts())
                        .find(|(id, _)| !notes.contains(id))
                        .map(|(_, toast)| toast.message.to_string())
                })?;
                if let Some(said) = said {
                    eprintln!("[automation] image: {said}");
                    break;
                }
                if started.elapsed() > IMAGE_PATIENCE {
                    bail!("the image did not arrive in the text");
                }
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
            }
            frame_pause(window, view, cx).await;
        }
        Step::Image(ImageStep::Select(number)) => {
            workspace_handle(window, cx)?.update(cx, |workspace, _, cx| -> Result<()> {
                let editor = workspace.editor().clone();
                let block = editor
                    .read(cx)
                    .document()
                    .images()
                    .nth(number - 1)
                    .map(|(block, _)| block)
                    .ok_or_else(|| anyhow!("the document has no image {number}"))?;
                editor.update(cx, |editor, cx| {
                    editor.edit(
                        |state| state.set_caret(innerword_core::Position::new(block, 0)),
                        cx,
                    );
                    editor.scroll_to_caret(innerword_editor::Autoscroll::Fit, cx);
                });
                eprintln!("[automation] image: selected image {number} (block {block})");
                Ok(())
            })??;
            frame_pause(window, view, cx).await;
        }
        Step::Zoom(what) => {
            workspace_handle(window, cx)?.update(cx, |workspace, window, cx| {
                workspace.zoom_step(&what, window, cx)
            })??;
            // What a zoom that follows the window comes to is known after
            // a frame.
            frame_pause(window, view, cx).await;
            let zoom =
                workspace_handle(window, cx)?.update(cx, |workspace, _, cx| {
                    match workspace.page_zoom() {
                        Some(percent) => format!("the page at {percent}%"),
                        None => workspace.describe_zoom(cx),
                    }
                })?;
            eprintln!("[automation] zoom {what}: {zoom}");
        }
        Step::Stall(duration) => {
            reset_spelling_cost(window, cx);
            let mut watch = MainThreadWatch::default();
            watch.run(duration, window, view, cx).await;
            for line in watch.report() {
                eprintln!("[automation] stall: {line}");
            }
            eprintln!("[automation] stall: {}", spelling_cost(window, cx));
        }
        Step::Quit => return Ok(StepOutcome::Quit),
    }
    Ok(StepOutcome::Continue)
}

/// The pause between two keys of `typecost` and `keycost`: about twelve
/// keys a second, a fast writer.
const KEY_PACE: Duration = Duration::from_millis(80);
/// The timer [`MainThreadWatch`] waits on.
const WATCH_BEAT: Duration = Duration::from_millis(4);
/// A frame is drawn at most this often while watching, as a display does.
const WATCH_FRAME: Duration = Duration::from_millis(16);

fn millis(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.)
}

/// Median, 95th percentile and maximum of `samples`.
fn spread(label: &str, samples: &mut [Duration]) -> String {
    if samples.is_empty() {
        return format!("{label}: nothing measured");
    }
    samples.sort();
    let at = |part: f64| samples[((samples.len() - 1) as f64 * part).round() as usize];
    format!(
        "{label}: median {}, p95 {}, max {} ({} samples)",
        millis(at(0.5)),
        millis(at(0.95)),
        millis(samples[samples.len() - 1]),
        samples.len()
    )
}

/// Watches the main thread from a task that runs on it: the task waits on
/// a short timer of the background executor again and again, and whatever
/// it wakes up late by is time the main thread was busy with something
/// else (a slice of the spell checker, a save, a layout). It also draws
/// the frames a display would ask for and times them.
#[derive(Default)]
struct MainThreadWatch {
    late: Vec<Duration>,
    frames: Vec<Duration>,
    last_frame: Option<Instant>,
    /// When the long waits happened, in milliseconds since the watch
    /// began, and how long they were (the first few).
    long_waits: Vec<(Duration, Duration)>,
    started: Option<Instant>,
}

impl MainThreadWatch {
    async fn run(
        &mut self,
        duration: Duration,
        window: AnyWindowHandle,
        view: Option<screenshot::NativeView>,
        cx: &mut AsyncApp,
    ) {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            let at = Instant::now();
            cx.background_executor().timer(WATCH_BEAT).await;
            let late = at.elapsed().saturating_sub(WATCH_BEAT);
            self.late.push(late);
            let started = *self.started.get_or_insert(at);
            if late > Duration::from_millis(16) && self.long_waits.len() < 12 {
                self.long_waits.push((at - started, late));
            }
            if self
                .last_frame
                .is_none_or(|last| last.elapsed() >= WATCH_FRAME)
            {
                let at = Instant::now();
                draw_frame(window, view, cx);
                self.frames.push(at.elapsed());
                self.last_frame = Some(Instant::now());
            }
        }
    }

    fn report(&mut self) -> Vec<String> {
        let over = |samples: &[Duration], limit: u64| {
            samples
                .iter()
                .filter(|sample| **sample > Duration::from_millis(limit))
                .count()
        };
        let mut report = Vec::new();
        if !self.long_waits.is_empty() {
            let waits: Vec<String> = self
                .long_waits
                .iter()
                .map(|(at, late)| {
                    format!("{} at {:.0} ms", millis(*late), at.as_secs_f64() * 1000.)
                })
                .collect();
            report.push(format!("waits over 16 ms: {}", waits.join(", ")));
        }
        report.splice(
            0..0,
            [
                format!(
                    "{}; over 8 ms: {}, over 16 ms: {}, over 50 ms: {}",
                    spread("main thread kept a 4 ms timer waiting", &mut self.late),
                    over(&self.late, 8),
                    over(&self.late, 16),
                    over(&self.late, 50),
                ),
                format!(
                    "{}; over 16 ms: {}",
                    spread("frames while watching", &mut self.frames),
                    over(&self.frames, 16),
                ),
            ],
        );
        report
    }
}

/// Presses `keys` one by one at [`KEY_PACE`] and reports what each cost.
async fn measure_keys(
    keys: Vec<Keystroke>,
    window: AnyWindowHandle,
    view: Option<screenshot::NativeView>,
    cx: &mut AsyncApp,
) -> Result<Vec<String>> {
    reset_spelling_cost(window, cx);
    let mut dispatch = Vec::new();
    let mut frame = Vec::new();
    let mut total = Vec::new();
    let mut watch = MainThreadWatch::default();
    for keystroke in keys {
        let at = Instant::now();
        window.update(cx, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        })?;
        let dispatched = at.elapsed();
        draw_frame(window, view, cx);
        let drawn = at.elapsed();
        dispatch.push(dispatched);
        frame.push(drawn - dispatched);
        total.push(drawn);
        watch.last_frame = Some(Instant::now());
        watch.run(KEY_PACE, window, view, cx).await;
    }
    let mut report = vec![
        spread("key dispatch", &mut dispatch),
        spread("frame after the key", &mut frame),
        spread("key to picture", &mut total),
    ];
    report.extend(watch.report());
    report.push(spelling_cost(window, cx));
    Ok(report)
}

/// Whether every block on screen has been checked for spelling; `None`
/// without a checker that is on, or before the text was laid out.
fn visible_text_checked(window: AnyWindowHandle, cx: &mut AsyncApp) -> Option<bool> {
    let workspace = workspace_handle(window, cx).ok()?;
    workspace
        .update(cx, |workspace, _, cx| {
            let spell = workspace.spell()?;
            if !spell.read(cx).is_active(cx) {
                return None;
            }
            let editor = workspace.editor().read(cx);
            let visible = editor.visible_range()?;
            Some(
                editor
                    .first_unchecked_spelling_block(visible.start.block..visible.end.block + 1)
                    .is_none(),
            )
        })
        .ok()
        .flatten()
}

fn reset_spelling_cost(window: AnyWindowHandle, cx: &mut AsyncApp) {
    if let Ok(workspace) = workspace_handle(window, cx) {
        workspace
            .update(cx, |workspace, _, cx| {
                if let Some(spell) = workspace.spell().cloned() {
                    spell.update(cx, |spell, _| spell.reset_slice_stats());
                }
            })
            .ok();
    }
}

/// What the spell checker cost the main thread since the last reset.
fn spelling_cost(window: AnyWindowHandle, cx: &mut AsyncApp) -> String {
    let cost = workspace_handle(window, cx).and_then(|workspace| {
        workspace.update(cx, |workspace, _, cx| {
            workspace.spell().map(|spell| {
                let spell = spell.read(cx);
                let (slices, longest) = spell.slice_stats();
                format!(
                    "spell checker: {slices} slices, longest {}, over twice the budget: {}, \
                     blocks still to check: {}, marked words: {}",
                    millis(longest),
                    spell.slow_slice_count(),
                    spell.pending_blocks(cx),
                    spell.misspelling_count(cx),
                )
            })
        })
    });
    match cost {
        Ok(Some(cost)) => cost,
        Ok(None) => "spell checker: none".to_string(),
        Err(error) => format!("spell checker: {error:#}"),
    }
}

/// What a change of a keyboard shortcut costs: the key of Bold is changed
/// and put back `rounds` times the way the list does it
/// ([`Workspace::set_keys_of_command`]: the keymap installed, the menu bar
/// built again, the change stored, the open list refreshed), with the
/// frame after each change, and then the parts of installing the keymap
/// alone ([`crate::keymap::measure_install`]). Medians and the worst.
async fn measure_keys_cost(
    rounds: usize,
    window: AnyWindowHandle,
    view: Option<screenshot::NativeView>,
    cx: &mut AsyncApp,
) -> Result<Vec<String>> {
    use std::time::Instant;
    fn ms(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1000.
    }
    fn summary(mut values: Vec<f64>) -> String {
        values.sort_by(|a, b| a.total_cmp(b));
        let median = values[values.len() / 2];
        let worst = values[values.len() - 1];
        format!("{median:.2} ms (worst {worst:.2})")
    }
    let workspace = workspace_handle(window, cx)?;
    let other = [innerword_keymap::KeySequence::parse("cmd-alt-b").map_err(|e| anyhow!("{e}"))?];
    let mut changes = Vec::new();
    let mut frames = Vec::new();
    for round in 0..rounds * 2 {
        let keys = (round % 2 == 0).then_some(other.as_slice());
        let start = Instant::now();
        workspace
            .update(cx, |workspace, window, cx| {
                workspace.set_keys_of_command("editor::ToggleBold", None, keys, window, cx)
            })?
            .map_err(|error| anyhow!("{error}"))?;
        changes.push(ms(start.elapsed()));
        let start = Instant::now();
        draw_frame(window, view, cx);
        frames.push(ms(start.elapsed()));
        cx.background_executor()
            .timer(Duration::from_millis(40))
            .await;
    }
    let mut parts: [Vec<f64>; 4] = Default::default();
    let mut bindings = 0;
    for _ in 0..rounds {
        let cost = cx
            .update(crate::keymap::measure_install)?
            .ok_or_else(|| anyhow!("the app has no keymap"))?;
        bindings = cost.bindings;
        for (part, value) in
            parts
                .iter_mut()
                .zip([cost.effective, cost.build, cost.bind, cost.menus])
        {
            part.push(ms(value));
        }
        draw_frame(window, view, cx);
        cx.background_executor()
            .timer(Duration::from_millis(40))
            .await;
    }
    let [effective, build, bind, menus] = parts;
    Ok(vec![
        format!(
            "a change of a key, {} times: {}",
            rounds * 2,
            summary(changes)
        ),
        format!("the frame after it: {}", summary(frames)),
        format!(
            "installing {bindings} bindings: what to bind {}, building them {}, clear and bind {}, the menu bar {}",
            summary(effective),
            summary(build),
            summary(bind),
            summary(menus)
        ),
    ])
}

/// Tells the open list of keyboard shortcuts that the key just pressed
/// was let go.
fn release_key_in_shortcuts_list(window: AnyWindowHandle, cx: &mut AsyncApp) {
    if let Ok(workspace) = workspace_handle(window, cx) {
        workspace
            .update(cx, |workspace, _, cx| {
                if let Some(list) = workspace.shortcuts_list(cx) {
                    list.update(cx, |list, _| list.key_released());
                }
            })
            .ok();
    }
}

/// How much of the document the `dump` step prints.
const TEXT_DUMP_CHARS: usize = 400;

fn workspace_handle(
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Result<gpui::WindowHandle<Workspace>> {
    cx.update(|_| window.downcast::<Workspace>())?
        .ok_or_else(|| anyhow!("the window does not show a workspace"))
}

/// Carries out one `page:` step.
async fn run_page_step(step: PageStep, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
    let workspace = workspace_handle(window, cx)?;
    match step {
        PageStep::Make(path, entry) => {
            let path = absolute(path);
            let made = workspace.update(cx, |workspace, window, cx| {
                let folder = workspace.panel().read(cx).selected_folder();
                let library = workspace.library().clone();
                let made = crate::pages::make_page(&library, &path, entry.as_deref(), folder, cx);
                if let Ok(meta) = &made {
                    workspace.open_document(meta.id, false, window, cx);
                }
                made
            })?;
            match made {
                Ok(meta) => {
                    let manifest = workspace
                        .update(cx, |workspace, _, _| workspace.page_manifest().cloned())?;
                    let (files, bytes) = manifest
                        .map(|manifest| (manifest.files.len(), manifest.served_bytes()))
                        .unwrap_or_default();
                    eprintln!(
                        "[automation] page: made page:{} “{}” {files} files {}",
                        meta.id.0,
                        meta.title,
                        innerword_page::megabytes(bytes)
                    );
                }
                Err(why) => bail!("page: {} was not made: {why}", path.display()),
            }
        }
        PageStep::Import(path) => {
            let path = absolute(path);
            let started = Instant::now();
            let asked = workspace.update(cx, |workspace, window, cx| {
                let folder = workspace.panel().read(cx).selected_folder();
                workspace.import_pages(vec![path.clone()], folder, window, cx)
            })?;
            let Some(asked) = asked else {
                bail!("page:import: pages cannot be added in this run");
            };
            let Some(report) = asked.await else {
                bail!("page:import: the import of {} did not end", path.display());
            };
            let library = workspace.update(cx, |workspace, _, _| workspace.library().clone())?;
            for page in &report.pages {
                let line = match &page.outcome {
                    crate::workspace::PageOutcome::Added {
                        id,
                        files,
                        bytes,
                        warnings,
                        ..
                    } => {
                        let title = library
                            .read_with(cx, |library, _| {
                                library.document_meta(*id).map(|meta| meta.title)
                            })
                            .ok()
                            .and_then(Result::ok)
                            .unwrap_or_else(|| page.name.clone());
                        let mut line = format!(
                            "added page:{} \u{201c}{title}\u{201d} {files} files {}",
                            id.0,
                            innerword_page::megabytes(*bytes)
                        );
                        for warning in warnings {
                            line.push_str(&format!("\n[automation] page:   {warning}"));
                        }
                        line
                    }
                    crate::workspace::PageOutcome::NotAdded { sentence } => {
                        format!("not added: {sentence}")
                    }
                    crate::workspace::PageOutcome::Cancelled => {
                        format!("not added \u{201c}{}\u{201d}: cancelled", page.name)
                    }
                };
                eprintln!("[automation] page: {line}");
            }
            eprintln!(
                "[automation] page: {} ({:.2} s)",
                report.summary(),
                started.elapsed().as_secs_f64()
            );
        }
        PageStep::Wait(limit) => {
            let started = Instant::now();
            let loaded = workspace.update(cx, |workspace, _, _| workspace.page_loaded())?;
            let Some(loaded) = loaded else {
                bail!("page:wait: no page is open");
            };
            let timer = cx.background_executor().timer(limit);
            let outcome = futures_select(loaded, timer).await;
            match outcome {
                Some(()) => eprintln!(
                    "[automation] page: loaded after {:.0} ms",
                    started.elapsed().as_secs_f64() * 1000.
                ),
                None => {
                    let state = workspace.update(cx, |workspace, _, _| workspace.page_state())?;
                    bail!(
                        "page:wait: the page did not load within {} ms ({state:?})",
                        limit.as_millis()
                    );
                }
            }
        }
        PageStep::Text => {
            let text = workspace.update(cx, |workspace, _, _| {
                workspace
                    .page_manifest()
                    .map(|manifest| manifest.text.clone())
            })?;
            let Some(text) = text else {
                bail!("page:text: no page is open");
            };
            let mut shown: String = text.chars().take(TEXT_DUMP_CHARS).collect();
            if text.chars().count() > TEXT_DUMP_CHARS {
                shown.push('…');
            }
            eprintln!(
                "[automation] page: text of {} characters: {shown:?}",
                text.chars().count()
            );
        }
        PageStep::Find(query) => {
            workspace.update(cx, |workspace, window, cx| {
                workspace.open_find_strip(Some(&query), window, cx)
            })?;
        }
        PageStep::Js(script) => {
            let answer =
                workspace.update(cx, |workspace, _, _| workspace.evaluate_in_page(&script))?;
            let Some(answer) = answer else {
                bail!("page:js: no page is shown");
            };
            match answer.await {
                Some(Ok(value)) => eprintln!("[automation] page: js answered {value:?}"),
                Some(Err(error)) => eprintln!("[automation] page: js failed: {error}"),
                None => eprintln!("[automation] page: js got no answer"),
            }
        }
        PageStep::Menu => {
            workspace.update(cx, |workspace, window, cx| {
                if let Some(focus) = workspace.page_focus_handle() {
                    window.focus(&focus);
                }
                window.dispatch_action(Box::new(crate::workspace::OpenPageContextMenu), cx);
            })?;
        }
    }
    Ok(())
}

/// Whichever of two futures is ready first: the first's output, or `None`
/// when the second (a timer) came first.
async fn futures_select<T>(
    first: impl std::future::Future<Output = Option<T>>,
    second: impl std::future::Future<Output = ()>,
) -> Option<T> {
    use std::{pin::pin, task::Poll};
    let mut first = pin!(first);
    let mut second = pin!(second);
    std::future::poll_fn(move |cx| {
        if let Poll::Ready(value) = first.as_mut().poll(cx) {
            return Poll::Ready(value);
        }
        if second.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// One line about the workspace, for the `state` step.
pub fn describe_state(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) -> String {
    use gpui::Focusable as _;
    let focus = if workspace.editor().read(cx).is_focused(window) {
        "editor"
    } else if workspace
        .panel()
        .focus_handle(cx)
        .contains_focused(window, cx)
    {
        "library"
    } else if workspace.comments().read(cx).has_focus(window, cx) {
        "comments"
    } else if workspace.modal_layer().read(cx).has_modal() {
        "modal"
    } else if workspace.tools().read(cx).active_tool().is_some() {
        "tool"
    } else if window.focused(cx).is_some() {
        "other"
    } else {
        "none"
    };
    let status = workspace.status_line(cx);
    let settings = workspace.settings();
    let vim = workspace.vim().read(cx);
    let vim = if vim.is_attached() {
        let status = vim.status();
        let mut text = status.mode_label().to_string();
        if !status.pending_keys.is_empty() {
            text.push_str(&format!(" pending={:?}", status.pending_keys));
        }
        if let Some(line) = &status.command_line {
            text.push_str(&format!(
                " line={:?}",
                format!("{}{}", line.prefix, line.text)
            ));
        }
        if let Some(message) = &status.message {
            text.push_str(&format!(" message={:?}", message.text));
        }
        text
    } else {
        "off".to_string()
    };
    let tools = workspace.tools().read(cx);
    let tool = match tools.active_tool() {
        Some(kind) => {
            let mut text = format!("{kind:?}");
            if let Some(label) = tools.selected_menu_label(cx) {
                text.push_str(&format!(" selected={label:?}"));
            }
            let find = tools.find_status();
            if !find.is_empty() {
                text.push_str(&format!(" find={find:?}"));
            }
            text
        }
        None => "none".to_string(),
    };
    let bar = if tools.format_bar_visible() {
        tools
            .format_bar_items(cx)
            .iter()
            .map(|item| match item.state {
                innerword_editor_tools::Mark::Check => format!("{}*", item.label),
                innerword_editor_tools::Mark::Mixed => format!("{}~", item.label),
                innerword_editor_tools::Mark::None => item.label.clone(),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    } else if tools.format_bar_enabled() {
        "none".to_string()
    } else {
        "off".to_string()
    };
    let comments = workspace.comments().read(cx);
    let comments = format!(
        "{} threads={} open={} active={:?} composing={}",
        if comments.is_visible() {
            "shown"
        } else {
            "hidden"
        },
        comments.threads().len(),
        comments.is_open(),
        comments.active_thread().map(|id| id.0),
        comments.is_composing(),
    );
    let last_save = match workspace.last_save_cost() {
        Some(cost) => {
            let millis = |duration: Duration| format!("{:.2} ms", duration.as_secs_f64() * 1000.);
            format!(
                "main {}, background {}, {} bytes",
                millis(cost.main_thread),
                cost.background.map(millis).unwrap_or_else(|| "none".into()),
                cost.bytes
            )
        }
        None => "none".to_string(),
    };
    let spelling = workspace.describe_spelling(cx);
    let shortcuts = workspace.describe_shortcuts(cx);
    // Agents that write while the app is open: the channel, the switch,
    // what came in.
    let live = workspace.describe_live(cx);
    // The Agents panel, while it is open.
    let agents = workspace
        .modal_layer()
        .read(cx)
        .active::<crate::agents_view::AgentsView>()
        .map(|view| format!(" agents=[{}]", view.read(cx).describe(cx)))
        .unwrap_or_default();
    // The assistant, in a run that has an engine for it.
    let assistant = workspace
        .describe_assistant(window, cx)
        .map(|assistant| {
            format!(
                " assistant=[{assistant}] right_panel={:?}",
                workspace.right_panel_shown()
            )
        })
        .unwrap_or_default();
    // The list of fonts of the settings, while it is open.
    let font_list = workspace
        .describe_font_list(cx)
        .map(|list| format!(" font_list=[{list}]"))
        .unwrap_or_default();
    let menu = match innerword_ui::ContextMenu::current(window, cx) {
        Some(menu) => menu.read(cx).labels().join(" | "),
        None => "none".to_string(),
    };
    let zoom = workspace.describe_zoom(cx);
    let print = workspace.describe_printing();
    let export = workspace.describe_pdf_export();
    let images = workspace.describe_images(cx);
    let web = workspace.describe_page(window, cx);
    let adding = workspace.describe_adding();
    format!(
        "document={:?} title={:?} save={:?} saving={} last_save=[{last_save}] focus={focus} \
         modal={} menu=[{menu}] sidebar={} page_view={} zoom=[{zoom}] print=[{print}] \
         export=[{export}] {images} \
         vim=[{vim}] tool=[{tool}] bar=[{bar}] comments=[{comments}] spelling=[{spelling}] \
         keys=[{shortcuts}] words={:?} characters={:?} \
         page={:?} \
         limit={:?} web=[{web}] adding=[{adding}] live=[{live}]{agents}{assistant}{font_list}",
        workspace.open_item_id().map(|id| id.0),
        workspace
            .document_title()
            .or(workspace.page_title())
            .unwrap_or_default(),
        workspace.save_state(),
        workspace.is_saving_in_background(),
        workspace.modal_layer().read(cx).has_modal(),
        settings.sidebar_visible,
        workspace.editor().read(cx).page_view(),
        status.words,
        status.characters,
        status.page,
        status.limit,
    )
}

/// Gives the window a chance to render between steps (about two frames),
/// and makes sure it did.
async fn frame_pause(
    window: AnyWindowHandle,
    view: Option<screenshot::NativeView>,
    cx: &mut AsyncApp,
) {
    draw_frame(window, view, cx);
    cx.background_executor()
        .timer(Duration::from_millis(32))
        .await;
    draw_frame(window, view, cx);
}

/// Draws a frame now if the window is dirty, also while it is covered by
/// other windows (see [`screenshot::force_frame`]). Must not be called
/// inside an update.
fn draw_frame(window: AnyWindowHandle, view: Option<screenshot::NativeView>, cx: &mut AsyncApp) {
    let Some(view) = view else {
        return;
    };
    // The view pointer is only good while the window is open.
    let open = cx
        .update(|cx| cx.windows().contains(&window))
        .unwrap_or(false);
    if open {
        screenshot::force_frame(view);
    }
}

/// What the step `shortcuts:import:<path>` logs: that the file was read,
/// or that there is none to read (`there`: what looking for the file gave).
pub fn import_log(path: &std::path::Path, there: std::io::Result<bool>) -> String {
    match there {
        Ok(true) => format!("shortcuts: read {}", path.display()),
        Ok(false) => format!(
            "shortcuts: {} is not a file, nothing was read",
            path.display()
        ),
        Err(error) => format!(
            "shortcuts: could not read {} ({error}), nothing was read",
            path.display()
        ),
    }
}

fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(&path))
            .unwrap_or(path)
    }
}
