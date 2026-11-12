//! The Agents panel: a modal that shows what agents on this Mac can do
//! with the library and sets them up for it.
//!
//! Three parts, top to bottom:
//!
//! - **Command line tool**: the absolute path of the `writer` binary that
//!   belongs to this copy of the app, with a button that copies it.
//! - **Skill**: every agent found on this Mac that reads skills, with the
//!   folder the skill goes to and whether it is there (not installed,
//!   installed, needs updating); a button per agent to install, update or
//!   remove it, and one to install it for all of them. What an action wrote
//!   or removed is listed below, path by path. Agents that were not found
//!   are named underneath.
//! - **MCP server**: for each agent the command, or the piece of its
//!   configuration file, that adds The Writer's MCP server to it, with the
//!   real path of the tool in it, a button that copies it and a link to the
//!   agent's own documentation. The app does not edit any agent's
//!   configuration.
//!
//! Opened with `cmd-alt-a`, "Agents" in the command palette, or
//! The Writer > Agents… in the menu bar. Everything in it comes from
//! [`crate::agents`] (and through it from `writer-agent-setup`).
//!
//! # Keys
//!
//! The panel is a list of rows; one is selected. In the `"AgentsView"`
//! context: `up` / `down` (also `ctrl-p` / `ctrl-n`) move, `home` / `end`
//! go to the first and the last row, `enter` does what the first button of
//! the row does (copy, install, update), `backspace` removes the skill of
//! the selected agent, `cmd-c` copies what the row shows, `o` opens the
//! documentation of the selected agent, `escape` closes. The footer names
//! what the keys do on the selected row. With the mouse: every row has its
//! buttons, and a click on a row selects it.

use gpui::{
    App, ClipboardItem, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, ScrollHandle, SharedString, Window, actions, div, prelude::*, px,
};
use writer_ui::{
    Button, ButtonSize, Icon, IconName, Toast,
    theme::{
        ActiveTheme, RADIUS_LG, RADIUS_MD, SPACE_2, SPACE_3, SPACE_4, UI_TEXT_BASE, UI_TEXT_LG,
        UI_TEXT_SM, UI_TEXT_XS,
    },
};

use writer_agent_setup::SkillState;

use crate::agents::{self, Environment, McpForm, Model, Report};

actions!(
    agents,
    [
        /// Select the next row.
        SelectNext,
        /// Select the previous row.
        SelectPrevious,
        /// Select the first row.
        SelectFirst,
        /// Select the last row.
        SelectLast,
        /// Press the first button of the selected row.
        Activate,
        /// Remove the skill of the selected agent.
        RemoveSkill,
        /// Copy what the selected row shows.
        CopyRow,
        /// Open the documentation of the selected agent.
        OpenDocumentation,
    ]
);

/// The key context of the Agents panel.
pub const KEY_CONTEXT: &str = "AgentsView";

/// The key bindings of the Agents panel.
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-n", SelectNext, c),
        KeyBinding::new("down", SelectNext, c),
        KeyBinding::new("ctrl-p", SelectPrevious, c),
        KeyBinding::new("up", SelectPrevious, c),
        KeyBinding::new("cmd-up", SelectFirst, c),
        KeyBinding::new("home", SelectFirst, c),
        KeyBinding::new("cmd-down", SelectLast, c),
        KeyBinding::new("end", SelectLast, c),
        KeyBinding::new("enter", Activate, c),
        KeyBinding::new("delete", RemoveSkill, c),
        KeyBinding::new("backspace", RemoveSkill, c),
        KeyBinding::new("cmd-c", CopyRow, c),
        KeyBinding::new("o", OpenDocumentation, c),
    ]
}

/// What the panel says first: what this is, and the one rule.
pub const INTRODUCTION: &str = "The AI agents on this Mac can work with your library through \
    the writer command line tool that comes inside The Writer: find and read documents, write \
    and edit them, leave comments. They can read at any time, and can only change things while \
    The Writer is closed.";

/// One row of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The path of the command line tool.
    Tool,
    /// "Install for all detected agents".
    InstallAll,
    /// The skill of the agent at this index of [`Model::skills`].
    Skill(usize),
    /// A way to add the MCP server to the agent at this index of
    /// [`Model::mcp`]: its command, or the piece of its configuration.
    Mcp(usize, McpForm),
}

/// What a key or a button does on a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deed {
    Copy,
    InstallAll,
    Install,
    Update,
    Remove,
    Documentation,
}

impl Deed {
    fn label(self, row: Row) -> &'static str {
        match (self, row) {
            (Deed::Copy, Row::Tool) => "Copy Path",
            (Deed::Copy, _) => "Copy",
            (Deed::InstallAll, _) => "Install for All",
            (Deed::Install, _) => "Install",
            (Deed::Update, _) => "Update",
            (Deed::Remove, _) => "Remove",
            (Deed::Documentation, _) => "Documentation",
        }
    }
}

/// The Agents panel. See the [module docs](self).
pub struct AgentsView {
    environment: Option<Environment>,
    model: Model,
    rows: Vec<Row>,
    selected: usize,
    /// What the last skill action did.
    report: Option<Report>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The child of the scrolled list each row is in.
    row_children: Vec<usize>,
}

impl EventEmitter<DismissEvent> for AgentsView {}

impl Focusable for AgentsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AgentsView {
    /// The panel over the app's agents environment ([`agents::environment`]).
    pub fn new(cx: &mut Context<Self>) -> Self {
        let environment = agents::environment(cx);
        let model = environment.as_ref().map(agents::load).unwrap_or_default();
        let mut this = Self {
            environment,
            model,
            rows: Vec::new(),
            selected: 0,
            report: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            row_children: Vec::new(),
        };
        this.rebuild_rows();
        this
    }

    fn rebuild_rows(&mut self) {
        let selected = self.rows.get(self.selected).copied();
        let mut rows = Vec::new();
        if self.environment.is_some() {
            rows.push(Row::Tool);
            if self.model.skills.len() > 1 {
                rows.push(Row::InstallAll);
            }
            rows.extend((0..self.model.skills.len()).map(Row::Skill));
            for (ix, agent) in self.model.mcp.iter().enumerate() {
                if agent.command.is_some() {
                    rows.push(Row::Mcp(ix, McpForm::Command));
                }
                if agent.config.is_some() {
                    rows.push(Row::Mcp(ix, McpForm::Config));
                }
            }
        }
        self.rows = rows;
        self.selected = selected
            .and_then(|row| self.rows.iter().position(|listed| *listed == row))
            .unwrap_or(0);
    }

    /// Reads the state of this Mac again (what is installed where).
    fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(environment) = &self.environment {
            self.model = agents::load(environment);
        }
        self.rebuild_rows();
        cx.notify();
    }

    // ----- what tests and the scripted run read ------------------------------------

    /// What the panel lists.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// The rows, top to bottom.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The selected row.
    pub fn selected_row(&self) -> Option<Row> {
        self.rows.get(self.selected).copied()
    }

    /// What the last install, update or removal did.
    pub fn report(&self) -> Option<&Report> {
        self.report.as_ref()
    }

    /// The labels of the buttons of `row`, in order.
    pub fn button_labels(&self, row: Row) -> Vec<&'static str> {
        self.deeds(row)
            .into_iter()
            .map(|deed| deed.label(row))
            .collect()
    }

    /// The text a copy on `row` puts on the clipboard.
    pub fn copy_text(&self, row: Row) -> Option<String> {
        match row {
            Row::Tool => Some(self.model.tool.to_string_lossy().into_owned()),
            Row::InstallAll => None,
            Row::Skill(ix) => self
                .model
                .skills
                .get(ix)
                .map(|skill| skill.directory.to_string_lossy().into_owned()),
            Row::Mcp(ix, form) => {
                let agent = self.model.mcp.get(ix)?;
                match form {
                    McpForm::Command => agent.command.clone(),
                    McpForm::Config => agent.config.as_ref().map(|config| config.text.clone()),
                }
            }
        }
    }

    /// One line that says what the panel shows: for the `state` step of a
    /// scripted run.
    pub fn describe(&self) -> String {
        let Some(environment) = &self.environment else {
            return "no environment".to_string();
        };
        let skills = self
            .model
            .skills
            .iter()
            .map(|skill| {
                format!(
                    "{}: {} in {}",
                    skill.name,
                    agents::skill_state_label(&skill.state),
                    skill.directory_display
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let report = match &self.report {
            Some(report) => format!("{:?} {:?}", report.headline, report.lines),
            None => "none".to_string(),
        };
        format!(
            "tool={:?} found={} home={:?} skills=[{skills}] not_found={:?} mcp={} rows={} \
             selected={:?} report=[{report}]",
            self.model.tool,
            self.model.tool_found,
            environment.home(),
            self.model.not_found,
            self.model.mcp.len(),
            self.rows.len(),
            self.selected_row(),
        )
    }

    // ----- selection ---------------------------------------------------------------

    /// Selects row `index` (clamped) and brings it into view.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = index.min(self.rows.len() - 1);
        if let Some(child) = self.row_children.get(self.selected) {
            self.scroll.scroll_to_item(*child);
        }
        cx.notify();
    }

    // ----- what a row can do -------------------------------------------------------

    /// The buttons of `row`, the first being what `enter` does.
    fn deeds(&self, row: Row) -> Vec<Deed> {
        match row {
            Row::Tool => vec![Deed::Copy],
            Row::InstallAll => vec![Deed::InstallAll],
            Row::Skill(ix) => match self.model.skills.get(ix) {
                // A copy no agent reads any more can only go.
                Some(skill) if skill.agent.is_none() => vec![Deed::Remove],
                Some(skill) => match skill.state {
                    SkillState::NotInstalled => vec![Deed::Install],
                    SkillState::Stale(_) => vec![Deed::Update, Deed::Remove],
                    SkillState::Current => vec![Deed::Remove],
                    // Somebody else's folder: nothing is written or removed.
                    SkillState::Foreign => Vec::new(),
                },
                None => Vec::new(),
            },
            Row::Mcp(..) => vec![Deed::Copy, Deed::Documentation],
        }
    }

    /// What `enter` does on `row`: its first button, unless that is
    /// "Remove" (removing has a key of its own, so that `enter` never
    /// takes anything away).
    fn primary(&self, row: Row) -> Option<Deed> {
        self.deeds(row)
            .into_iter()
            .next()
            .filter(|deed| *deed != Deed::Remove)
    }

    fn can(&self, row: Row, deed: Deed) -> bool {
        self.deeds(row).contains(&deed)
    }

    /// Does what the first button of the selected row does.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row()
            && let Some(deed) = self.primary(row)
        {
            self.perform(row, deed, cx);
        }
    }

    /// Removes the skill of the selected agent, when it is installed.
    pub fn remove_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row()
            && self.can(row, Deed::Remove)
        {
            self.perform(row, Deed::Remove, cx);
        }
    }

    /// Copies what the selected row shows.
    pub fn copy_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row() {
            self.copy(row, cx);
        }
    }

    /// Opens the documentation of the agent of the selected row.
    pub fn open_documentation(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row()
            && self.can(row, Deed::Documentation)
        {
            self.perform(row, Deed::Documentation, cx);
        }
    }

    fn perform(&mut self, row: Row, deed: Deed, cx: &mut Context<Self>) {
        let Some(environment) = self.environment.clone() else {
            return;
        };
        match (deed, row) {
            (Deed::Copy, _) => self.copy(row, cx),
            (Deed::InstallAll, _) => {
                self.report = Some(agents::install_skill_for_all(&environment));
                self.reload(cx);
            }
            (Deed::Install | Deed::Update, Row::Skill(ix)) => {
                if let Some(skill) = self.model.skills.get(ix) {
                    self.report = Some(agents::install_skill(&environment, skill));
                    self.reload(cx);
                }
            }
            (Deed::Remove, Row::Skill(ix)) => {
                if let Some(skill) = self.model.skills.get(ix) {
                    self.report = Some(agents::remove_skill(&environment, skill));
                    self.reload(cx);
                }
            }
            (Deed::Documentation, Row::Mcp(ix, _)) => {
                if let Some(agent) = self.model.mcp.get(ix) {
                    cx.open_url(&agent.docs_url);
                }
            }
            _ => {}
        }
    }

    fn copy(&mut self, row: Row, cx: &mut Context<Self>) {
        let Some(text) = self.copy_text(row) else {
            return;
        };
        let what = match row {
            Row::Tool => "the path of the tool".to_string(),
            Row::Skill(_) => "the folder of the skill".to_string(),
            Row::Mcp(ix, form) => {
                let name = self
                    .model
                    .mcp
                    .get(ix)
                    .map(|agent| agent.name.clone())
                    .unwrap_or_default();
                match form {
                    McpForm::Command => format!("the command for {name}"),
                    McpForm::Config => format!("the configuration for {name}"),
                }
            }
            Row::InstallAll => return,
        };
        let writes = self
            .environment
            .as_ref()
            .is_some_and(|environment| environment.clipboard);
        let message = if writes {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            format!("Copied {what}.")
        } else {
            // A scripted run leaves the user's clipboard alone.
            format!("Not copied in a scripted run: {what}.")
        };
        writer_ui::toast::show_toast(
            Toast::new(message).duration(std::time::Duration::from_secs(3)),
            cx,
        );
    }

    // ----- drawing -----------------------------------------------------------------

    fn render_button(
        &self,
        row_ix: usize,
        row: Row,
        deed: Deed,
        position: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let name = match deed {
            Deed::Copy => "copy",
            Deed::InstallAll => "install-all",
            Deed::Install => "install",
            Deed::Update => "update",
            Deed::Remove => "remove",
            Deed::Documentation => "documentation",
        };
        let button = Button::new(("agents-button", row_ix * 4 + position), deed.label(row))
            .size(ButtonSize::Compact)
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                this.select(row_ix, cx);
                this.perform(row, deed, cx);
            }));
        div()
            .debug_selector(move || format!("agents-{name}-{row_ix}"))
            .flex_none()
            .child(button)
    }

    fn render_row(&self, row_ix: usize, row: Row, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let mono = theme.fonts.mono.clone();
        let selected = row_ix == self.selected;
        let buttons: Vec<_> = self
            .deeds(row)
            .into_iter()
            .enumerate()
            .map(|(position, deed)| {
                self.render_button(row_ix, row, deed, position, cx)
                    .into_any_element()
            })
            .collect();
        // The content of a row is a block, not a flex column: wrapped text in
        // a flex column inside a flex row is measured at no width at all and
        // comes out thousands of pixels tall (seen with every multi-line
        // configuration entry).
        let code = |text: SharedString| {
            div()
                .px(SPACE_2)
                .py(px(5.))
                .rounded(RADIUS_MD)
                .bg(colors.canvas)
                .border_1()
                .border_color(colors.hairline)
                .font_family(mono.clone())
                .text_size(px(12.))
                .line_height(px(17.))
                .child(text)
        };
        let muted = |text: SharedString| {
            div()
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(text)
        };

        let content = match row {
            Row::Tool => {
                let path: SharedString = self.model.tool.to_string_lossy().into_owned().into();
                let mut column = div().flex_1().min_w_0().child(code(path));
                if !self.model.tool_found {
                    column = column.child(
                        div()
                            .debug_selector(|| "agents-tool-missing".into())
                            .mt(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(UI_TEXT_SM)
                            .line_height(px(16.))
                            .text_color(colors.danger)
                            .child(Icon::new(IconName::Alert).color(colors.danger))
                            .child(
                                "The tool is not at this path. This copy of The Writer was \
                                 built without it.",
                            ),
                    );
                } else if let Some(warning) = agents::temporary_place(&self.model.tool) {
                    column = column.child(
                        div()
                            .debug_selector(|| "agents-tool-temporary".into())
                            .mt(px(4.))
                            .text_size(UI_TEXT_SM)
                            .line_height(px(16.))
                            .text_color(colors.danger)
                            .child(warning),
                    );
                }
                column.into_any_element()
            }
            Row::InstallAll => div()
                .flex_1()
                .min_w_0()
                .child(format!(
                    "Install or update the skill for all {} agents found on this Mac",
                    self.model.skills.len()
                ))
                .into_any_element(),
            Row::Skill(ix) => {
                let skill = &self.model.skills[ix];
                let state_color = match skill.state {
                    SkillState::Current => colors.success,
                    SkillState::Stale(_) => colors.accent,
                    SkillState::NotInstalled => colors.muted,
                    SkillState::Foreign => colors.danger,
                };
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_2)
                            .child(div().truncate().child(skill.name.clone()))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(UI_TEXT_SM)
                                    .text_color(state_color)
                                    .child(agents::skill_state_label(&skill.state)),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(1.))
                            .truncate()
                            .font_family(mono.clone())
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .child(skill.directory_display.clone()),
                    )
                    .children(
                        skill
                            .detail
                            .clone()
                            .map(|detail| muted(detail.into()).mt(px(1.))),
                    )
                    .into_any_element()
            }
            Row::Mcp(ix, form) => {
                let agent = &self.model.mcp[ix];
                let column = div().flex_1().min_w_0();
                match form {
                    McpForm::Command => column
                        .child(muted("Run in a terminal:".into()))
                        .child(code(agent.command.clone().unwrap_or_default().into()).mt(px(4.)))
                        .into_any_element(),
                    McpForm::Config => {
                        let config = agent.config.as_ref();
                        let place = config
                            .map(|config| format!("{}. {}", config.place, config.placement))
                            .unwrap_or_default();
                        column
                            .child(muted(place.into()))
                            .child(
                                code(
                                    config
                                        .map(|config| config.text.clone())
                                        .unwrap_or_default()
                                        .into(),
                                )
                                .mt(px(4.)),
                            )
                            .into_any_element()
                    }
                }
            }
        };

        div()
            .id(("agents-row", row_ix))
            .debug_selector(move || format!("agents-row-{row_ix}"))
            .w_full()
            .px(SPACE_3)
            .py(px(7.))
            .flex()
            .items_start()
            .gap(SPACE_3)
            .rounded(RADIUS_MD)
            .when(selected, |el| el.bg(colors.accent_subtle))
            .when(!selected, |el| el.hover(|style| style.bg(colors.hover)))
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                this.select(row_ix, cx);
            }))
            .child(content)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .children(buttons),
            )
            .into_any_element()
    }

    fn render_report(&self, cx: &App) -> Option<gpui::AnyElement> {
        let report = self.report.as_ref()?;
        let theme = cx.theme();
        let colors = theme.colors;
        let mono = theme.fonts.mono.clone();
        let icon = if report.ok {
            Icon::new(IconName::Check).color(colors.success)
        } else {
            Icon::new(IconName::Alert).color(colors.danger)
        };
        Some(
            div()
                .debug_selector(|| "agents-report".into())
                .mx(SPACE_3)
                .mt(px(6.))
                .px(SPACE_3)
                .py(px(7.))
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(colors.hairline)
                .flex()
                .items_start()
                .gap(SPACE_2)
                .child(div().flex_none().pt(px(1.)).child(icon))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(report.headline.clone())
                        .children(report.lines.iter().map(|line| {
                            div()
                                .mt(px(2.))
                                .font_family(mono.clone())
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(line.clone())
                        }))
                        .children(report.note.clone().map(|note| {
                            div()
                                .mt(px(4.))
                                .text_size(UI_TEXT_SM)
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(note)
                        })),
                )
                .into_any_element(),
        )
    }
}

impl Render for AgentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font = theme.fonts.ui.clone();
        let overlay_shadow = theme.shadow_overlay();
        // Leave room for the modal's top offset, the header with the
        // introduction and the footer.
        let list_height = (f32::from(window.viewport_size().height) * 0.82 - 190.).max(160.);

        let heading = |title: &'static str, about: SharedString, first: bool| {
            div()
                .px(SPACE_3)
                .pt(if first { px(2.) } else { px(14.) })
                .pb(px(4.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(UI_TEXT_XS)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(UI_TEXT_SM)
                        .line_height(px(16.))
                        .text_color(colors.muted)
                        .child(about),
                )
        };
        let quiet = |text: SharedString| {
            div()
                .px(SPACE_3)
                .py(px(5.))
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(text)
        };

        // The children of the scrolled list. A row is one child, together
        // with what belongs above it (the heading of its section, the name
        // of its agent), so that bringing a row into view brings that
        // along.
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut row_children = Vec::with_capacity(self.rows.len());
        let rows = self.rows.clone();
        let first_skill_row = rows
            .iter()
            .position(|row| matches!(row, Row::InstallAll | Row::Skill(_)));
        let first_mcp_row = rows.iter().position(|row| matches!(row, Row::Mcp(..)));
        let skill_heading = || {
            heading(
                "SKILL",
                "A skill tells an agent that the tool exists and how to use it. It is a folder \
                 with one file, put where the agent looks for skills."
                    .into(),
                false,
            )
        };
        let mcp_heading = || {
            heading(
                "MCP SERVER",
                "The same tool is an MCP server. Each agent is told about it in its own way; \
                 The Writer does not edit the configuration of any agent."
                    .into(),
                false,
            )
        };
        // What follows the last skill row: the report and the agents that
        // were not found.
        let after_skills = |this: &Self, cx: &App| {
            let mut after: Vec<gpui::AnyElement> = Vec::new();
            after.extend(this.render_report(cx));
            if !this.model.not_found.is_empty() {
                after.push(
                    quiet(
                        format!(
                            "Also supported, not found on this Mac: {}.",
                            this.model.not_found.join(", ")
                        )
                        .into(),
                    )
                    .debug_selector(|| "agents-not-found".into())
                    .into_any_element(),
                );
            }
            after
        };
        let last_skill_row = rows
            .iter()
            .rposition(|row| matches!(row, Row::InstallAll | Row::Skill(_)));

        if self.environment.is_none() {
            children.push(
                quiet(
                    format!(
                        "Agents are not looked for in a scripted run unless {} names the home \
                         directory to use.",
                        agents::AGENT_HOME_ENV
                    )
                    .into(),
                )
                .into_any_element(),
            );
        }
        if let Some(problem) = &self.model.problem {
            children.push(
                quiet(problem.clone().into())
                    .text_color(colors.danger)
                    .into_any_element(),
            );
        }
        let mut agent_shown: Option<usize> = None;
        for (row_ix, row) in rows.iter().copied().enumerate() {
            let mut above: Vec<gpui::AnyElement> = Vec::new();
            if row == Row::Tool {
                above.push(
                    heading(
                        "COMMAND LINE TOOL",
                        "Agents call the tool by this path. Nothing is added to your PATH.".into(),
                        true,
                    )
                    .into_any_element(),
                );
            }
            if Some(row_ix) == first_skill_row {
                above.push(skill_heading().into_any_element());
            }
            if Some(row_ix) == first_mcp_row {
                // Without a single agent that reads skills, the section
                // still says so.
                if first_skill_row.is_none() {
                    above.push(skill_heading().into_any_element());
                    above.push(
                        quiet("No agent that reads skills was found on this Mac.".into())
                            .debug_selector(|| "agents-none-found".into())
                            .into_any_element(),
                    );
                    above.extend(after_skills(self, cx));
                }
                above.push(mcp_heading().into_any_element());
            }
            if let Row::Mcp(ix, _) = row
                && agent_shown != Some(ix)
            {
                agent_shown = Some(ix);
                let agent = &self.model.mcp[ix];
                above.push(
                    div()
                        .px(SPACE_3)
                        .pt(px(8.))
                        .pb(px(1.))
                        .flex()
                        .items_baseline()
                        .gap(SPACE_2)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(agent.name.clone()),
                        )
                        .when(!agent.detected, |el| {
                            el.child(
                                div()
                                    .text_size(UI_TEXT_SM)
                                    .text_color(colors.muted)
                                    .child("not found on this Mac"),
                            )
                        })
                        .into_any_element(),
                );
                above.extend(agent.note.clone().map(|note| {
                    div()
                        .px(SPACE_3)
                        .pb(px(2.))
                        .text_size(UI_TEXT_SM)
                        .line_height(px(16.))
                        .text_color(colors.muted)
                        .child(note)
                        .into_any_element()
                }));
            }
            let below = if Some(row_ix) == last_skill_row {
                after_skills(self, cx)
            } else {
                Vec::new()
            };
            row_children.push(children.len());
            children.push(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .children(above)
                    .child(self.render_row(row_ix, row, cx))
                    .children(below)
                    .into_any_element(),
            );
        }
        self.row_children = row_children;

        let close = crate::controls::close_control(
            cx.listener(|_, _: &gpui::ClickEvent, _, cx| cx.emit(DismissEvent)),
            cx,
        );
        // The footer names the keys as the keymap has them.
        let hint = |actions: &[&dyn gpui::Action], what: &'static str| {
            crate::controls::footer_hint(actions, KEY_CONTEXT, what, cx)
        };
        // What the keys do on the selected row.
        let selected = self.selected_row();
        let primary = selected.and_then(|row| self.primary(row).map(|deed| deed.label(row)));
        let removes = selected.is_some_and(|row| self.can(row, Deed::Remove));
        let documents = selected.is_some_and(|row| self.can(row, Deed::Documentation));
        // Named when enter does something else on the row.
        let copies = selected.is_some_and(|row| {
            self.copy_text(row).is_some() && self.primary(row) != Some(Deed::Copy)
        });

        div()
            .id("agents")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &SelectNext, _, cx| this.select(this.selected + 1, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                this.select(this.selected.saturating_sub(1), cx)
            }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select(0, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select(usize::MAX, cx)))
            .on_action(cx.listener(|this, _: &Activate, _, cx| this.activate(cx)))
            .on_action(cx.listener(|this, _: &RemoveSkill, _, cx| this.remove_selected(cx)))
            .on_action(cx.listener(|this, _: &CopyRow, _, cx| this.copy_selected(cx)))
            .on_action(
                cx.listener(|this, _: &OpenDocumentation, _, cx| this.open_documentation(cx)),
            )
            // Clicks on the panel must not close the modal or move the
            // keyboard out of it.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .w(px(crate::workspace::WIDE_MODAL_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(overlay_shadow)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .child(Icon::new(IconName::Keyboard).color(colors.muted))
                    .child(div().flex_1().text_size(UI_TEXT_LG).child("Agents"))
                    .child(close),
            )
            .child(
                div()
                    .flex_none()
                    .px(SPACE_4)
                    .pb(SPACE_3)
                    .border_b_1()
                    .border_color(colors.hairline)
                    .text_color(colors.muted)
                    .child(INTRODUCTION),
            )
            .child(
                div()
                    .id("agents-list")
                    .debug_selector(|| "agents-list".into())
                    .max_h(px(list_height))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(SPACE_2)
                    .flex()
                    .flex_col()
                    .children(children),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(34.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .border_t_1()
                    .border_color(colors.hairline)
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.muted)
                    .children(hint(&[&SelectPrevious, &SelectNext], "Choose"))
                    .children(primary.and_then(|label| hint(&[&Activate], label)))
                    .when(removes, |el| el.children(hint(&[&RemoveSkill], "Remove")))
                    .when(documents, |el| {
                        el.children(hint(&[&OpenDocumentation], "Documentation"))
                    })
                    .when(copies, |el| el.children(hint(&[&CopyRow], "Copy")))
                    .children(crate::controls::close_hint(cx)),
            )
    }
}
