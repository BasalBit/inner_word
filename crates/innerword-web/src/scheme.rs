//! The `WKURLSchemeHandler` that serves a page's files under `innerword-page://<host>/`.
//!
//! WebKit calls `webView:startURLSchemeTask:` on the main thread. The bytes are read
//! on a background queue (`PageSource::file`) and the answer is given back on the main
//! queue, where `webView:stopURLSchemeTask:` is called too: a task WebKit stopped
//! meanwhile is not answered, because answering a stopped task raises an exception.
//! Every task and the handler itself are retained while a read is in flight, so an
//! answer never meets a freed object.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{Arc, Once},
    time::Instant,
};

use dispatch::{Queue, QueuePriority};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Protocol, Sel},
    sel, sel_impl,
};

use crate::{
    PageSource, ServedFile, http,
    ns::{Id, nsstring, release, retain, string_of},
};

/// What the handler saw: for the view's statistics and the check's report and its
/// settling.
#[derive(Default)]
pub(crate) struct RequestLog {
    /// Answers given (any status).
    pub answered: Cell<usize>,
    /// Answers with a file (200, 206).
    pub served: Cell<usize>,
    /// Answers 404, counted.
    pub missing_count: Cell<usize>,
    /// The paths answered 404, each once, in order.
    pub missing: RefCell<Vec<String>>,
    /// When a request last came in or was answered.
    pub last_activity: Cell<Option<Instant>>,
}

impl RequestLog {
    pub(crate) fn touch(&self) {
        self.last_activity.set(Some(Instant::now()));
    }

    /// Starts a new load: the answers so far belong to the one before.
    pub(crate) fn reset(&self) {
        self.answered.set(0);
        self.served.set(0);
        self.missing_count.set(0);
        self.missing.borrow_mut().clear();
        self.touch();
    }
}

/// The handler's own state, behind its instance variable.
pub(crate) struct SchemeState {
    pub source: Arc<dyn PageSource>,
    /// The host whose files these are (`page-12`, `check`); another host is answered
    /// 404.
    pub host: String,
    /// The file `innerword-page://<host>/` stands for.
    pub entry: String,
    pub log: Rc<RequestLog>,
    /// Tasks started and neither stopped nor answered yet (their addresses; each is
    /// retained meanwhile).
    live: RefCell<HashSet<usize>>,
}

impl SchemeState {
    pub(crate) fn new(
        source: Arc<dyn PageSource>,
        host: &str,
        entry: &str,
        log: Rc<RequestLog>,
    ) -> SchemeState {
        SchemeState {
            source,
            host: host.to_ascii_lowercase(),
            entry: entry.trim_start_matches('/').to_string(),
            log,
            live: RefCell::new(HashSet::new()),
        }
    }
}

const CLASS: &str = "InnerWordWebSchemeHandler";
const STATE: &str = "writerState";

fn handler_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut decl) = ClassDecl::new(CLASS, class!(NSObject)) else {
            return;
        };
        if let Some(protocol) = Protocol::get("WKURLSchemeHandler") {
            decl.add_protocol(protocol);
        }
        decl.add_ivar::<*mut c_void>(STATE);
        // SAFETY: each function has the signature its selector's encoding says.
        unsafe {
            decl.add_method(
                sel!(webView:startURLSchemeTask:),
                start_task as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(
                sel!(webView:stopURLSchemeTask:),
                stop_task as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
        }
        decl.register();
    });
    Class::get(CLASS).expect("the scheme handler's class")
}

/// A new handler (owned by the caller: +1) that serves from `state`.
pub(crate) fn new_handler(state: SchemeState) -> Id {
    // SAFETY: `new` on a subclass of NSObject; the instance variable is ours.
    unsafe {
        let handler: Id = msg_send![handler_class(), new];
        let state = Box::into_raw(Box::new(state));
        (*handler).set_ivar::<*mut c_void>(STATE, state.cast());
        handler
    }
}

unsafe fn state_of(handler: &Object) -> Option<&SchemeState> {
    // SAFETY: the instance variable `new_handler` set; it lives until `dealloc`.
    unsafe {
        let state = *handler.get_ivar::<*mut c_void>(STATE);
        (state as *const SchemeState).as_ref()
    }
}

extern "C" fn dealloc(this: &mut Object, _: Sel) {
    // SAFETY: the box `new_handler` made, freed once, then NSObject's dealloc.
    unsafe {
        let state = *this.get_ivar::<*mut c_void>(STATE);
        if !state.is_null() {
            this.set_ivar::<*mut c_void>(STATE, std::ptr::null_mut());
            drop(Box::from_raw(state as *mut SchemeState));
        }
        let _: () = msg_send![super(this, class!(NSObject)), dealloc];
    }
}

extern "C" fn start_task(this: &Object, _: Sel, _web_view: Id, task: Id) {
    // SAFETY: on the main thread, with a live task WebKit handed over.
    unsafe {
        let Some(state) = state_of(this) else {
            return;
        };
        state.log.touch();
        let request: Id = msg_send![task, request];
        let url: Id = msg_send![request, URL];
        let absolute: Id = msg_send![url, absoluteString];
        let absolute = string_of(absolute);
        let range: Id = msg_send![request, valueForHTTPHeaderField: nsstring("Range")];
        let range = (!range.is_null()).then(|| string_of(range));
        let method: Id = msg_send![request, HTTPMethod];
        let head = string_of(method).eq_ignore_ascii_case("HEAD");

        retain(task);
        state.live.borrow_mut().insert(task as usize);

        let path = match http::split_url(&absolute) {
            Some((host, path)) if host == state.host => {
                if path.is_empty() {
                    state.entry.clone()
                } else {
                    path
                }
            }
            // Another page's files, or no URL of ours: nothing.
            _ => {
                finish(this, task, String::new(), None, range, head);
                return;
            }
        };

        // The handler stays alive until the answer is given.
        retain(this as *const Object as Id);
        let handler = this as *const Object as usize;
        let task = task as usize;
        let source = state.source.clone();
        Queue::global(QueuePriority::High).exec_async(move || {
            let file = catch_unwind(AssertUnwindSafe(|| source.file(&path))).unwrap_or(None);
            // Both objects were retained above and are released here.
            Queue::main().exec_async(move || {
                let handler = handler as Id;
                finish(&*handler, task as Id, path, file, range, head);
                release(handler);
            });
        });
    }
}

extern "C" fn stop_task(this: &Object, _: Sel, _web_view: Id, task: Id) {
    // SAFETY: the state of a live handler; the task stays retained until its read
    // comes back, which then finds it no longer live and only releases it.
    unsafe {
        if let Some(state) = state_of(this) {
            state.live.borrow_mut().remove(&(task as usize));
            state.log.touch();
        }
    }
}

/// Answers `task` (when it is still live) and releases it.
unsafe fn finish(
    handler: &Object,
    task: Id,
    path: String,
    file: Option<ServedFile>,
    range: Option<String>,
    head: bool,
) {
    // SAFETY: on the main thread; `task` is retained by `start_task`.
    unsafe {
        let live = match state_of(handler) {
            Some(state) => state.live.borrow_mut().remove(&(task as usize)),
            None => false,
        };
        if live {
            let answer = http::answer(file.as_ref(), range.as_deref(), head);
            respond(task, &answer, file.as_ref());
            if let Some(state) = state_of(handler) {
                let log = &state.log;
                log.answered.set(log.answered.get() + 1);
                if answer.status == 404 {
                    log.missing_count.set(log.missing_count.get() + 1);
                    let mut missing = log.missing.borrow_mut();
                    if !path.is_empty() && !missing.contains(&path) {
                        missing.push(path);
                    }
                } else if answer.status == 200 || answer.status == 206 {
                    log.served.set(log.served.get() + 1);
                }
                log.touch();
            }
        }
        release(task);
    }
}

/// The response, the body and the end of a task.
unsafe fn respond(task: Id, answer: &http::Answer, file: Option<&ServedFile>) {
    // SAFETY: Foundation's and WebKit's own selectors with the types they declare,
    // on a task that is live (not stopped).
    unsafe {
        let request: Id = msg_send![task, request];
        let url: Id = msg_send![request, URL];
        let keys: Vec<Id> = answer
            .headers
            .iter()
            .map(|(name, _)| nsstring(name))
            .collect();
        let values: Vec<Id> = answer
            .headers
            .iter()
            .map(|(_, value)| nsstring(value))
            .collect();
        let headers: Id = msg_send![class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr()
            forKeys: keys.as_ptr()
            count: keys.len()];
        let response: Id = msg_send![class!(NSHTTPURLResponse), alloc];
        let response: Id = msg_send![response,
            initWithURL: url
            statusCode: answer.status as isize
            HTTPVersion: nsstring("HTTP/1.1")
            headerFields: headers];
        let _: () = msg_send![task, didReceiveResponse: response];
        release(response);
        let body: &[u8] = match (answer.body, file) {
            (Some((start, end)), Some(file)) => &file.bytes[start..end],
            _ => answer.text.as_bytes(),
        };
        if !body.is_empty() {
            let data: Id = msg_send![class!(NSData),
                dataWithBytes: body.as_ptr() as *const c_void
                length: body.len()];
            let _: () = msg_send![task, didReceiveData: data];
        }
        let _: () = msg_send![task, didFinish];
    }
}
