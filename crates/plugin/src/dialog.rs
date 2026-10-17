//! Protocol 1.2: the dialogs a plugin command's result may ask the host to show (`dialog`), and
//! the local file it may ask the host to open (`open`).
//!
//! A plugin is hostile input like any file: the dialog is validated here, everything over a
//! limit is truncated, and a malformed dialog is an error (the command fails with a message),
//! never a crash. The app draws the dialog with its own widgets; what the user does runs another
//! plugin command ([`Dialog::ok_params`], [`Dialog::button_params`]).

use serde_json::{Map, Value as Json, json};

/// Most list items.
pub const MAX_ITEMS: usize = 10_000;
/// Longest list item id.
pub const MAX_ITEM_ID: usize = 4_096;
/// Longest list item label and detail, and form field label.
pub const MAX_ITEM_TEXT: usize = 256;
/// Most form fields.
pub const MAX_FIELDS: usize = 16;
/// Longest prompt (a message dialog's text).
pub const MAX_PROMPT: usize = 4_000;
/// Longest dialog title, and `open`'s `label`.
pub const MAX_TITLE: usize = 200;
/// Most extra buttons.
pub const MAX_BUTTONS: usize = 4;
/// Longest button label (as the manifest's labels).
pub const MAX_BUTTON_LABEL: usize = 64;
/// Most options in a choice field.
pub const MAX_OPTIONS: usize = 1_000;
/// Longest text field value (a cell's text limit).
pub const MAX_FIELD_TEXT: usize = 32_767;
/// Longest multi-line text field value (protocol 2.2): a script of a few thousand lines.
pub const MAX_MULTILINE_TEXT: usize = 200_000;
/// A multi-line field's height in lines: the default, the fewest and the most.
pub const DEFAULT_ROWS: u8 = 8;
pub const MIN_ROWS: u8 = 2;
pub const MAX_ROWS: u8 = 30;
/// A list shows a filter box when it has more items than this.
pub const FILTER_AFTER: usize = 20;

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Items to pick from; `multi` allows several.
    List { items: Vec<Item>, multi: bool },
    /// Fields to fill in.
    Form { fields: Vec<Field> },
    /// The prompt alone.
    Message,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub detail: String,
    /// An icon name from the app's set (as for ribbon buttons); empty: a generic file.
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// `text`/`secret`: a string; `number`: a number or null; `checkbox`: a bool; `choice`: an
    /// option id.
    pub value: Json,
    /// A choice field's options: `(id, label)`.
    pub options: Vec<(String, String)>,
    /// A text field edited on several lines (protocol 2.2: `"multiline": true`); Enter starts a
    /// new line.
    pub multiline: bool,
    /// A multi-line field holding code (`"code": true`): a monospace font, Tab indents.
    pub code: bool,
    /// A multi-line field's height in lines (`rows`, 2–30, default 8).
    pub rows: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Secret,
    Number,
    Checkbox,
    Choice,
}

impl FieldKind {
    pub fn name(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Secret => "secret",
            FieldKind::Number => "number",
            FieldKind::Checkbox => "checkbox",
            FieldKind::Choice => "choice",
        }
    }
}

/// An extra button: runs `command` with `params` (plus the selection or the fields).
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    pub command: String,
    pub params: Map<String, Json>,
}

/// A dialog a plugin command asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub title: String,
    pub prompt: String,
    pub content: Content,
    /// The command OK (or a double-click on a list item) runs; a message dialog may have none.
    pub command: Option<String>,
    pub params: Map<String, Json>,
    pub ok_label: String,
    pub buttons: Vec<Button>,
}

/// `s`'s first `n` characters.
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn opt_str<'a>(v: &'a Json, key: &str, what: &str) -> Result<Option<&'a str>, String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("{what}: `{key}` must be text")),
    }
}

fn params_of(v: &Json, what: &str) -> Result<Map<String, Json>, String> {
    match v.get("params") {
        None | Some(Json::Null) => Ok(Map::new()),
        Some(Json::Object(m)) => Ok(m.clone()),
        Some(_) => Err(format!("{what}: `params` must be an object")),
    }
}

fn command_of(v: &Json, what: &str) -> Result<Option<String>, String> {
    match opt_str(v, "command", what)? {
        None => Ok(None),
        Some(c) if crate::valid_id(c) => Ok(Some(c.to_string())),
        Some(c) => Err(format!("{what}: `{}` isn't a command id", cut(c, 64))),
    }
}

impl Dialog {
    /// Validates the `dialog` of a command's result. Text over a limit is truncated, lists and
    /// buttons beyond theirs are dropped; a wrong shape is an error.
    pub fn parse(v: &Json) -> Result<Dialog, String> {
        const WHAT: &str = "dialog";
        if !v.is_object() {
            return Err("`dialog` must be an object".into());
        }
        let title = cut(opt_str(v, "title", WHAT)?.unwrap_or(""), MAX_TITLE);
        let prompt = cut(opt_str(v, "prompt", WHAT)?.unwrap_or(""), MAX_PROMPT);
        let kind = opt_str(v, "kind", WHAT)?.ok_or("dialog: `kind` is missing (list, form or message)")?;
        let content = match kind {
            "list" => {
                let raw = match v.get("items") {
                    None | Some(Json::Null) => &[][..],
                    Some(Json::Array(a)) => &a[..],
                    Some(_) => return Err("dialog: `items` must be an array".into()),
                };
                let mut items = Vec::with_capacity(raw.len().min(MAX_ITEMS));
                for (i, it) in raw.iter().take(MAX_ITEMS).enumerate() {
                    let what = format!("dialog item {}", i + 1);
                    if !it.is_object() {
                        return Err(format!("{what} must be an object"));
                    }
                    let id = match it.get("id") {
                        Some(Json::String(s)) => cut(s, MAX_ITEM_ID),
                        Some(Json::Number(n)) => n.to_string(),
                        _ => return Err(format!("{what}: `id` is missing")),
                    };
                    let label = cut(opt_str(it, "label", &what)?.unwrap_or(&id), MAX_ITEM_TEXT);
                    let detail = cut(opt_str(it, "detail", &what)?.unwrap_or(""), MAX_ITEM_TEXT);
                    let icon = cut(opt_str(it, "icon", &what)?.unwrap_or(""), 64);
                    items.push(Item { id, label, detail, icon });
                }
                let multi = match v.get("multi") {
                    None | Some(Json::Null) => false,
                    Some(Json::Bool(b)) => *b,
                    Some(_) => return Err("dialog: `multi` must be true or false".into()),
                };
                Content::List { items, multi }
            }
            "form" => {
                let raw = match v.get("fields") {
                    Some(Json::Array(a)) => &a[..],
                    None | Some(Json::Null) => &[][..],
                    Some(_) => return Err("dialog: `fields` must be an array".into()),
                };
                let mut fields = Vec::new();
                for (i, f) in raw.iter().take(MAX_FIELDS).enumerate() {
                    fields.push(Field::parse(f, i)?);
                }
                Content::Form { fields }
            }
            "message" => Content::Message,
            other => return Err(format!("dialog: unknown kind `{}` (list, form or message)", cut(other, 32))),
        };
        let command = command_of(v, WHAT)?;
        if command.is_none() && !matches!(content, Content::Message) {
            return Err("dialog: `command` is missing (the command OK runs)".into());
        }
        let ok_label = cut(opt_str(v, "okLabel", WHAT)?.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("OK"), MAX_BUTTON_LABEL);
        let mut buttons = Vec::new();
        match v.get("buttons") {
            None | Some(Json::Null) => {}
            Some(Json::Array(a)) => {
                for (i, b) in a.iter().take(MAX_BUTTONS).enumerate() {
                    let what = format!("dialog button {}", i + 1);
                    if !b.is_object() {
                        return Err(format!("{what} must be an object"));
                    }
                    let label = opt_str(b, "label", &what)?.map(str::trim).filter(|s| !s.is_empty()).ok_or(format!("{what}: `label` is missing"))?;
                    let command = command_of(b, &what)?.ok_or(format!("{what}: `command` is missing"))?;
                    buttons.push(Button { label: cut(label, MAX_BUTTON_LABEL), command, params: params_of(b, &what)? });
                }
            }
            Some(_) => return Err("dialog: `buttons` must be an array".into()),
        }
        Ok(Dialog { title, prompt, content, command, params: params_of(v, WHAT)?, ok_label, buttons })
    }

    /// Every command the dialog can run (OK's and the buttons').
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.command.as_deref().into_iter().chain(self.buttons.iter().map(|b| b.command.as_str()))
    }

    /// The dialog as validated, in the protocol's shape (what `plugins.run` returns).
    pub fn to_json(&self) -> Json {
        let mut out = json!({
            "title": self.title,
            "prompt": self.prompt,
            "params": self.params,
            "okLabel": self.ok_label,
            "buttons": self.buttons.iter().map(|b| json!({"label": b.label, "command": b.command, "params": b.params})).collect::<Vec<_>>(),
        });
        if let Some(c) = &self.command {
            out["command"] = json!(c);
        }
        match &self.content {
            Content::List { items, multi } => {
                out["kind"] = json!("list");
                out["multi"] = json!(multi);
                out["items"] = items.iter().map(|i| json!({"id": i.id, "label": i.label, "detail": i.detail, "icon": i.icon})).collect();
            }
            Content::Form { fields } => {
                out["kind"] = json!("form");
                out["fields"] = fields
                    .iter()
                    .map(|f| {
                        let mut j = json!({"id": f.id, "label": f.label, "kind": f.kind.name(), "value": f.value});
                        if f.multiline {
                            j["multiline"] = json!(true);
                            j["code"] = json!(f.code);
                            j["rows"] = json!(f.rows);
                        }
                        if f.kind == FieldKind::Choice {
                            j["options"] = f.options.iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
                        }
                        j
                    })
                    .collect();
            }
            Content::Message => out["kind"] = json!("message"),
        }
        out
    }

    /// What OK runs its command with: the dialog's `params` plus, for a list, `id` (or `ids` when
    /// `multi`) and, for a form, `fields`. `selection` is the chosen items' ids; `fields` the
    /// form's values by field id.
    pub fn ok_params(&self, selection: &[String], fields: &Map<String, Json>) -> Json {
        Self::with_input(&self.params, &self.content, selection, fields, false)
    }

    /// What extra button `i` runs its command with: its own `params` plus the selection (only when
    /// there is one) or the fields.
    pub fn button_params(&self, i: usize, selection: &[String], fields: &Map<String, Json>) -> Option<(String, Json)> {
        let b = self.buttons.get(i)?;
        Some((b.command.clone(), Self::with_input(&b.params, &self.content, selection, fields, true)))
    }

    fn with_input(params: &Map<String, Json>, content: &Content, selection: &[String], fields: &Map<String, Json>, optional: bool) -> Json {
        let mut p = params.clone();
        match content {
            Content::List { multi: true, .. } => {
                if !(optional && selection.is_empty()) {
                    p.insert("ids".into(), json!(selection));
                }
            }
            Content::List { multi: false, .. } => {
                if let Some(id) = selection.first() {
                    p.insert("id".into(), json!(id));
                }
            }
            Content::Form { .. } => {
                p.insert("fields".into(), Json::Object(fields.clone()));
            }
            Content::Message => {}
        }
        Json::Object(p)
    }
}

impl Field {
    fn parse(f: &Json, i: usize) -> Result<Field, String> {
        let what = format!("dialog field {}", i + 1);
        if !f.is_object() {
            return Err(format!("{what} must be an object"));
        }
        let id = opt_str(f, "id", &what)?.filter(|s| !s.is_empty()).ok_or(format!("{what}: `id` is missing"))?;
        let id = cut(id, MAX_ITEM_TEXT);
        let label = cut(opt_str(f, "label", &what)?.unwrap_or(&id), MAX_ITEM_TEXT);
        let kind = match opt_str(f, "kind", &what)?.unwrap_or("text") {
            "text" => FieldKind::Text,
            "secret" => FieldKind::Secret,
            "number" => FieldKind::Number,
            "checkbox" => FieldKind::Checkbox,
            "choice" => FieldKind::Choice,
            other => return Err(format!("{what}: unknown kind `{}` (text, secret, number, checkbox or choice)", cut(other, 32))),
        };
        let flag = |key: &str| match f.get(key) {
            None | Some(Json::Null) => Ok(false),
            Some(Json::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{what}: `{key}` must be true or false")),
        };
        // Multi-line and code apply to text fields; others ignore them.
        let multiline = kind == FieldKind::Text && (flag("multiline")? || flag("code")?);
        let code = multiline && flag("code")?;
        let rows = match f.get("rows") {
            None | Some(Json::Null) => DEFAULT_ROWS,
            Some(Json::Number(n)) => {
                n.as_f64().filter(|x| x.is_finite()).map_or(DEFAULT_ROWS, |x| x.round().clamp(MIN_ROWS as f64, MAX_ROWS as f64) as u8)
            }
            Some(_) => return Err(format!("{what}: `rows` must be a number")),
        };
        let max_text = if multiline { MAX_MULTILINE_TEXT } else { MAX_FIELD_TEXT };
        let raw = f.get("value").cloned().unwrap_or(Json::Null);
        let mut options = Vec::new();
        if kind == FieldKind::Choice {
            let list = match f.get("options") {
                Some(Json::Array(a)) => &a[..],
                None | Some(Json::Null) => &[][..],
                Some(_) => return Err(format!("{what}: `options` must be an array")),
            };
            for o in list.iter().take(MAX_OPTIONS) {
                let oid = match o.get("id") {
                    Some(Json::String(s)) => cut(s, MAX_ITEM_TEXT),
                    Some(Json::Number(n)) => n.to_string(),
                    _ => return Err(format!("{what}: each option needs an `id`")),
                };
                let olabel = cut(opt_str(o, "label", &what)?.unwrap_or(&oid), MAX_ITEM_TEXT);
                options.push((oid, olabel));
            }
        }
        let value = match kind {
            FieldKind::Text | FieldKind::Secret => match raw {
                Json::String(s) => Json::String(cut(&s, max_text)),
                Json::Null => json!(""),
                Json::Number(n) => json!(n.to_string()),
                Json::Bool(b) => json!(b.to_string()),
                _ => return Err(format!("{what}: `value` must be text")),
            },
            FieldKind::Number => match raw {
                Json::Number(n) => Json::Number(n),
                Json::Null => Json::Null,
                Json::String(s) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map_or(Json::Null, |x| json!(x)),
                _ => return Err(format!("{what}: `value` must be a number")),
            },
            FieldKind::Checkbox => match raw {
                Json::Bool(b) => Json::Bool(b),
                Json::Null => Json::Bool(false),
                _ => return Err(format!("{what}: `value` must be true or false")),
            },
            FieldKind::Choice => {
                let want = match &raw {
                    Json::String(s) => Some(s.clone()),
                    Json::Number(n) => Some(n.to_string()),
                    _ => None,
                };
                match want.filter(|w| options.iter().any(|(id, _)| id == w)).or_else(|| options.first().map(|(id, _)| id.clone())) {
                    Some(id) => json!(id),
                    None => Json::Null,
                }
            }
        };
        Ok(Field { id, label, kind, value, options, multiline, code, rows })
    }
}

/// `open` in a command's result: a local file the app opens as File › Open would.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub path: String,
    /// Shown where the app shows the document's location; at most 200 characters.
    pub label: Option<String>,
    /// Open read-only where the app supports it (GridCraft doesn't yet: ignored).
    pub read_only: bool,
}

impl OpenRequest {
    /// Validates the shape of `open` (the app checks the file itself: absolute, a regular file it
    /// opens).
    pub fn parse(v: &Json) -> Result<OpenRequest, String> {
        const WHAT: &str = "open";
        if !v.is_object() {
            return Err("`open` must be an object".into());
        }
        let path = opt_str(v, "path", WHAT)?.filter(|p| !p.trim().is_empty()).ok_or("open: `path` is missing")?;
        if path.len() > 32_768 || path.contains('\0') {
            return Err("open: `path` isn't a usable file path".into());
        }
        let label = opt_str(v, "label", WHAT)?.map(str::trim).filter(|s| !s.is_empty()).map(|s| cut(s, MAX_TITLE));
        let read_only = match v.get("readOnly") {
            None | Some(Json::Null) => false,
            Some(Json::Bool(b)) => *b,
            Some(_) => return Err("open: `readOnly` must be true or false".into()),
        };
        Ok(OpenRequest { path: path.to_string(), label, read_only })
    }

    pub fn to_json(&self) -> Json {
        let mut out = json!({"path": self.path, "readOnly": self.read_only});
        if let Some(l) = &self.label {
            out["label"] = json!(l);
        }
        out
    }
}
