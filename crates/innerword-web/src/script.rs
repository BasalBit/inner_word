//! One script, run offscreen, and its PDF (`docs/whiteboard-design.md`, sections 4.2 and
//! 5): what the whiteboard's export page needs, and the converter's cross-check.
//!
//! It generalises the check and the offscreen print: an offscreen window, a web view with
//! occlusion detection off (Excalidraw draws in animation frames), a data store of its
//! own, the rule list, the caller's policy on every answer, a host of its own
//! (`board-export`). The entry is loaded and `didFinish` waited for, then **one**
//! `callAsyncJavaScript` runs in the page world with the caller's body and argument.
//! With `pdf`, the answer is `{"width": w, "height": h}` in CSS pixels (points 1:1):
//! after `document.fonts.ready` and two animation frames (a second call),
//! `createPDFWithConfiguration:` makes the PDF of `(0, 0, ceil(w), ceil(h))` (WebKit drops
//! a fractional size), which must be one page.
//!
//! Main thread, from the run loop: WebKit's callbacks only write down what happened, a
//! timer moves the run on, `done` is called from a timer of its own.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use block::ConcreteBlock;
use objc::{class, msg_send, sel, sel_impl};

use crate::{
    PrintedPdf, ScriptInput, ScriptOutput, check, config, delegate, http,
    ns::{self, Id, NIL, release, retain},
    page, pixels, rules,
    scheme::{self, RequestLog, SchemeState},
};

/// The host of the script's files: never a page's (`page-…`) or a whiteboard's
/// (`board-…`) origin.
pub(crate) const HOST: &str = "board-export";
/// How often the run looks at where it stands.
const POLL: Duration = Duration::from_millis(20);
/// The size of the offscreen window.
const WINDOW: (f64, f64) = (1280., 900.);

/// After the script answered the size of the PDF: the fonts are ready and two frames
/// were drawn.
const SETTLE: &str = "await document.fonts.ready;\n\
await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));\n\
return '';";

type Done = Box<dyn FnOnce(Result<ScriptOutput, String>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// The entry is loading.
    Loading,
    /// The script runs.
    Running,
    /// The fonts and two frames, before the PDF.
    Settling,
    /// `createPDFWithConfiguration:` runs.
    Printing,
}

struct ScriptRun {
    started: Instant,
    limit: Duration,
    stage: Stage,
    entry: String,
    script: String,
    argument_json: String,
    pdf: bool,
    finished: bool,
    navigation_error: Option<String>,
    crashed: bool,
    /// The script's answer, kept while the PDF is made.
    value: String,
    /// The rectangle of the PDF.
    size: (f64, f64),
    window: Id,
    web_view: Id,
    delegate: Id,
    navigation: Id,
    log: Rc<RequestLog>,
    timer: Option<ns::Timer>,
    done: Option<Done>,
}

type Run = Rc<RefCell<ScriptRun>>;

/// The delegate's host of a run.
struct ScriptHost(Weak<RefCell<ScriptRun>>);

impl delegate::Host for ScriptHost {
    fn own_host(&self) -> String {
        HOST.to_string()
    }

    fn last_reader_event(&self) -> Option<f64> {
        None
    }

    fn main_frame_may_load(&self, _url: &str) -> bool {
        // Only the entry, once: a script that navigates the export page away would make
        // the PDF of something else.
        self.0.upgrade().is_some_and(|run| !run.borrow().finished)
    }

    fn finished(&self, navigation: Id) {
        if let Some(run) = self.0.upgrade() {
            let mut run = run.borrow_mut();
            if run.navigation == navigation {
                run.finished = true;
            }
        }
    }

    fn failed(&self, navigation: Id, message: String) {
        if let Some(run) = self.0.upgrade() {
            let mut run = run.borrow_mut();
            if run.navigation == navigation && !run.finished {
                run.navigation_error = Some(message);
            }
        }
    }

    fn terminated(&self) {
        if let Some(run) = self.0.upgrade() {
            run.borrow_mut().crashed = true;
        }
    }

    fn cancelled(&self, _url: &str) {}

    fn open_externally(&self, _url: String) {}

    fn alert(&self, _message: String) {}
}

/// Loads `input.entry` offscreen, runs `input.script` once in its page world, and (with
/// `input.pdf`) makes the one-page PDF of the size the script answers. Main thread; `done`
/// is called from the run loop, outside any caller's borrow. Fails on a crash of the
/// WebContent process, a navigation error, an entry the source does not have, a script
/// error (its message), an answer that is no size, a PDF that is not one page, and the
/// time limit.
pub fn run_script(input: ScriptInput, done: impl FnOnce(Result<ScriptOutput, String>) + 'static) {
    let done: Done = Box::new(done);
    if !ns::on_main_thread() {
        done(Err(
            "the script was asked to run off the main thread".to_string()
        ));
        return;
    }
    if let Err(why) = crate::available() {
        ns::after(Duration::ZERO, move || done(Err(why)));
        return;
    }
    let ScriptInput {
        source,
        entry,
        script,
        argument_json,
        pdf,
        csp,
        limit,
        rules_dir,
    } = input;
    let started = Instant::now();
    let entry = entry.trim_start_matches('/').to_string();
    rules::when_ready(&rules_dir, move |result| {
        let list = match result {
            Ok(list) => list,
            Err(why) => {
                ns::after(Duration::ZERO, move || {
                    done(Err(format!("the script could not be run: {why}")))
                });
                return;
            }
        };
        // SAFETY: on the main thread; WebKit's and AppKit's own selectors; what is made
        // here is held by the run and released by `end`.
        unsafe {
            let log = Rc::new(RequestLog::default());
            let handler = scheme::new_handler(
                SchemeState::new(source, HOST, &entry, log.clone()).with_policy(csp),
            );
            let store = config::fresh_data_store();
            let configuration = config::configuration(handler, store);
            release(store);
            release(handler);
            let controller: Id = msg_send![configuration, userContentController];
            let _: () = msg_send![controller, addContentRuleList: list];
            let window = check::offscreen_window_of(WINDOW);
            let web_view = check::checked_web_view(window, configuration);
            release(configuration);
            let run: Run = Rc::new(RefCell::new(ScriptRun {
                started,
                limit,
                stage: Stage::Loading,
                entry: entry.clone(),
                script,
                argument_json,
                pdf,
                finished: false,
                navigation_error: None,
                crashed: false,
                value: String::new(),
                size: (0., 0.),
                window,
                web_view,
                delegate: NIL,
                navigation: NIL,
                log: log.clone(),
                timer: None,
                done: Some(done),
            }));
            let delegate_object = delegate::new_delegate(Rc::new(ScriptHost(Rc::downgrade(&run))));
            let _: () = msg_send![web_view, setNavigationDelegate: delegate_object];
            let _: () = msg_send![web_view, setUIDelegate: delegate_object];
            run.borrow_mut().delegate = delegate_object;
            log.reset();
            let request = delegate::request_for(&http::url_of(HOST, &entry));
            if request.is_null() {
                fail(&run, format!("{entry} cannot be named as an address"));
                return;
            }
            let navigation: Id = msg_send![web_view, loadRequest: request];
            run.borrow_mut().navigation = retain(navigation);
            let ticking = run.clone();
            let timer = ns::every(POLL, move || tick(&ticking));
            run.borrow_mut().timer = Some(timer);
        }
    });
}

/// For a program (the tool's child `innerword __board-pdf`): as
/// [`crate::check_blocking`], for [`run_script`].
pub fn run_script_blocking(input: ScriptInput) -> Result<ScriptOutput, String> {
    ns::program_application();
    let limit = input.limit;
    let slot: Rc<RefCell<Option<Result<ScriptOutput, String>>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    run_script(input, move |result| *answer.borrow_mut() = Some(result));
    ns::run_until(limit + Duration::from_secs(10), || slot.borrow().is_some());
    let result = slot.borrow_mut().take();
    result.unwrap_or_else(|| Err("the script did not come to an end".to_string()))
}

fn tick(run: &Run) {
    enum Next {
        Wait,
        Call(Id, String, String),
        Fail(String),
    }
    let next = {
        let mut state = run.borrow_mut();
        let state = &mut *state;
        if state.done.is_none() {
            Next::Wait
        } else if state.crashed {
            Next::Fail("the page stopped working".to_string())
        } else if let Some(error) = &state.navigation_error {
            Next::Fail(format!("the page could not be loaded: {error}"))
        } else if state.started.elapsed() >= state.limit {
            Next::Fail(match state.stage {
                Stage::Loading => format!(
                    "the page did not finish loading in {} s",
                    state.limit.as_secs_f64()
                ),
                Stage::Running => format!(
                    "the script did not answer in {} s",
                    state.limit.as_secs_f64()
                ),
                Stage::Settling | Stage::Printing => {
                    format!("the PDF was not made in {} s", state.limit.as_secs_f64())
                }
            })
        } else if state.stage == Stage::Loading && state.finished {
            if state.log.missing.borrow().contains(&state.entry) {
                Next::Fail(format!(
                    "the page's entry {} is not among its files",
                    state.entry
                ))
            } else {
                state.stage = Stage::Running;
                Next::Call(
                    state.web_view,
                    state.script.clone(),
                    state.argument_json.clone(),
                )
            }
        } else {
            Next::Wait
        }
    };
    match next {
        Next::Wait => {}
        Next::Fail(why) => fail(run, why),
        Next::Call(web_view, script, argument) => {
            let weak = Rc::downgrade(run);
            // SAFETY: the run's web view, held until `end`.
            unsafe {
                page::call_async_in(
                    web_view,
                    &script,
                    &argument,
                    Box::new(move |answer| {
                        if let Some(run) = weak.upgrade() {
                            answered(&run, answer);
                        }
                    }),
                );
            }
        }
    }
}

/// The script answered.
fn answered(run: &Run, answer: Result<String, String>) {
    let value = match answer {
        Ok(value) => value,
        Err(why) => {
            fail(run, format!("the script failed: {why}"));
            return;
        }
    };
    let (pdf, web_view) = {
        let state = run.borrow();
        if state.done.is_none() {
            return;
        }
        (state.pdf, state.web_view)
    };
    if !pdf {
        succeed(run, ScriptOutput { value, pdf: None });
        return;
    }
    let Some(size) = size_of(&value) else {
        fail(
            run,
            format!(
                "the script's answer is not the size of a PDF: {}",
                excerpt(&value)
            ),
        );
        return;
    };
    {
        let mut state = run.borrow_mut();
        state.value = value;
        state.size = size;
        state.stage = Stage::Settling;
    }
    let weak = Rc::downgrade(run);
    // SAFETY: the run's web view, held until `end`.
    unsafe {
        page::call_async_in(
            web_view,
            SETTLE,
            "{}",
            Box::new(move |answer| {
                let Some(run) = weak.upgrade() else {
                    return;
                };
                match answer {
                    Ok(_) => print(&run),
                    Err(why) => fail(&run, format!("the page did not settle: {why}")),
                }
            }),
        );
    }
}

/// The rectangle of a PDF from the script's answer: `{"width": w, "height": h}`, both
/// finite and above nothing, rounded up (WebKit drops a fraction).
fn size_of(value: &str) -> Option<(f64, f64)> {
    let value: serde_json::Value = serde_json::from_str(value).ok()?;
    let width = value.get("width")?.as_f64()?;
    let height = value.get("height")?.as_f64()?;
    (width.is_finite() && height.is_finite() && width > 0. && height > 0.)
        .then(|| (width.ceil(), height.ceil()))
}

fn excerpt(text: &str) -> String {
    let short: String = text.chars().take(120).collect();
    if short.len() < text.len() {
        format!("{short}…")
    } else {
        short
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// SAFETY: the encoding of CGRect on 64-bit Apple platforms.
unsafe impl objc::Encode for CGRect {
    fn encode() -> objc::Encoding {
        // SAFETY: the type's own layout.
        unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

/// `createPDFWithConfiguration:` of the run's size.
fn print(run: &Run) {
    let (web_view, size) = {
        let mut state = run.borrow_mut();
        if state.done.is_none() {
            return;
        }
        state.stage = Stage::Printing;
        (state.web_view, state.size)
    };
    let weak = Rc::downgrade(run);
    let block = ConcreteBlock::new(move |data: Id, error: Id| {
        let Some(run) = weak.upgrade() else {
            return;
        };
        // SAFETY: an NSData or nil, and an NSError or nil.
        let bytes = unsafe {
            if !error.is_null() {
                Err(crate::ns::error_text(error))
            } else if data.is_null() {
                Err("WebKit made no PDF".to_string())
            } else {
                let length: usize = msg_send![data, length];
                let pointer: *const u8 = msg_send![data, bytes];
                if pointer.is_null() || length == 0 {
                    Err("WebKit made an empty PDF".to_string())
                } else {
                    Ok(std::slice::from_raw_parts(pointer, length).to_vec())
                }
            }
        };
        ns::guarded("a PDF's end", move || printed(&run, bytes));
    })
    .copy();
    // SAFETY: WebKit's own (macOS 11), asked for by `check_selectors`.
    unsafe {
        let configuration: Id = msg_send![class!(WKPDFConfiguration), new];
        let rect = CGRect {
            x: 0.,
            y: 0.,
            width: size.0,
            height: size.1,
        };
        let _: () = msg_send![configuration, setRect: rect];
        let _: () = msg_send![web_view,
            createPDFWithConfiguration: configuration
            completionHandler: &*block];
        release(configuration);
    }
}

fn printed(run: &Run, bytes: Result<Vec<u8>, String>) {
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(why) => {
            fail(run, format!("the PDF could not be made: {why}"));
            return;
        }
    };
    match pixels::pdf_pages(&bytes) {
        Some((1, paper)) => {
            let value = std::mem::take(&mut run.borrow_mut().value);
            succeed(
                run,
                ScriptOutput {
                    value,
                    pdf: Some(PrintedPdf {
                        bytes,
                        pages: 1,
                        paper,
                    }),
                },
            );
        }
        Some((pages, _)) => fail(run, format!("WebKit made a PDF of {pages} pages, not one")),
        None => fail(run, "WebKit's PDF has no pages".to_string()),
    }
}

fn succeed(run: &Run, output: ScriptOutput) {
    finish(run, Ok(output));
}

fn fail(run: &Run, why: String) {
    finish(run, Err(why));
}

/// Tears the run down and hands `result` on, once.
fn finish(run: &Run, result: Result<ScriptOutput, String>) {
    let (done, window, web_view, delegate_object, navigation, timer) = {
        let mut state = run.borrow_mut();
        let Some(done) = state.done.take() else {
            return;
        };
        (
            done,
            std::mem::replace(&mut state.window, NIL),
            std::mem::replace(&mut state.web_view, NIL),
            std::mem::replace(&mut state.delegate, NIL),
            std::mem::replace(&mut state.navigation, NIL),
            state.timer.take(),
        )
    };
    if let Some(timer) = &timer {
        timer.stop();
    }
    // SAFETY: the references `run_script` took, given back once.
    unsafe {
        release(navigation);
        check::tear_down(window, web_view, delegate_object);
    }
    // The timer is let go of after its own call has returned; `done` runs outside any
    // borrow of the run.
    ns::after(Duration::ZERO, move || {
        drop(timer);
        done(result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_of_a_pdf_is_rounded_up_and_never_nothing() {
        assert_eq!(
            size_of(r#"{"width":413.5,"height":200}"#),
            Some((414., 200.))
        );
        assert_eq!(size_of(r#"{"width":1,"height":0.2}"#), Some((1., 1.)));
        assert_eq!(size_of(r#"{"width":0,"height":20}"#), None);
        assert_eq!(size_of(r#"{"width":-3,"height":20}"#), None);
        assert_eq!(size_of(r#"{"width":"3","height":20}"#), None);
        assert_eq!(size_of(r#"{"height":20}"#), None);
        assert_eq!(size_of("not json"), None);
    }

    #[test]
    fn a_long_answer_is_cut_in_an_error() {
        let long = "x".repeat(500);
        assert_eq!(excerpt(&long).chars().count(), 121);
        assert_eq!(excerpt("short"), "short");
    }
}
