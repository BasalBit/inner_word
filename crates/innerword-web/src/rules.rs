//! The content rule list that blocks every load of `http(s)`, `ws(s)`, `ftp` and `file`
//! ([`crate::BLOCK_RULES`]), compiled into a store of the caller's own directory under
//! the one fixed identifier [`crate::RULE_LIST_ID`]: it is looked up first and compiled
//! only when it is missing, so the store holds one list whatever the number of runs.
//! `WKContentRuleListStore.defaultStore` is never used.
//!
//! Once a list is there it is kept for the life of the process, per directory.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use block::ConcreteBlock;
use objc::{class, msg_send, runtime::YES, sel, sel_impl};

use crate::{
    BLOCK_RULES, RULE_LIST_ID,
    ns::{Id, error_text, guarded, nsstring, retain, string_of},
};

type Waiter = Box<dyn FnOnce(Result<Id, String>)>;

enum Slot {
    Ready(Id),
    Waiting(Vec<Waiter>),
}

thread_local! {
    static LISTS: RefCell<HashMap<PathBuf, Slot>> = RefCell::new(HashMap::new());
}

/// Hands the compiled list of `dir` to `then` (on the main thread): at once when the
/// process has it, else once it was looked up, or compiled. `Err` is the sentence of
/// why there is none.
pub(crate) fn when_ready(dir: &Path, then: impl FnOnce(Result<Id, String>) + 'static) {
    enum Next {
        Now(Id),
        Wait,
        Ask,
    }
    let mut then: Option<Waiter> = Some(Box::new(then));
    let next = LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        match lists.get_mut(dir) {
            Some(Slot::Ready(list)) => Next::Now(*list),
            Some(Slot::Waiting(waiters)) => {
                waiters.extend(then.take());
                Next::Wait
            }
            None => {
                lists.insert(
                    dir.to_path_buf(),
                    Slot::Waiting(then.take().into_iter().collect()),
                );
                Next::Ask
            }
        }
    });
    match next {
        Next::Now(list) => {
            if let Some(then) = then {
                then(Ok(list));
            }
        }
        Next::Wait => {}
        Next::Ask => look_up_or_compile(dir.to_path_buf()),
    }
}

/// Every waiter of `dir` gets `result`; a list is kept, an error forgotten (the next
/// caller tries again).
fn settle(dir: &Path, result: Result<Id, String>) {
    let waiters = LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        let waiters = match lists.remove(dir) {
            Some(Slot::Waiting(waiters)) => waiters,
            Some(ready @ Slot::Ready(_)) => {
                lists.insert(dir.to_path_buf(), ready);
                Vec::new()
            }
            None => Vec::new(),
        };
        if let Ok(list) = &result {
            lists.insert(dir.to_path_buf(), Slot::Ready(*list));
        }
        waiters
    });
    for waiter in waiters {
        let result = result.clone();
        guarded("a waiter for the rule list", move || waiter(result));
    }
}

/// The store of `dir` (made when missing).
fn store_of(dir: &Path) -> Result<Id, String> {
    std::fs::create_dir_all(dir).map_err(|error| {
        format!(
            "the folder of the block list could not be made ({}: {error})",
            dir.display()
        )
    })?;
    // SAFETY: Foundation's and WebKit's own selectors; the store is autoreleased.
    unsafe {
        let url: Id = msg_send![class!(NSURL),
            fileURLWithPath: nsstring(&dir.to_string_lossy())
            isDirectory: YES];
        let store: Id = msg_send![class!(WKContentRuleListStore), storeWithURL: url];
        if store.is_null() {
            return Err(format!(
                "WebKit made no store for the block list in {}",
                dir.display()
            ));
        }
        Ok(store)
    }
}

fn look_up_or_compile(dir: PathBuf) {
    let store = match store_of(&dir) {
        Ok(store) => store,
        Err(error) => {
            settle(&dir, Err(error));
            return;
        }
    };
    // SAFETY: the store is retained until both answers are in.
    let store = unsafe { retain(store) };
    let found = {
        let dir = dir.clone();
        ConcreteBlock::new(move |list: Id, _error: Id| {
            if !list.is_null() {
                // SAFETY: kept for the life of the process.
                let list = unsafe { retain(list) };
                // SAFETY: the reference taken above.
                unsafe { crate::ns::release(store) };
                settle(&dir, Ok(list));
                return;
            }
            compile(store, dir.clone());
        })
        .copy()
    };
    // SAFETY: WebKit's own selector; the block is copied by WebKit.
    unsafe {
        let _: () = msg_send![store,
            lookUpContentRuleListForIdentifier: nsstring(RULE_LIST_ID)
            completionHandler: &*found];
    }
}

fn compile(store: Id, dir: PathBuf) {
    let compiled = ConcreteBlock::new(move |list: Id, error: Id| {
        // SAFETY: the reference `look_up_or_compile` took.
        unsafe { crate::ns::release(store) };
        if list.is_null() {
            // SAFETY: an NSError or nil.
            let why = unsafe { error_text(error) };
            settle(
                &dir,
                Err(format!("the block list could not be compiled: {why}")),
            );
            return;
        }
        // SAFETY: kept for the life of the process.
        let list = unsafe { retain(list) };
        settle(&dir, Ok(list));
    })
    .copy();
    // SAFETY: WebKit's own selector; the block is copied by WebKit.
    unsafe {
        let _: () = msg_send![store,
            compileContentRuleListForIdentifier: nsstring(RULE_LIST_ID)
            encodedContentRuleList: nsstring(BLOCK_RULES)
            completionHandler: &*compiled];
    }
}

/// The identifiers of the lists in the store of `dir` (for the test that compiles
/// twice and finds one).
pub(crate) fn identifiers_in(dir: &Path, then: impl FnOnce(Vec<String>) + 'static) {
    let store = match store_of(dir) {
        Ok(store) => store,
        Err(_) => {
            then(Vec::new());
            return;
        }
    };
    let slot = RefCell::new(Some(then));
    let block = ConcreteBlock::new(move |identifiers: Id| {
        let mut names = Vec::new();
        if !identifiers.is_null() {
            // SAFETY: an NSArray of NSString.
            unsafe {
                let count: usize = msg_send![identifiers, count];
                for index in 0..count {
                    let name: Id = msg_send![identifiers, objectAtIndex: index];
                    names.push(string_of(name));
                }
            }
        }
        let then = slot.borrow_mut().take();
        if let Some(then) = then {
            then(names);
        }
    })
    .copy();
    // SAFETY: WebKit's own selector.
    unsafe {
        let _: () = msg_send![store, getAvailableContentRuleListIdentifiers: &*block];
    }
}

/// Forgets the lists the process holds, so that the next `with_rules` asks the store
/// again (the test that compiles twice).
pub(crate) fn forget() {
    LISTS.with(|lists| {
        lists
            .borrow_mut()
            .retain(|_, slot| matches!(slot, Slot::Waiting(_)))
    });
}
