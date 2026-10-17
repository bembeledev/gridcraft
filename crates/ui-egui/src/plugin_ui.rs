//! What plugins add to the app: ribbon groups (on a built-in tab or a tab of their own) and
//! right-click menu items, declared in their manifest's `ui` section. Each runs one of the
//! plugin's commands through `plugins.run`.
//!
//! Protocol 1.2: when a button, menu item or plugin dialog ran the command, its result's `open`
//! opens the file as File › Open does and its `dialog` is drawn here ([`dialog_window`]), whose
//! OK and buttons run the next plugin command the same way (so dialogs chain). What plugins
//! answer to document events shows in the status bar or the info bar ([`poll_events`]).

use egui::Ui;
use gridcraft_engine::plugin::{self, ButtonSize, MenuKind};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::icons::Icon;
use crate::widgets::{big_button, small_button};

/// One button or menu item.
pub struct Item {
    pub plugin: String,
    pub command: String,
    pub label: String,
    pub tooltip: String,
    pub icon: Icon,
    pub small: bool,
    pub params: Json,
}

/// The tabs plugins add (names not among the built-in tabs), in load order.
pub fn extra_tabs() -> Vec<String> {
    let mut tabs: Vec<String> = Vec::new();
    for p in plugin::list().into_iter().filter(|p| p.active()) {
        for g in &p.manifest.ui.ribbon {
            let built_in = plugin::RIBBON_TABS.contains(&g.tab.as_str()) || (g.tab == "Developer" && crate::developer_ui::visible());
            if !built_in && !tabs.contains(&g.tab) {
                tabs.push(g.tab.clone());
            }
        }
    }
    tabs
}

/// The plugin groups on `tab`: (group name, buttons).
pub fn groups(tab: &str) -> Vec<(String, Vec<Item>)> {
    let mut out = Vec::new();
    for p in plugin::list().into_iter().filter(|p| p.active()) {
        let m = &p.manifest;
        for g in m.ui.ribbon.iter().filter(|g| g.tab == tab) {
            let buttons = g
                .buttons
                .iter()
                .map(|b| {
                    let decl = m.command(&b.command);
                    Item {
                        plugin: m.id.clone(),
                        command: b.command.clone(),
                        label: b.label.clone().or_else(|| decl.map(|c| c.label.clone())).unwrap_or_else(|| b.command.clone()),
                        tooltip: b.tooltip.clone().or_else(|| decl.map(|c| c.description.clone())).unwrap_or_default(),
                        icon: b.icon.as_deref().and_then(icon_named).unwrap_or(Icon::Script),
                        small: b.size == ButtonSize::Small,
                        params: b.params.clone(),
                    }
                })
                .collect();
            out.push((g.group.clone(), buttons));
        }
    }
    out
}

/// The plugin items for a right-click menu.
pub fn menu_items(kind: MenuKind) -> Vec<Item> {
    let mut out = Vec::new();
    for p in plugin::list().into_iter().filter(|p| p.active()) {
        let m = &p.manifest;
        for item in m.ui.menus.iter().filter(|i| i.menu == kind) {
            let decl = m.command(&item.command);
            out.push(Item {
                plugin: m.id.clone(),
                command: item.command.clone(),
                label: item.label.clone().or_else(|| decl.map(|c| c.label.clone())).unwrap_or_else(|| item.command.clone()),
                tooltip: decl.map(|c| c.description.clone()).unwrap_or_default(),
                icon: Icon::Script,
                small: true,
                params: item.params.clone(),
            });
        }
    }
    out
}

/// Runs an item's command; `extra` fields (e.g. the right-clicked sheet) are added to its params.
pub fn run(app: &mut SheetApp, item: &Item, extra: Json) {
    let mut params = if item.params.is_object() { item.params.clone() } else { json!({}) };
    if let (Some(p), Json::Object(e)) = (params.as_object_mut(), extra) {
        for (k, v) in e {
            p.entry(k).or_insert(v);
        }
    }
    app.run_or_alert("plugins.run", json!({"plugin": item.plugin, "command": item.command, "params": params}));
}

/// `plugins.run` from the interface (a button, a menu item, a plugin dialog): runs the command,
/// shows its error, and performs what its result asks for after the step: opens `open`, joins
/// `collab.join` and shows `dialog`.
pub fn run_command(app: &mut SheetApp, params: Json) {
    let plugin = params.get("plugin").and_then(Json::as_str).unwrap_or("").to_string();
    match app.run("plugins.run", params) {
        Ok(r) => follow(app, &plugin, &r),
        Err(e) => app.alert(e),
    }
}

/// Performs a plugin command's `open`, `collab.join` and `dialog` (already validated by the engine).
pub fn follow(app: &mut SheetApp, plugin: &str, r: &Json) {
    if let Some(o) = r.get("open").and_then(|o| plugin::dialog::OpenRequest::parse(o).ok()) {
        app.open_path(&o.path);
        // The label shows where the app shows the document's location (the status bar). GridCraft
        // has no read-only mode: `readOnly` is ignored.
        if let Some(d) = app.session.active().filter(|d| d.path.as_deref() == Some(o.path.as_str())) {
            let uid = d.uid;
            match o.label {
                Some(l) => {
                    app.doc_labels.insert(uid, l);
                }
                None => {
                    app.doc_labels.remove(&uid);
                }
            }
        }
    }
    // Protocol 2: the room the transport's command set up (its Share or Join). The invitation
    // it returned (`result.link`) is kept for the title bar's Share › Copy invitation.
    if let Some(join) = r.get("collab").and_then(|c| c.get("join")).filter(|j| j.is_object()) {
        let mut join = join.clone();
        if join.get("plugin").and_then(Json::as_str).is_none() {
            join["plugin"] = json!(plugin);
        }
        match app.run("collab.join", join) {
            Ok(out) => {
                let link = r.get("result").and_then(|x| x.get("link")).and_then(Json::as_str);
                if let (Some(room), Some(link)) = (out.get("room").and_then(Json::as_str), link) {
                    app.collab_invites.insert(room.to_string(), link.to_string());
                }
            }
            Err(e) => app.alert(e),
        }
    }
    if let Some(d) = r.get("dialog") {
        // A message that comes with a dialog goes to the status bar, so two modals don't stack.
        if let Some(m) = r.get("message").and_then(Json::as_str)
            && app.message.as_ref().is_some_and(|(_, shown)| shown == m)
        {
            app.message = None;
            app.status_message = Some((m.to_string(), crate::now_ms()));
        }
        match plugin::dialog::Dialog::parse(d) {
            Ok(dialog) => app.plugin_dialog = Some(DialogState::new(plugin, dialog)),
            Err(e) => app.alert(e),
        }
    }
}

// ---------------------------------------------------------------- dialogs (protocol 1.2)

/// A form field being edited.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// Text, secret and number fields (a number is parsed on OK).
    Text(String),
    Check(bool),
    /// A choice field's option id.
    Choice(String),
}

/// A plugin's dialog on screen.
pub struct DialogState {
    pub plugin: String,
    pub dialog: plugin::dialog::Dialog,
    /// The list's filter text.
    pub filter: String,
    /// The selected items (indices into the list), in the order they were picked.
    pub selected: Vec<usize>,
    pub values: Vec<FieldValue>,
}

impl DialogState {
    pub fn new(plugin: &str, dialog: plugin::dialog::Dialog) -> DialogState {
        let values = match &dialog.content {
            plugin::dialog::Content::Form { fields } => fields
                .iter()
                .map(|f| match f.kind {
                    plugin::dialog::FieldKind::Checkbox => FieldValue::Check(f.value.as_bool().unwrap_or(false)),
                    plugin::dialog::FieldKind::Choice => FieldValue::Choice(f.value.as_str().unwrap_or("").to_string()),
                    plugin::dialog::FieldKind::Number => FieldValue::Text(match &f.value {
                        Json::Number(n) => n.to_string(),
                        _ => String::new(),
                    }),
                    _ => FieldValue::Text(f.value.as_str().unwrap_or("").to_string()),
                })
                .collect(),
            _ => Vec::new(),
        };
        DialogState { plugin: plugin.to_string(), dialog, filter: String::new(), selected: Vec::new(), values }
    }

    /// The selected items' ids.
    pub fn selection(&self) -> Vec<String> {
        match &self.dialog.content {
            plugin::dialog::Content::List { items, .. } => self.selected.iter().filter_map(|i| items.get(*i)).map(|it| it.id.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// The form's values by field id: text, a number (null when it isn't one), true/false, an
    /// option id.
    pub fn fields(&self) -> serde_json::Map<String, Json> {
        let mut out = serde_json::Map::new();
        if let plugin::dialog::Content::Form { fields } = &self.dialog.content {
            for (f, v) in fields.iter().zip(&self.values) {
                let value = match (f.kind, v) {
                    (plugin::dialog::FieldKind::Number, FieldValue::Text(t)) => {
                        t.trim().replace(',', ".").parse::<f64>().ok().filter(|x| x.is_finite()).map_or(Json::Null, |x| json!(x))
                    }
                    (_, FieldValue::Text(t)) => json!(t),
                    (_, FieldValue::Check(b)) => json!(b),
                    (_, FieldValue::Choice(c)) => json!(c),
                };
                out.insert(f.id.clone(), value);
            }
        }
        out
    }

    /// The `plugins.run` params OK runs (None for a message dialog without a command).
    pub fn ok_run(&self) -> Option<Json> {
        let command = self.dialog.command.as_ref()?;
        let params = self.dialog.ok_params(&self.selection(), &self.fields());
        Some(json!({"plugin": self.plugin, "command": command, "params": params}))
    }

    /// The `plugins.run` params extra button `i` runs.
    pub fn button_run(&self, i: usize) -> Option<Json> {
        let (command, params) = self.dialog.button_params(i, &self.selection(), &self.fields())?;
        Some(json!({"plugin": self.plugin, "command": command, "params": params}))
    }

    /// Whether OK can run: a list needs a selected item.
    pub fn can_ok(&self) -> bool {
        match &self.dialog.content {
            plugin::dialog::Content::List { .. } => !self.selected.is_empty(),
            _ => true,
        }
    }

    /// The list items the filter lets through (indices), all of them when there's no filter box.
    pub fn visible(&self) -> Vec<usize> {
        let plugin::dialog::Content::List { items, .. } = &self.dialog.content else { return Vec::new() };
        let needle = self.filter.trim().to_lowercase();
        if items.len() <= plugin::dialog::FILTER_AFTER || needle.is_empty() {
            return (0..items.len()).collect();
        }
        items.iter().enumerate().filter(|(_, it)| it.label.to_lowercase().contains(&needle)).map(|(i, _)| i).collect()
    }
}

enum DialogAction {
    Keep,
    Close,
    Run(Json),
}

/// Draws the plugin's dialog, if one is up: a modal the app draws itself, with the theme's
/// colours. OK, a double-clicked item or a button runs the next plugin command; Cancel and Escape
/// close it.
pub fn dialog_window(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(mut st) = app.plugin_dialog.take() else { return };
    let lang = app.ui.language;
    let t = crate::theme::Tokens::get(ctx);
    let mut action = DialogAction::Keep;
    // A form with a multi-line field (code, say) gets room for its lines.
    let wide = matches!(&st.dialog.content, plugin::dialog::Content::Form { fields } if fields.iter().any(|f| f.multiline));
    egui::Modal::new(egui::Id::new("plugin_dialog")).show(ctx, |ui| {
        ui.set_width(if wide { 640.0 } else { 460.0 });
        if !st.dialog.title.is_empty() {
            ui.label(egui::RichText::new(&st.dialog.title).strong().size(15.0).color(t.text));
            ui.add_space(4.0);
        }
        if !st.dialog.prompt.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&st.dialog.prompt).color(t.text)).wrap());
            ui.add_space(6.0);
        }
        match st.dialog.content.clone() {
            plugin::dialog::Content::List { items, multi } => {
                if items.len() > plugin::dialog::FILTER_AFTER {
                    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(lang.tr("Filter")).desired_width(f32::INFINITY));
                    ui.add_space(4.0);
                }
                let visible = st.visible();
                let row_h = 26.0;
                egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.input_border)).fill(t.input_bg).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show_rows(ui, row_h, visible.len(), |ui, range| {
                        for &i in visible.get(range).unwrap_or(&[]) {
                            let Some(it) = items.get(i) else { continue };
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
                            let on = st.selected.contains(&i);
                            if on {
                                ui.painter().rect_filled(rect, 2.0, t.accent_soft);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 2.0, t.hover);
                            }
                            let mut x = rect.left() + 6.0;
                            if multi {
                                let b = egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), egui::vec2(12.0, 12.0));
                                ui.painter().rect_stroke(b, 2.0, egui::Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
                                if on {
                                    crate::icons::paint(ui.painter(), b.shrink(1.0), Icon::Check, t.accent);
                                }
                                x += 20.0;
                            }
                            let icon = icon_named(&it.icon).unwrap_or(Icon::Paper);
                            crate::icons::paint(
                                ui.painter(),
                                egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), egui::vec2(16.0, 16.0)),
                                icon,
                                t.text_dim,
                            );
                            x += 24.0;
                            let font = crate::theme::ui_font(13.0);
                            ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, &it.label, font.clone(), t.text);
                            if !it.detail.is_empty() {
                                ui.painter().text(
                                    egui::pos2(rect.right() - 8.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    &it.detail,
                                    crate::theme::ui_font(12.0),
                                    t.text_dim,
                                );
                            }
                            let label = it.label.clone();
                            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, &label));
                            if resp.double_clicked() && !multi {
                                st.selected = vec![i];
                                if let Some(run) = st.ok_run() {
                                    action = DialogAction::Run(run);
                                }
                            } else if resp.clicked() {
                                if multi {
                                    if let Some(k) = st.selected.iter().position(|s| *s == i) {
                                        st.selected.remove(k);
                                    } else {
                                        st.selected.push(i);
                                    }
                                } else {
                                    st.selected = vec![i];
                                }
                            }
                        }
                    });
                });
            }
            plugin::dialog::Content::Form { fields } => {
                egui::Grid::new("plugin_dialog_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    for (i, f) in fields.iter().enumerate() {
                        ui.label(egui::RichText::new(&f.label).color(t.text));
                        match st.values.get_mut(i) {
                            Some(FieldValue::Text(text)) if f.multiline => {
                                // Protocol 2.2: several lines (Enter is a new line), monospace for code,
                                // scrolling past its height; bounded like the plugin's value.
                                let mut edit = egui::TextEdit::multiline(text)
                                    .desired_width(460.0)
                                    .desired_rows(usize::from(f.rows))
                                    .char_limit(plugin::dialog::MAX_MULTILINE_TEXT);
                                if f.code {
                                    edit = edit.code_editor();
                                }
                                let label = f.label.clone();
                                let line_h = ui.text_style_height(&if f.code { egui::TextStyle::Monospace } else { egui::TextStyle::Body });
                                egui::ScrollArea::vertical()
                                    .id_salt(("plugin_dialog_multiline", i))
                                    .max_height(line_h * f32::from(f.rows) + 12.0)
                                    .show(ui, |ui| {
                                        ui.add(edit).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
                                    });
                            }
                            Some(FieldValue::Text(text)) => {
                                let edit = egui::TextEdit::singleline(text)
                                    .desired_width(280.0)
                                    .char_limit(plugin::dialog::MAX_FIELD_TEXT)
                                    .password(f.kind == plugin::dialog::FieldKind::Secret);
                                let label = f.label.clone();
                                ui.add(edit).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
                            }
                            Some(FieldValue::Check(b)) => {
                                ui.checkbox(b, "");
                            }
                            Some(FieldValue::Choice(c)) => {
                                let shown = f.options.iter().find(|(id, _)| id == c).map(|(_, l)| l.clone()).unwrap_or_default();
                                egui::ComboBox::from_id_salt(("plugin_dialog_choice", i)).selected_text(shown).width(280.0).show_ui(ui, |ui| {
                                    for (id, label) in &f.options {
                                        ui.selectable_value(c, id.clone(), label);
                                    }
                                });
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            }
            plugin::dialog::Content::Message => {}
        }
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has_command = st.dialog.command.is_some();
            if has_command && ui.button(lang.tr("Cancel")).clicked() {
                action = DialogAction::Close;
            }
            if ui.add_enabled(st.can_ok(), egui::Button::new(&st.dialog.ok_label)).clicked() {
                action = match st.ok_run() {
                    Some(run) => DialogAction::Run(run),
                    None => DialogAction::Close,
                };
            }
            for i in (0..st.dialog.buttons.len()).rev() {
                let label = st.dialog.buttons.get(i).map(|b| b.label.clone()).unwrap_or_default();
                if ui.button(label).clicked()
                    && let Some(run) = st.button_run(i)
                {
                    action = DialogAction::Run(run);
                }
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = DialogAction::Close;
        }
    });
    match action {
        DialogAction::Keep => app.plugin_dialog = Some(st),
        DialogAction::Close => {}
        DialogAction::Run(params) => run_command(app, params),
    }
}

/// Shows what plugins answered to document events: a message in the status bar, an error in the
/// info bar, a `host/message` in a message box. Keeps painting while events are on their way.
pub fn poll_events(app: &mut SheetApp, ctx: &egui::Context) {
    for o in plugin::events::take_outcomes() {
        let name = plugin::get(&o.plugin).map(|p| p.manifest.name.clone()).unwrap_or_else(|| o.plugin.clone());
        match o.kind {
            plugin::events::OutcomeKind::Message => app.status_message = Some((o.text, crate::now_ms())),
            plugin::events::OutcomeKind::Error => {
                app.notice = Some(crate::Notice { text: format!("{name}: {}", o.text), kind: crate::NoticeKind::Warning, action: None });
            }
            plugin::events::OutcomeKind::HostMessage => app.message = Some((name, o.text)),
        }
    }
    if plugin::events::pending() > 0 || plugin::invalidation_pending() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    // Plugins started from now on are told the interface's language.
    let tag = match app.ui.language {
        crate::i18n::Language::PtBr => "pt-BR",
        l => l.code(),
    };
    if plugin::host_language() != tag {
        plugin::set_host_language(tag);
    }
}

/// Draws the plugin groups on `tab`, each after a separator; small buttons stack three high.
pub fn ribbon_groups(app: &mut SheetApp, ui: &mut Ui, tab: &str, separate_first: bool) {
    for (i, (_group, buttons)) in groups(tab).into_iter().enumerate() {
        if separate_first || i > 0 {
            crate::ribbon::sep(ui);
        }
        let mut k = 0;
        while let Some(b) = buttons.get(k) {
            if !b.small {
                if labelled(big_button(ui, b.icon, &b.label, &b.tooltip, false), &b.label).clicked() {
                    run(app, b, json!({}));
                }
                k += 1;
                continue;
            }
            let column: Vec<&Item> = buttons.iter().skip(k).take_while(|b| b.small).take(3).collect();
            k += column.len();
            ui.vertical(|ui| {
                for b in column {
                    if labelled(small_button(ui, b.icon, &b.label, &b.tooltip, false), &b.label).clicked() {
                        run(app, b, json!({}));
                    }
                }
            });
        }
    }
}

/// Names the button for screen readers (and tests).
fn labelled(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp
}

/// The icon a manifest names (`"icon": "chart"`); case and `-`/`_` don't matter.
pub fn icon_named(name: &str) -> Option<Icon> {
    let key: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    Some(match key.as_str() {
        "script" | "plugin" => Icon::Script,
        "paste" => Icon::Paste,
        "copy" => Icon::Copy,
        "cut" => Icon::Cut,
        "insert" => Icon::Insert,
        "delete" => Icon::Delete,
        "format" => Icon::Format,
        "sum" => Icon::Sum,
        "function" => Icon::Function,
        "fx" => Icon::Fx,
        "calc" | "calculate" | "refresh" => Icon::Calc,
        "table" => Icon::Table,
        "pivottable" => Icon::PivotTable,
        "chart" => Icon::Chart,
        "chartbar" => Icon::ChartBar,
        "chartline" => Icon::ChartLine,
        "chartpie" => Icon::ChartPie,
        "chartscatter" => Icon::ChartScatter,
        "sparkline" => Icon::Sparkline,
        "picture" | "image" => Icon::Picture,
        "link" => Icon::Link,
        "comment" => Icon::Comment,
        "note" => Icon::Note,
        "filter" => Icon::Filter,
        "sort" | "sortasc" => Icon::SortAsc,
        "sortdesc" => Icon::SortDesc,
        "find" => Icon::Find,
        "search" => Icon::Search,
        "lock" => Icon::Lock,
        "settings" => Icon::Settings,
        "share" => Icon::Share,
        "sheet" => Icon::Sheet,
        "book" | "workbook" => Icon::Book,
        "folder" | "open" => Icon::OpenFolder,
        "save" => Icon::Save,
        "print" => Icon::Print,
        "calendar" | "date" => Icon::Calendar,
        "clock" | "time" => Icon::Clock,
        "info" => Icon::Info,
        "check" => Icon::Check,
        "plus" | "add" => Icon::Plus,
        "minus" => Icon::Minus,
        "pen" => Icon::Pen,
        "currency" | "money" => Icon::Currency,
        "percent" => Icon::Percent,
        "validation" => Icon::Validation,
        "name" => Icon::Name,
        "zoom" => Icon::Zoom,
        "more" => Icon::More,
        _ => return None,
    })
}

// ---------------------------------------------------------------- managing plugins

/// Installing a plugin from GitHub: the address typed, the confirmation step, and the download,
/// which runs on its own thread so the window keeps painting. Also the questions requirements
/// between add-ins raise (manifest 1.3): installing the add-ins an add-in needs, updating one
/// to a version an add-in that needs it doesn't accept, and turning off one others need.
#[derive(Default)]
pub struct Install {
    pub url: String,
    /// The repository waiting for the user's go-ahead (`owner/name`).
    pub confirming: Option<String>,
    pending: Option<std::sync::mpsc::Receiver<Result<Done, String>>>,
    /// The last install's outcome: `Ok(message)` or `Err(error)`.
    pub outcome: Option<Result<String, String>>,
    /// A download whose version an installed add-in that needs it doesn't accept: what to say.
    pub confirm_update: Option<(plugin::install::Fetched, String)>,
    /// The add-ins an add-in needs, offered for installing ("Install it too?").
    pub offer: Option<Offer>,
    /// Turning off or unloading an add-in others need: asking first.
    pub confirm_off: Option<ConfirmOff>,
}

/// The missing add-ins of `plugin` offered for installing, each with whether it's ticked
/// (required ones are, optional ones aren't).
pub struct Offer {
    pub plugin: String,
    pub name: String,
    pub items: Vec<(plugin::requires::Missing, bool)>,
}

/// Turning off (or unloading) `id`, which `dependents` need.
pub struct ConfirmOff {
    pub id: String,
    pub unload: bool,
    pub dependents: Vec<String>,
}

/// What a download thread hands back.
enum Done {
    /// Downloaded; the add-ins that need it don't accept its version (the messages).
    Conflicts(Box<plugin::install::Fetched>, Vec<String>),
    /// Unpacked into this folder (from this release).
    Unpacked(std::path::PathBuf),
    /// Required add-ins: per add-in, its folder and release, or why it failed.
    Requirements(Vec<(String, Result<(std::path::PathBuf, Option<String>), String>)>),
}

impl Install {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

/// Runs `job` on a thread; its outcome is picked up by [`poll_install`].
fn spawn(app: &mut SheetApp, job: impl FnOnce() -> Result<Done, String> + Send + 'static) {
    let st = &mut app.plugin_install;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("plugin-install".into()).spawn(move || {
            let _ = tx.send(job());
        });
        match spawned {
            Ok(_) => {
                st.pending = Some(rx);
                st.outcome = None;
            }
            Err(e) => st.outcome = Some(Err(format!("Couldn't start the download: {e}"))),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        drop(job);
        st.outcome = Some(Err("Plugins need the desktop app.".into()));
    }
}

/// Starts downloading the plugin at `url` (after the user confirmed).
pub fn start_install(app: &mut SheetApp, url: &str) {
    let url = url.trim().to_string();
    app.plugin_install.confirming = None;
    let Some(root) = plugin::install::install_dir() else {
        app.plugin_install.outcome = Some(Err("There's no plugins folder to install into (preferences are turned off).".into()));
        return;
    };
    spawn(app, move || {
        let fetched = plugin::install::fetch(&url, None)?;
        let conflicts = plugin::conflicts(&fetched.manifest);
        if !conflicts.is_empty() {
            return Ok(Done::Conflicts(Box::new(fetched), conflicts.into_iter().map(|c| c.message).collect()));
        }
        fetched.unpack(&root).map(Done::Unpacked)
    });
}

/// Installs the ticked add-ins of the offer: for each, the newest release in its range.
pub fn install_offer(app: &mut SheetApp) {
    let Some(offer) = app.plugin_install.offer.take() else { return };
    let Some(root) = plugin::install::install_dir() else {
        app.plugin_install.outcome = Some(Err("There's no plugins folder to install into (preferences are turned off).".into()));
        return;
    };
    let wanted: Vec<plugin::requires::Missing> = offer.items.into_iter().filter(|(_, on)| *on).map(|(m, _)| m).collect();
    if wanted.is_empty() {
        return;
    }
    spawn(app, move || {
        let done = wanted
            .into_iter()
            .map(|m| {
                let r = (|| -> Result<(std::path::PathBuf, Option<String>), String> {
                    let github = m.github.as_deref().ok_or_else(|| format!("{} has no GitHub repository to install from", m.name))?;
                    let range = plugin::version::Range::parse(&m.version)?;
                    let fetched = plugin::install::fetch(github, Some(&range))?;
                    if fetched.manifest.id != m.id {
                        return Err(format!("github.com/{github} holds {}, not {}", fetched.manifest.id, m.id));
                    }
                    let fits = plugin::version::Version::parse(&fetched.manifest.version).map(|v| range.matches(&v)).unwrap_or(range.is_any());
                    if !fits {
                        return Err(format!("{} {} from github.com/{github} isn't in {range}", m.name, fetched.manifest.version));
                    }
                    if let Some(c) = plugin::conflicts(&fetched.manifest).first() {
                        return Err(c.message.clone());
                    }
                    Ok((fetched.unpack(&root)?, fetched.release.clone()))
                })();
                (m.id, r)
            })
            .collect();
        Ok(Done::Requirements(done))
    });
}

/// Offers the missing or out-of-range add-ins `id` needs that name a GitHub repository; does
/// nothing when there are none.
pub fn offer_missing(app: &mut SheetApp, id: &str) {
    let items: Vec<(plugin::requires::Missing, bool)> =
        plugin::missing(id).into_iter().filter(|m| m.github.is_some()).map(|m| (m.clone(), !m.optional)).collect();
    if items.is_empty() {
        return;
    }
    let name = plugin::get(id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.to_string());
    app.plugin_install.offer = Some(Offer { plugin: id.to_string(), name, items });
}

/// Whether `id` is blocked by required add-ins that Install Missing can install.
pub fn can_install_missing(id: &str) -> bool {
    plugin::missing(id).iter().any(|m| !m.optional && m.github.is_some())
}

/// Loads a plugin unpacked by a download thread (its version was checked, or the user agreed).
fn load_unpacked(app: &mut SheetApp, dir: &std::path::Path) -> Result<Json, String> {
    app.run("plugins.load", json!({"path": dir, "force": true}))
}

/// Picks up a finished download and loads the plugin (every frame; nothing to do most of them).
pub fn poll_install(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(rx) = &app.plugin_install.pending else { return };
    let result = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the download stopped unexpectedly".into()),
    };
    app.plugin_install.pending = None;
    match result {
        Err(e) => app.plugin_install.outcome = Some(Err(e)),
        Ok(Done::Conflicts(fetched, why)) => {
            let text = format!("{}. Install {} {} anyway?", why.join("; "), fetched.manifest.name, fetched.manifest.version);
            app.plugin_install.confirm_update = Some((*fetched, text));
        }
        Ok(Done::Unpacked(dir)) => {
            let outcome = load_unpacked(app, &dir).map(|info| {
                let name = info.get("name").and_then(Json::as_str).unwrap_or("The plugin");
                let version = info.get("version").and_then(Json::as_str).unwrap_or("");
                (info.get("id").and_then(Json::as_str).unwrap_or("").to_string(), format!("Installed {name} {version}").trim_end().to_string())
            });
            if let Ok((id, _)) = &outcome {
                app.plugin_install.url.clear();
                let id = id.clone();
                offer_missing(app, &id);
            }
            app.plugin_install.outcome = Some(outcome.map(|(_, m)| m));
        }
        Ok(Done::Requirements(list)) => {
            let (mut done, mut failed, mut ids) = (Vec::new(), Vec::new(), Vec::new());
            for (id, r) in list {
                match r.and_then(|(dir, release)| load_unpacked(app, &dir).map(|info| (info, release))) {
                    Ok((info, release)) => {
                        let name = info.get("name").and_then(Json::as_str).unwrap_or(&id).to_string();
                        let version = info.get("version").and_then(Json::as_str).unwrap_or("").to_string();
                        done.push(match release {
                            Some(tag) => format!("{name} {version} (release {tag})"),
                            None => format!("{name} {version}").trim_end().to_string(),
                        });
                        ids.push(id);
                    }
                    Err(e) => failed.push(e),
                }
            }
            // What the new ones need in turn.
            for id in ids {
                if app.plugin_install.offer.is_none() {
                    offer_missing(app, &id);
                }
            }
            app.plugin_install.outcome = Some(if failed.is_empty() {
                Ok(format!("Installed {}", done.join(", ")))
            } else if done.is_empty() {
                Err(failed.join("; "))
            } else {
                Err(format!("Installed {}; {}", done.join(", "), failed.join("; ")))
            });
        }
    }
}

/// "Install from GitHub": an address field, a confirmation naming the repository, progress and
/// the outcome; then the questions an install can raise (an update the add-ins that need it
/// don't accept; the add-ins it needs).
pub fn install_row(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    ui.horizontal(|ui| {
        ui.label(lang.tr("Install from GitHub:"));
        let busy = app.plugin_install.busy();
        let field = egui::TextEdit::singleline(&mut app.plugin_install.url).hint_text("https://github.com/owner/repository").desired_width(340.0);
        let resp = ui.add_enabled(!busy, field);
        let go = ui.add_enabled(!busy && !app.plugin_install.url.trim().is_empty(), egui::Button::new(lang.tr("Install"))).clicked()
            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !app.plugin_install.url.trim().is_empty());
        if go {
            match plugin::install::parse_github_url(&app.plugin_install.url) {
                Ok(r) => app.plugin_install.confirming = Some(format!("{}/{}", r.owner, r.name)),
                Err(e) => app.plugin_install.outcome = Some(Err(e)),
            }
        }
        if busy {
            ui.spinner();
            ui.label(lang.tr("Installing…"));
        }
    });
    if let Some(repo) = app.plugin_install.confirming.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "{} github.com/{repo}",
                lang.tr("A plugin is a program that runs with your permissions. Install it only if you trust its author. Install and run")
            ));
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Install")).clicked() {
                    let url = app.plugin_install.url.clone();
                    start_install(app, &url);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    app.plugin_install.confirming = None;
                }
            });
        });
    }
    if let Some((_, text)) = &app.plugin_install.confirm_update {
        let text = text.clone();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.add(egui::Label::new(text).wrap());
            ui.weak(lang.tr("The add-ins that need the installed version are blocked until it suits them again."));
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Install Anyway")).clicked()
                    && let Some((fetched, _)) = app.plugin_install.confirm_update.take()
                {
                    match plugin::install::install_dir() {
                        Some(root) => spawn(app, move || fetched.unpack(&root).map(Done::Unpacked)),
                        None => app.plugin_install.outcome = Some(Err("There's no plugins folder to install into.".into())),
                    }
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    app.plugin_install.confirm_update = None;
                }
            });
        });
    }
    offer_frame(app, ui);
    match &app.plugin_install.outcome {
        Some(Ok(m)) => {
            ui.label(egui::RichText::new(m).color(crate::theme::Tokens::get(ui.ctx()).accent));
        }
        Some(Err(e)) => {
            ui.label(egui::RichText::new(e).color(ui.visuals().error_fg_color));
        }
        None => {}
    }
}

/// "Reports needs craft-vba ^0.1.4 (Runs the report macros). Install it too?", one tick box per
/// add-in (optional ones unticked), and Install / Not Now.
fn offer_frame(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let busy = app.plugin_install.busy();
    let Some(offer) = app.plugin_install.offer.as_mut() else { return };
    let mut install = false;
    let mut dismiss = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        for (m, on) in offer.items.iter_mut() {
            let why = if m.why.trim().is_empty() { String::new() } else { format!(" ({})", m.why.trim()) };
            let what = match &m.installed {
                Some(v) => format!("{} {}{why}; {} {v} {}", m.name, m.version, lang.tr("version"), lang.tr("is installed")),
                None => format!("{} {}{why}", m.name, m.version),
            };
            let text = if m.optional {
                format!("{} {} {what}. {}", offer.name, lang.tr("can use"), lang.tr("Install it too?"))
            } else {
                format!("{} {} {what}. {}", offer.name, lang.tr("needs"), lang.tr("Install it too?"))
            };
            ui.checkbox(on, text);
            if let Some(g) = &m.github {
                ui.indent(("offer", &m.id), |ui| {
                    ui.weak(format!("github.com/{g} · {}", lang.tr("the newest release in range")));
                });
            }
        }
        ui.weak(lang.tr("Add-ins are programs that run with your permissions. Install them only if you trust their authors."));
        ui.horizontal(|ui| {
            install = ui.add_enabled(!busy && offer.items.iter().any(|(_, on)| *on), egui::Button::new(lang.tr("Install Selected"))).clicked();
            dismiss = ui.button(lang.tr("Not Now")).clicked();
        });
    });
    if install {
        install_offer(app);
    } else if dismiss {
        app.plugin_install.offer = None;
    }
}

/// Turns `id` off (or unloads it), and first the add-ins that need it when the user agreed.
fn turn_off(app: &mut SheetApp, id: &str, unload: bool, dependents: &[String]) {
    // The ones furthest up the chain first.
    for d in dependents.iter().rev() {
        app.run_or_alert("plugins.setEnabled", json!({"id": d, "enabled": false}));
    }
    if unload {
        app.run_or_alert("plugins.unload", json!({"id": id}));
    } else {
        app.run_or_alert("plugins.setEnabled", json!({"id": id, "enabled": false}));
    }
}

/// Deactivate or Unload: asks first when other add-ins need it.
fn ask_turn_off(app: &mut SheetApp, id: &str, unload: bool) {
    let dependents = plugin::dependents(id);
    if dependents.is_empty() {
        turn_off(app, id, unload, &[]);
    } else {
        app.plugin_install.confirm_off = Some(ConfirmOff { id: id.to_string(), unload, dependents });
    }
}

fn names(ids: &[String]) -> Vec<String> {
    ids.iter().map(|d| plugin::get(d).map(|p| p.manifest.name.clone()).unwrap_or_else(|| d.clone())).collect()
}

/// One plugin as a full-width card: name and version, then status, author, id and where it came
/// from, the description across the width, and Documentation / Activate or Deactivate / Unload on
/// the right. A blocked plugin says why, with Install Missing when that can help; turning off one
/// that others need asks first.
pub fn plugin_card(app: &mut SheetApp, ui: &mut Ui, p: &plugin::Plugin, unload: bool) {
    let lang = app.ui.language;
    let m = &p.manifest;
    let on = p.enabled();
    egui::Frame::group(ui.style()).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(format!("{} {}", m.name, m.version).trim_end().to_string()).strong());
                let status = p.status();
                let mut about = vec![match status.as_str() {
                    "off" => lang.tr("Inactive").to_string(),
                    s if s.starts_with("blocked") => lang.tr("Blocked").to_string(),
                    s if s.starts_with("failed") => s.to_string(),
                    _ => lang.tr("Active").to_string(),
                }];
                if !m.author.is_empty() {
                    about.push(m.author.clone());
                }
                about.push(m.id.clone());
                if let Some(repo) = p.source.as_ref().and_then(|s| s.get("repository")).and_then(Json::as_str) {
                    about.push(repo.trim_start_matches("https://").to_string());
                }
                ui.weak(about.join(" · "));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if unload && ui.button(lang.tr("Unload")).on_hover_text(lang.tr("Until the next start")).clicked() {
                    ask_turn_off(app, &m.id, true);
                }
                if ui.button(lang.tr(if on { "Deactivate" } else { "Activate" })).clicked() {
                    if on {
                        ask_turn_off(app, &m.id, false);
                    } else {
                        app.run_or_alert("plugins.setEnabled", json!({"id": m.id, "enabled": true}));
                    }
                }
                if on
                    && p.is_blocked()
                    && can_install_missing(&m.id)
                    && ui.add_enabled(!app.plugin_install.busy(), egui::Button::new(lang.tr("Install Missing"))).clicked()
                {
                    offer_missing(app, &m.id);
                }
                if !m.homepage.trim().is_empty() && ui.link(lang.tr("Documentation")).on_hover_text(m.homepage.trim()).clicked() {
                    app.run_or_alert("plugins.openHomepage", json!({"id": m.id}));
                }
            });
        });
        if on {
            for b in p.blocked_by() {
                ui.add(egui::Label::new(egui::RichText::new(&b.message).color(ui.visuals().warn_fg_color)).wrap());
            }
        }
        if !m.description.is_empty() {
            ui.add(egui::Label::new(&m.description).wrap());
        }
        let asking = app.plugin_install.confirm_off.as_ref().filter(|c| c.id == m.id).map(|c| (c.unload, c.dependents.clone()));
        if let Some((unloading, dependents)) = asking {
            let who = names(&dependents).join(", ");
            let verb = if dependents.len() == 1 { lang.tr("needs") } else { lang.tr("need") };
            let question = if dependents.len() == 1 { lang.tr("Turn both off?") } else { lang.tr("Turn them all off?") };
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.add(egui::Label::new(format!("{who} {verb} {}. {question}", m.name)).wrap());
                ui.horizontal(|ui| {
                    let label = if dependents.len() == 1 { "Turn Both Off" } else { "Turn All Off" };
                    if ui.button(lang.tr(label)).clicked() {
                        app.plugin_install.confirm_off = None;
                        turn_off(app, &m.id, unloading, &dependents);
                    }
                    if ui.button(lang.tr("Cancel")).clicked() {
                        app.plugin_install.confirm_off = None;
                    }
                });
            });
        }
    });
}
