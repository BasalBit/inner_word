//! The keymap as a whole: every crate's bindings together.

use std::collections::BTreeMap;

use gpui::TestAppContext;

use super::init;

/// Every binding as (keys, context, action), in keymap order.
fn bindings(cx: &mut TestAppContext) -> Vec<(String, String, String)> {
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap
            .bindings()
            .map(|binding| {
                let keys = binding
                    .keystrokes()
                    .iter()
                    .map(|key| key.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ");
                let context = binding
                    .predicate()
                    .map(|predicate| predicate.to_string())
                    .unwrap_or_default();
                (keys, context, binding.action().name().to_string())
            })
            .collect()
    })
}

#[gpui::test]
fn keymap_dump(cx: &mut TestAppContext) {
    init(cx);
    let mut by_key: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new();
    for (ix, (keys, context, action)) in bindings(cx).into_iter().enumerate() {
        by_key.entry(keys).or_default().push((ix, context, action));
    }
    for (keys, list) in by_key {
        if list.len() > 1 {
            println!("{keys}");
            for (ix, context, action) in list {
                println!("    #{ix:<4} [{context}] {action}");
            }
        }
    }
}
