//! The model and effort pickers: two small controls that show the current
//! choice and open a menu of what the agent offers.
//!
//! The list is the engine's ([`Catalog`]): "the agent's own default" first,
//! then its models, or the effort levels the chosen model takes. Choosing a
//! model whose levels do not include the chosen effort puts the effort back
//! on the agent's default ([`Catalog::normalized`]).
//!
//! ```ignore
//! // In render: the control. `bounds` remembers where it was drawn, so the
//! // menu can drop down from it (also when a key opens it).
//! picker_button(PickerKind::Model, &catalog, &selection, &self.model_bounds, focused, &colors, cx)
//!     .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerKind::Model, false, window, cx)))
//!
//! // On a click or a key:
//! let this = cx.weak_entity();
//! open_picker_menu(PickerKind::Model, &catalog, &selection, &self.model_bounds, from_keyboard,
//!     move |selection, _window, cx| {
//!         this.update(cx, |this, cx| this.set_selection(selection, cx)).ok();
//!     }, window, cx);
//! ```
//!
//! The menu is a `writer_ui::ContextMenu`: it never takes the keyboard, the
//! arrows and `enter` work in it, and the window needs a
//! `writer_ui::ToastLayer` or `writer_ui::ContextMenuLayer` to draw it.

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, Div, Entity, Pixels, SharedString, Stateful, Window, canvas, div, point,
    prelude::*, px,
};
use writer_assistant::{Catalog, EffortChoice, ModelChoice, Selection};
use writer_ui::{
    ContextMenu, ContextMenuItem, Icon, IconName, KeyHint, MenuAnchor, Tooltip,
    theme::{RADIUS_SM, ThemeColors, UI_TEXT_SM},
};

use crate::{
    actions::{ChooseEffort, ChooseModel},
    controls::{keep_focus, keyboard_mark},
};

/// Where a control was drawn in the last frame: a menu drops down from
/// there.
#[derive(Clone, Default)]
pub struct ControlBounds(Rc<Cell<Option<Bounds<Pixels>>>>);

impl ControlBounds {
    /// Nothing drawn yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bounds of the control in the last frame that drew it.
    pub fn get(&self) -> Option<Bounds<Pixels>> {
        self.0.get()
    }

    /// Forgets the bounds (the control is not drawn any more).
    pub fn clear(&self) {
        self.0.set(None);
    }

    /// An invisible child that fills its (`relative`) parent and notes the
    /// parent's bounds when it is drawn.
    pub fn probe(&self) -> impl IntoElement + use<> {
        let cell = self.0.clone();
        canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .size_full()
    }
}

/// Which of the two pickers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PickerKind {
    /// The model that answers.
    Model,
    /// How much effort it spends.
    Effort,
}

impl PickerKind {
    /// The debug selector of the control.
    pub fn selector(self) -> &'static str {
        match self {
            PickerKind::Model => "assistant-model",
            PickerKind::Effort => "assistant-effort",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            PickerKind::Model => "Model",
            PickerKind::Effort => "Effort",
        }
    }
}

/// What the control of a picker shows: the name of the choice, or
/// "Default" for the agent's own (the menu says whose default it is).
pub fn picker_label(kind: PickerKind, catalog: &Catalog, selection: &Selection) -> SharedString {
    match kind {
        PickerKind::Model => match &selection.model {
            ModelChoice::AgentDefault => "Default model".into(),
            choice => catalog.model_label(choice).into(),
        },
        PickerKind::Effort => match &selection.effort {
            EffortChoice::AgentDefault => "Default effort".into(),
            choice => catalog.effort_label(choice).into(),
        },
    }
}

/// Whether the picker has anything to choose from: a model without an
/// effort setting leaves the effort picker with the default alone.
pub fn picker_has_choices(kind: PickerKind, catalog: &Catalog, selection: &Selection) -> bool {
    match kind {
        PickerKind::Model => !catalog.models.is_empty(),
        PickerKind::Effort => !catalog.efforts_for(&selection.model).is_empty(),
    }
}

/// The control of a picker: the current choice and a small chevron. The
/// caller attaches the click handler (see the [module docs](self)).
pub fn picker_button(
    kind: PickerKind,
    catalog: &Catalog,
    selection: &Selection,
    bounds: &ControlBounds,
    focused: bool,
    colors: &ThemeColors,
    cx: &App,
) -> Stateful<Div> {
    let name = kind.selector();
    let hint = match kind {
        PickerKind::Model => KeyHint::for_action(&ChooseModel, cx),
        PickerKind::Effort => KeyHint::for_action(&ChooseEffort, cx),
    };
    let enabled = picker_has_choices(kind, catalog, selection);
    let hover = colors.hover;
    let ink = colors.ink;
    let button = div()
        .id(name)
        .debug_selector(move || name.into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .h(px(22.))
        .pl(px(6.))
        .pr(px(3.))
        .rounded(RADIUS_SM)
        .whitespace_nowrap()
        .text_size(UI_TEXT_SM)
        .line_height(px(16.))
        .text_color(colors.muted)
        .border_1()
        .border_color(gpui::transparent_black())
        .map(|button| keyboard_mark(button, focused, colors))
        .map(|button| {
            if enabled {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover).text_color(ink))
            } else {
                button.opacity(0.5).cursor_default()
            }
        })
        .child(bounds.probe())
        .child(picker_label(kind, catalog, selection))
        .child(
            Icon::new(IconName::ChevronDown)
                .size(px(12.))
                .color(colors.faint),
        )
        .tooltip(Tooltip::new(kind.tooltip()).key_hint(hint).build());
    keep_focus(button)
}

/// The items of a picker's menu. `on_choose` gets the whole selection with
/// the choice made in it.
pub fn picker_menu_items(
    kind: PickerKind,
    catalog: &Catalog,
    selection: &Selection,
    on_choose: impl Fn(Selection, &mut Window, &mut App) + 'static,
) -> Vec<ContextMenuItem> {
    let on_choose = Rc::new(on_choose);
    let item = |title: String, checked: bool, chosen: Selection| {
        let on_choose = on_choose.clone();
        ContextMenuItem::new(title, move |window, cx| {
            on_choose(chosen.clone(), window, cx)
        })
        .checked(checked)
    };
    let mut items = Vec::new();
    match kind {
        PickerKind::Model => {
            let choose = |model: ModelChoice| {
                catalog.normalized(&Selection {
                    model,
                    effort: selection.effort.clone(),
                })
            };
            items.push(item(
                catalog.agent_default_label.clone(),
                selection.model == ModelChoice::AgentDefault,
                choose(ModelChoice::AgentDefault),
            ));
            if !catalog.models.is_empty() {
                items.push(ContextMenuItem::separator());
            }
            for model in &catalog.models {
                let choice = ModelChoice::named(model.id.clone());
                items.push(item(
                    model.name.clone(),
                    selection.model == choice,
                    choose(choice),
                ));
            }
            // A remembered model the agent no longer lists is still what
            // is sent: show it, checked.
            if let ModelChoice::Named(id) = &selection.model
                && catalog.model(id).is_none()
            {
                items.push(item(id.clone(), true, selection.clone()));
            }
        }
        PickerKind::Effort => {
            let choose = |effort: EffortChoice| Selection {
                model: selection.model.clone(),
                effort,
            };
            items.push(item(
                catalog.effort_label(&EffortChoice::AgentDefault),
                selection.effort == EffortChoice::AgentDefault,
                choose(EffortChoice::AgentDefault),
            ));
            let efforts = catalog.efforts_for(&selection.model);
            if !efforts.is_empty() {
                items.push(ContextMenuItem::separator());
            }
            for effort in efforts {
                let choice = EffortChoice::named(effort.id.clone());
                items.push(item(
                    effort.name.clone(),
                    selection.effort == choice,
                    choose(choice),
                ));
            }
        }
    }
    items
}

/// Opens the menu of a picker under its control (`bounds`; under the top
/// left corner of the window while the control was never drawn). From the
/// keyboard the current choice is marked, so the arrows start from it and
/// `enter` keeps it. Returns the open menu.
#[allow(clippy::too_many_arguments)]
pub fn open_picker_menu(
    kind: PickerKind,
    catalog: &Catalog,
    selection: &Selection,
    bounds: &ControlBounds,
    from_keyboard: bool,
    on_choose: impl Fn(Selection, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let items = picker_menu_items(kind, catalog, selection, on_choose);
    let anchor = match bounds.get() {
        Some(bounds) => MenuAnchor::Below(bounds),
        None => MenuAnchor::Point(point(px(8.), px(8.))),
    };
    let menu = ContextMenu::open(anchor, items, window, cx);
    if from_keyboard {
        menu.update(cx, |menu, cx| menu.select_checked(cx));
    }
    menu
}
