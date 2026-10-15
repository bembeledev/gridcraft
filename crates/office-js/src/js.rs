//! The JavaScript side: the `Office`, `Excel` and `OfficeExtension` globals, the proxy classes,
//! and the transport to the host.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect};
use serde_json::{Map, Value as Json, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

use crate::core::{Ctx, OfficeError, Path, child, is_set_supported, load_props, method, prop};

/// A synchronous transport (tests wire the engine in-process).
type SyncTransport = Rc<dyn Fn(String) -> String>;

thread_local! {
    static TRANSPORT: RefCell<Option<SyncTransport>> = const { RefCell::new(None) };
    static WAITING: RefCell<HashMap<u64, Function>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static SETTINGS: RefCell<Map<String, Json>> = RefCell::new(Map::new());
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static READY: RefCell<Option<Promise>> = const { RefCell::new(None) };
    /// Whether the library is ready (`Office.initialize` assigned later is called at once).
    static IS_READY: Cell<bool> = const { Cell::new(false) };
    static INITIALIZE: RefCell<Option<Function>> = const { RefCell::new(None) };
    /// `Office.actions.associate(name, fn)`.
    static ACTIONS: RefCell<HashMap<String, Function>> = RefCell::new(HashMap::new());
    /// `Office.context.document.addHandlerAsync(DocumentSelectionChanged, fn)`.
    static SELECTION_HANDLERS: RefCell<Vec<Function>> = const { RefCell::new(Vec::new()) };
    /// Worksheets added without a name so far (their tokens).
    static ADDED: Cell<u64> = const { Cell::new(0) };
}

/// Most actions and handlers kept.
const MAX_REGISTERED: usize = 1000;

// ---------------------------------------------------------------- values and errors

pub(crate) fn to_js(v: &Json) -> JsValue {
    match v {
        Json::Null => JsValue::NULL,
        _ => js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL),
    }
}

pub(crate) fn from_js(v: &JsValue) -> Json {
    if v.is_undefined() || v.is_null() {
        return Json::Null;
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Json::Null)
}

/// An `OfficeExtension.Error`: an `Error` named `RichApi.Error` with `code` and `debugInfo`.
fn js_error(e: &OfficeError) -> JsValue {
    let err = js_sys::Error::new(&e.message);
    err.set_name("RichApi.Error");
    let _ = Reflect::set(&err, &"code".into(), &e.code.as_str().into());
    let debug = to_js(&json!({"code": e.code, "message": e.message, "errorLocation": e.action}));
    let _ = Reflect::set(&err, &"debugInfo".into(), &debug);
    err.into()
}

fn not_implemented(what: &str) -> JsValue {
    js_error(&OfficeError::not_implemented(what))
}

// ---------------------------------------------------------------- the transport

/// Sends one batch of actions and waits for the host's answer (`{"results"}` or `{"error"}`).
async fn send(actions: Vec<Json>) -> Result<Json, OfficeError> {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    let request = json!({"id": id, "actions": actions}).to_string();
    if let Some(t) = TRANSPORT.with(|t| t.borrow().clone()) {
        return serde_json::from_str(&t(request)).map_err(|e| OfficeError::new("GeneralException", format!("the host's answer isn't JSON: {e}")));
    }
    let global = js_sys::global();
    let host = Reflect::get(&global, &"__craftOffice".into()).unwrap_or(JsValue::UNDEFINED);
    let post = Reflect::get(&host, &"post".into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
    let Some(post) = post else { return Err(OfficeError::new("GeneralException", "this page isn't running in GridCraft")) };
    let answer = Promise::new(&mut |resolve, _reject| {
        WAITING.with(|w| w.borrow_mut().insert(id, resolve));
    });
    post.call1(&host, &request.into()).map_err(|_| OfficeError::new("GeneralException", "GridCraft didn't take the request"))?;
    let reply = JsFuture::from(answer).await.map_err(|_| OfficeError::new("GeneralException", "the request was dropped"))?;
    let text = reply.as_string().unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| OfficeError::new("GeneralException", format!("the host's answer isn't JSON: {e}")))
}

/// `globalThis.__craftOfficeReply(json)`: the host's answer to a request, by its `id`.
fn reply(text: String) {
    let id = serde_json::from_str::<Json>(&text).ok().and_then(|v| v.get("id").and_then(Json::as_u64));
    if let Some(id) = id
        && let Some(resolve) = WAITING.with(|w| w.borrow_mut().remove(&id))
    {
        let _ = resolve.call1(&JsValue::UNDEFINED, &text.into());
    }
}

// ---------------------------------------------------------------- proxies

/// A context and a path: what every proxy object is.
#[derive(Clone)]
struct P {
    ctx: Rc<RefCell<Ctx>>,
    path: Path,
}

impl P {
    fn to(&self, step: Json) -> P {
        P { ctx: self.ctx.clone(), path: child(&self.path, step) }
    }
    fn get(&self, name: &str) -> Result<JsValue, JsValue> {
        self.ctx.borrow().get(&self.path, name).map(to_js).ok_or_else(|| js_error(&OfficeError::not_loaded(name)))
    }
    fn set(&self, name: &str, v: &JsValue) {
        self.ctx.borrow_mut().set(&self.path, name, from_js(v));
    }
    fn load(&self, arg: &JsValue) {
        self.ctx.borrow_mut().load(&self.path, &load_props(&from_js(arg)));
    }
    fn call(&self, name: &str, args: Vec<Json>) {
        self.ctx.borrow_mut().call(&self.path, name, args);
    }
}

fn num(v: f64) -> Json {
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 { json!(v as i64) } else { json!(v) }
}

/// The own properties of a plain object, as `(name, value)`.
fn entries(props: &JsValue) -> Vec<(String, JsValue)> {
    let Ok(o) = props.clone().dyn_into::<Object>() else { return Vec::new() };
    Object::entries(&o)
        .iter()
        .filter_map(|e| {
            let pair: Array = e.dyn_into().ok()?;
            Some((pair.get(0).as_string()?, pair.get(1)))
        })
        .collect()
}

/// `proxy.set({...})`: each of `props` as if assigned, `nested` ones into the object they name
/// (`range.set({format: {font: {bold: true}}})`).
fn set_all(props: &JsValue, scalar: &[&str], p: &P, mut nested: impl FnMut(&str, &JsValue) -> Option<Result<(), JsValue>>) -> Result<(), JsValue> {
    if !props.is_object() {
        return Err(js_error(&OfficeError::new("InvalidArgument", "set() takes an object of properties")));
    }
    for (k, v) in entries(props) {
        if scalar.contains(&k.as_str()) {
            p.set(&k, &v);
        } else if let Some(r) = nested(&k, &v) {
            r?;
        } else {
            return Err(js_error(&OfficeError::new("InvalidArgument", format!("the property '{k}' can't be set here"))));
        }
    }
    Ok(())
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct RequestContext {
    p: P,
}

#[wasm_bindgen]
impl RequestContext {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RequestContext {
        RequestContext { p: P { ctx: Rc::new(RefCell::new(Ctx::new())), path: Vec::new() } }
    }
    #[wasm_bindgen(getter)]
    pub fn workbook(&self) -> Workbook {
        Workbook { p: self.p.to(prop("workbook")) }
    }
    /// Sends the queued actions; resolves once loaded properties are filled.
    pub fn sync(&self) -> Promise {
        let ctx = self.p.ctx.clone();
        future_to_promise(async move { sync(ctx).await.map(|_| JsValue::UNDEFINED) })
    }
    /// `context.trackedObjects.add/remove`: ranges are found by address, so there's nothing to track.
    #[wasm_bindgen(getter, js_name = trackedObjects)]
    pub fn tracked_objects(&self) -> JsValue {
        let o = Object::new();
        let noop = Closure::<dyn Fn(JsValue)>::new(|_| {}).into_js_value();
        let _ = Reflect::set(&o, &"add".into(), &noop);
        let _ = Reflect::set(&o, &"remove".into(), &noop);
        o.into()
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        RequestContext::new()
    }
}

async fn sync(ctx: Rc<RefCell<Ctx>>) -> Result<(), JsValue> {
    let (actions, pending) = ctx.borrow_mut().take();
    if actions.is_empty() {
        return Ok(());
    }
    let answer = send(actions).await.map_err(|e| js_error(&e))?;
    ctx.borrow_mut().apply(pending, &answer).map(|_| ()).map_err(|e| js_error(&e))
}

#[wasm_bindgen]
pub struct Workbook {
    p: P,
}

#[wasm_bindgen]
impl Workbook {
    #[wasm_bindgen(getter)]
    pub fn worksheets(&self) -> WorksheetCollection {
        WorksheetCollection { p: self.p.to(prop("worksheets")) }
    }
    #[wasm_bindgen(js_name = getSelectedRange)]
    pub fn get_selected_range(&self) -> Range {
        Range { p: self.p.to(method("getSelectedRange", vec![])) }
    }
    #[wasm_bindgen(js_name = getActiveCell)]
    pub fn get_active_cell(&self) -> Range {
        Range { p: self.p.to(method("getActiveCell", vec![])) }
    }
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> Result<JsValue, JsValue> {
        self.p.get("name")
    }
    pub fn load(&self, props: JsValue) -> Workbook {
        self.p.load(&props);
        Workbook { p: self.p.clone() }
    }
}

#[wasm_bindgen]
pub struct WorksheetCollection {
    p: P,
}

#[wasm_bindgen]
impl WorksheetCollection {
    #[wasm_bindgen(js_name = getActiveWorksheet)]
    pub fn get_active_worksheet(&self) -> Worksheet {
        Worksheet { p: self.p.to(method("getActiveWorksheet", vec![])) }
    }
    #[wasm_bindgen(js_name = getItem)]
    pub fn get_item(&self, key: String) -> Worksheet {
        Worksheet { p: self.p.to(method("getItem", vec![json!(key)])) }
    }
    /// Adds a worksheet at the end (the next "SheetN" without a name). The proxy finds it by
    /// the name given, or by a token the host remembers for it (`getAdded`).
    pub fn add(&self, name: Option<String>) -> Worksheet {
        match name.filter(|n| !n.trim().is_empty()) {
            Some(name) => {
                self.p.call("add", vec![json!(name)]);
                self.get_item(name)
            }
            None => {
                let n = ADDED.with(|a| {
                    let n = a.get().wrapping_add(1);
                    a.set(n);
                    n
                });
                let token = format!("{n:x}-{:x}", (js_sys::Math::random() * 1e15) as u64);
                self.p.call("add", vec![Json::Null, json!({"token": token})]);
                Worksheet { p: self.p.to(method("getAdded", vec![json!(token)])) }
            }
        }
    }
    /// The worksheets, once `load("items")` and a sync have run.
    #[wasm_bindgen(getter)]
    pub fn items(&self) -> Result<Array, JsValue> {
        let items = self.p.ctx.borrow().get(&self.p.path, "items").cloned().ok_or_else(|| js_error(&OfficeError::not_loaded("items")))?;
        let out = Array::new();
        for item in items.as_array().into_iter().flatten() {
            if let Some(name) = item.get("name").and_then(Json::as_str) {
                out.push(&JsValue::from(self.get_item(name.to_string())));
            }
        }
        Ok(out)
    }
    pub fn load(&self, props: JsValue) -> WorksheetCollection {
        let props = if props.is_undefined() { JsValue::from_str("items") } else { props };
        self.p.load(&props);
        WorksheetCollection { p: self.p.clone() }
    }
}

#[wasm_bindgen]
pub struct Worksheet {
    p: P,
}

#[wasm_bindgen]
impl Worksheet {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> Result<JsValue, JsValue> {
        self.p.get("name")
    }
    #[wasm_bindgen(setter)]
    pub fn set_name(&self, v: JsValue) {
        self.p.set("name", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn position(&self) -> Result<JsValue, JsValue> {
        self.p.get("position")
    }
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> Result<JsValue, JsValue> {
        self.p.get("id")
    }
    #[wasm_bindgen(js_name = getRange)]
    pub fn get_range(&self, address: Option<String>) -> Range {
        let args = address.map(|a| vec![json!(a)]).unwrap_or_default();
        Range { p: self.p.to(method("getRange", args)) }
    }
    #[wasm_bindgen(js_name = getCell)]
    pub fn get_cell(&self, row: f64, column: f64) -> Range {
        Range { p: self.p.to(method("getCell", vec![num(row), num(column)])) }
    }
    #[wasm_bindgen(js_name = getUsedRange)]
    pub fn get_used_range(&self) -> Range {
        Range { p: self.p.to(method("getUsedRange", vec![])) }
    }
    pub fn activate(&self) {
        self.p.call("activate", vec![]);
    }
    pub fn delete(&self) {
        self.p.call("delete", vec![]);
    }
    pub fn load(&self, props: JsValue) -> Worksheet {
        self.p.load(&props);
        Worksheet { p: self.p.clone() }
    }
    /// `set({name})`.
    pub fn set(&self, props: JsValue) -> Result<(), JsValue> {
        set_all(&props, &["name"], &self.p, |_, _| None)
    }
}

#[wasm_bindgen]
pub struct Range {
    p: P,
}

#[wasm_bindgen]
impl Range {
    #[wasm_bindgen(getter)]
    pub fn values(&self) -> Result<JsValue, JsValue> {
        self.p.get("values")
    }
    #[wasm_bindgen(setter)]
    pub fn set_values(&self, v: JsValue) {
        self.p.set("values", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn formulas(&self) -> Result<JsValue, JsValue> {
        self.p.get("formulas")
    }
    #[wasm_bindgen(setter)]
    pub fn set_formulas(&self, v: JsValue) {
        self.p.set("formulas", &v);
    }
    #[wasm_bindgen(getter = numberFormat)]
    pub fn number_format(&self) -> Result<JsValue, JsValue> {
        self.p.get("numberFormat")
    }
    #[wasm_bindgen(setter = numberFormat)]
    pub fn set_number_format(&self, v: JsValue) {
        self.p.set("numberFormat", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> Result<JsValue, JsValue> {
        self.p.get("text")
    }
    #[wasm_bindgen(getter)]
    pub fn address(&self) -> Result<JsValue, JsValue> {
        self.p.get("address")
    }
    #[wasm_bindgen(getter = rowCount)]
    pub fn row_count(&self) -> Result<JsValue, JsValue> {
        self.p.get("rowCount")
    }
    #[wasm_bindgen(getter = columnCount)]
    pub fn column_count(&self) -> Result<JsValue, JsValue> {
        self.p.get("columnCount")
    }
    #[wasm_bindgen(getter = cellCount)]
    pub fn cell_count(&self) -> Result<JsValue, JsValue> {
        self.p.get("cellCount")
    }
    #[wasm_bindgen(getter = rowIndex)]
    pub fn row_index(&self) -> Result<JsValue, JsValue> {
        self.p.get("rowIndex")
    }
    #[wasm_bindgen(getter = columnIndex)]
    pub fn column_index(&self) -> Result<JsValue, JsValue> {
        self.p.get("columnIndex")
    }
    #[wasm_bindgen(getter)]
    pub fn format(&self) -> RangeFormat {
        RangeFormat { p: self.p.to(prop("format")) }
    }
    #[wasm_bindgen(getter)]
    pub fn worksheet(&self) -> Worksheet {
        Worksheet { p: self.p.to(prop("worksheet")) }
    }
    fn to(&self, m: &str, args: Vec<Json>) -> Range {
        Range { p: self.p.to(method(m, args)) }
    }
    #[wasm_bindgen(js_name = getCell)]
    pub fn get_cell(&self, row: f64, column: f64) -> Range {
        self.to("getCell", vec![num(row), num(column)])
    }
    #[wasm_bindgen(js_name = getOffsetRange)]
    pub fn get_offset_range(&self, rows: f64, columns: f64) -> Range {
        self.to("getOffsetRange", vec![num(rows), num(columns)])
    }
    #[wasm_bindgen(js_name = getResizedRange)]
    pub fn get_resized_range(&self, delta_rows: f64, delta_columns: f64) -> Range {
        self.to("getResizedRange", vec![num(delta_rows), num(delta_columns)])
    }
    #[wasm_bindgen(js_name = getRow)]
    pub fn get_row(&self, row: f64) -> Range {
        self.to("getRow", vec![num(row)])
    }
    #[wasm_bindgen(js_name = getColumn)]
    pub fn get_column(&self, column: f64) -> Range {
        self.to("getColumn", vec![num(column)])
    }
    #[wasm_bindgen(js_name = getEntireRow)]
    pub fn get_entire_row(&self) -> Range {
        self.to("getEntireRow", vec![])
    }
    #[wasm_bindgen(js_name = getEntireColumn)]
    pub fn get_entire_column(&self) -> Range {
        self.to("getEntireColumn", vec![])
    }
    #[wasm_bindgen(js_name = getUsedRange)]
    pub fn get_used_range(&self) -> Range {
        self.to("getUsedRange", vec![])
    }
    /// `clear(applyTo?)`: `"All"` (default), `"Formats"` or `"Contents"`.
    pub fn clear(&self, apply_to: Option<String>) {
        self.p.call("clear", vec![json!(apply_to.unwrap_or_else(|| "All".into()))]);
    }
    pub fn select(&self) {
        self.p.call("select", vec![]);
    }
    pub fn load(&self, props: JsValue) -> Range {
        self.p.load(&props);
        Range { p: self.p.clone() }
    }
    /// `set({values, formulas, numberFormat, format: {...}})`.
    pub fn set(&self, props: JsValue) -> Result<(), JsValue> {
        set_all(&props, &["values", "formulas", "numberFormat"], &self.p, |k, v| (k == "format").then(|| self.format().set(v.clone())))
    }
}

#[wasm_bindgen]
pub struct RangeFormat {
    p: P,
}

#[wasm_bindgen]
impl RangeFormat {
    #[wasm_bindgen(getter)]
    pub fn font(&self) -> RangeFont {
        RangeFont { p: self.p.to(prop("font")) }
    }
    #[wasm_bindgen(getter)]
    pub fn fill(&self) -> RangeFill {
        RangeFill { p: self.p.to(prop("fill")) }
    }
    pub fn load(&self, props: JsValue) -> RangeFormat {
        self.p.load(&props);
        RangeFormat { p: self.p.clone() }
    }
    /// `set({font: {...}, fill: {...}})`.
    pub fn set(&self, props: JsValue) -> Result<(), JsValue> {
        set_all(&props, &[], &self.p, |k, v| match k {
            "font" => Some(self.font().set(v.clone())),
            "fill" => Some(self.fill().set(v.clone())),
            _ => None,
        })
    }
}

#[wasm_bindgen]
pub struct RangeFont {
    p: P,
}

#[wasm_bindgen]
impl RangeFont {
    #[wasm_bindgen(getter)]
    pub fn bold(&self) -> Result<JsValue, JsValue> {
        self.p.get("bold")
    }
    #[wasm_bindgen(setter)]
    pub fn set_bold(&self, v: JsValue) {
        self.p.set("bold", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn italic(&self) -> Result<JsValue, JsValue> {
        self.p.get("italic")
    }
    #[wasm_bindgen(setter)]
    pub fn set_italic(&self, v: JsValue) {
        self.p.set("italic", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn color(&self) -> Result<JsValue, JsValue> {
        self.p.get("color")
    }
    #[wasm_bindgen(setter)]
    pub fn set_color(&self, v: JsValue) {
        self.p.set("color", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> Result<JsValue, JsValue> {
        self.p.get("size")
    }
    #[wasm_bindgen(setter)]
    pub fn set_size(&self, v: JsValue) {
        self.p.set("size", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> Result<JsValue, JsValue> {
        self.p.get("name")
    }
    #[wasm_bindgen(setter)]
    pub fn set_name(&self, v: JsValue) {
        self.p.set("name", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn underline(&self) -> Result<JsValue, JsValue> {
        self.p.get("underline")
    }
    #[wasm_bindgen(setter)]
    pub fn set_underline(&self, v: JsValue) {
        self.p.set("underline", &v);
    }
    #[wasm_bindgen(getter)]
    pub fn strikethrough(&self) -> Result<JsValue, JsValue> {
        self.p.get("strikethrough")
    }
    #[wasm_bindgen(setter)]
    pub fn set_strikethrough(&self, v: JsValue) {
        self.p.set("strikethrough", &v);
    }
    pub fn load(&self, props: JsValue) -> RangeFont {
        self.p.load(&props);
        RangeFont { p: self.p.clone() }
    }
    /// `set({bold, italic, underline, strikethrough, color, size, name})`.
    pub fn set(&self, props: JsValue) -> Result<(), JsValue> {
        set_all(&props, &["bold", "italic", "underline", "strikethrough", "color", "size", "name"], &self.p, |_, _| None)
    }
}

#[wasm_bindgen]
pub struct RangeFill {
    p: P,
}

#[wasm_bindgen]
impl RangeFill {
    #[wasm_bindgen(getter)]
    pub fn color(&self) -> Result<JsValue, JsValue> {
        self.p.get("color")
    }
    #[wasm_bindgen(setter)]
    pub fn set_color(&self, v: JsValue) {
        self.p.set("color", &v);
    }
    /// No fill.
    pub fn clear(&self) {
        self.p.ctx.borrow_mut().set(&self.p.path, "color", json!("none"));
    }
    pub fn load(&self, props: JsValue) -> RangeFill {
        self.p.load(&props);
        RangeFill { p: self.p.clone() }
    }
    /// `set({color})`.
    pub fn set(&self, props: JsValue) -> Result<(), JsValue> {
        set_all(&props, &["color"], &self.p, |_, _| None)
    }
}

// ---------------------------------------------------------------- Excel.run

/// `Excel.run(batch)` or `Excel.run(options, batch)`: runs `batch(context)`, then sends what it
/// left queued; resolves with the batch's value.
fn excel_run(a: JsValue, b: JsValue) -> Promise {
    let batch = a.clone().dyn_into::<Function>().or_else(|_| b.dyn_into::<Function>());
    future_to_promise(async move {
        let batch = batch.map_err(|_| js_error(&OfficeError::new("InvalidArgument", "Excel.run needs a function")))?;
        let rc = RequestContext::new();
        let ctx = rc.p.ctx.clone();
        let mut value = batch.call1(&JsValue::UNDEFINED, &JsValue::from(rc))?;
        if let Ok(p) = value.clone().dyn_into::<Promise>() {
            value = JsFuture::from(p).await?;
        }
        if ctx.borrow().has_queued() {
            sync(ctx).await?;
        }
        Ok(value)
    })
}

// ---------------------------------------------------------------- Office (common API)

fn platform() -> String {
    let host = Reflect::get(&js_sys::global(), &"__craftOffice".into()).unwrap_or(JsValue::UNDEFINED);
    Reflect::get(&host, &"platform".into()).ok().and_then(|p| p.as_string()).unwrap_or_else(|| "PC".into())
}

fn obj(pairs: &[(&str, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &(*k).into(), v);
    }
    o
}

/// `{status, value, error, asyncContext}` for a callback.
fn async_result(r: Result<JsValue, OfficeError>, async_context: JsValue) -> JsValue {
    let o = match r {
        Ok(v) => obj(&[("status", "succeeded".into()), ("value", v)]),
        Err(e) => {
            let err = obj(&[("code", e.code.as_str().into()), ("name", e.code.as_str().into()), ("message", e.message.as_str().into())]);
            obj(&[("status", "failed".into()), ("value", JsValue::UNDEFINED), ("error", err.into())])
        }
    };
    let _ = Reflect::set(&o, &"asyncContext".into(), &async_context);
    o.into()
}

/// `(options?, callback?)` as Office.js's `*Async` methods take them.
fn options_and_callback(a: JsValue, b: JsValue) -> (JsValue, Option<Function>) {
    match a.clone().dyn_into::<Function>() {
        Ok(f) => (JsValue::UNDEFINED, Some(f)),
        Err(_) => (a, b.dyn_into::<Function>().ok()),
    }
}

fn finish(callback: Option<Function>, options: JsValue, r: Result<JsValue, OfficeError>) {
    if let Some(cb) = callback {
        let ctx = Reflect::get(&options, &"asyncContext".into()).unwrap_or(JsValue::UNDEFINED);
        let _ = cb.call1(&JsValue::UNDEFINED, &async_result(r, ctx));
    }
}

async fn document_call(method_name: &str, args: Vec<Json>) -> Result<Json, OfficeError> {
    let answer = send(vec![json!({"op": "call", "path": [prop("document")], "method": method_name, "args": args})]).await?;
    let results = crate::core::results(&answer)?;
    Ok(results.first().cloned().unwrap_or(Json::Null))
}

fn document() -> Object {
    let get_selected = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|coercion: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let kind = coercion.as_string().unwrap_or_else(|| "text".into());
        spawn_local(async move {
            let r = match kind.as_str() {
                "text" | "matrix" => document_call("getSelectedData", vec![json!(kind)]).await.map(|v| to_js(&v)),
                other => Err(OfficeError::not_implemented(&format!("coercion type {other}"))),
            };
            finish(cb, options, r);
        });
    });
    let set_selected = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|data: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let data = from_js(&data);
        spawn_local(async move {
            let r = document_call("setSelectedData", vec![data]).await.map(|_| JsValue::UNDEFINED);
            finish(cb, options, r);
        });
    });
    let doc = obj(&[
        ("getSelectedDataAsync", get_selected.into_js_value()),
        ("setSelectedDataAsync", set_selected.into_js_value()),
        ("addHandlerAsync", add_handler()),
        ("removeHandlerAsync", remove_handler()),
        ("goToByIdAsync", unsupported_async("goToByIdAsync")),
        ("getFileAsync", unsupported_async("getFileAsync")),
        ("getFilePropertiesAsync", unsupported_async("getFilePropertiesAsync")),
    ]);
    let bindings = obj(&[
        ("addFromNamedItemAsync", unsupported_async("bindings")),
        ("addFromPromptAsync", unsupported_async("bindings")),
        ("addFromSelectionAsync", unsupported_async("bindings")),
        ("getAllAsync", unsupported_async("bindings")),
        ("getByIdAsync", unsupported_async("bindings")),
        ("releaseByIdAsync", unsupported_async("bindings")),
    ]);
    let _ = Reflect::set(&doc, &"bindings".into(), &bindings.into());
    let _ = Reflect::set(&doc, &"settings".into(), &settings().into());
    let _ = Reflect::set(&doc, &"mode".into(), &"readWrite".into());
    doc
}

/// Calls `callback(asyncResult)` later, as Office does (never before the method returns).
fn finish_later(callback: Option<Function>, options: JsValue, r: Result<JsValue, OfficeError>) {
    if callback.is_some() {
        spawn_local(async move { finish(callback, options, r) });
    }
}

fn not_supported(what: &str) -> OfficeError {
    OfficeError::new("NotSupported", format!("{what} isn't supported by GridCraft yet"))
}

/// A common-API `*Async` method GridCraft doesn't have: its callback (the last function among the
/// arguments) gets a failed result with code `NotSupported`, instead of the page getting a `TypeError`.
fn unsupported_async(what: &'static str) -> JsValue {
    Closure::<dyn Fn(JsValue, JsValue, JsValue, JsValue)>::new(move |a: JsValue, b: JsValue, c: JsValue, d: JsValue| {
        let args = [a, b, c, d];
        let cb = args.iter().rev().find_map(|v| v.clone().dyn_into::<Function>().ok());
        let options = args.iter().rev().find(|v| v.is_object() && !v.is_function()).cloned().unwrap_or(JsValue::UNDEFINED);
        finish_later(cb, options, Err(not_supported(what)));
    })
    .into_js_value()
}

/// A Promise-based method GridCraft doesn't have: rejects with `NotImplemented`.
fn rejecting(what: &'static str) -> JsValue {
    Closure::<dyn Fn() -> Promise>::new(move || Promise::reject(&not_implemented(what))).into_js_value()
}

const SELECTION_CHANGED: &str = "documentSelectionChanged";

/// `addHandlerAsync(eventType, handler, options?, callback?)`: `DocumentSelectionChanged` is
/// delivered (`__craftOfficeEvent`); other events answer `NotSupported`.
fn add_handler() -> JsValue {
    Closure::<dyn Fn(JsValue, JsValue, JsValue, JsValue)>::new(|kind: JsValue, handler: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let kind = kind.as_string().unwrap_or_default();
        let r = if kind != SELECTION_CHANGED {
            Err(not_supported(&format!("the event `{kind}`")))
        } else {
            match handler.dyn_into::<Function>() {
                Ok(f) => SELECTION_HANDLERS.with(|h| {
                    let mut h = h.borrow_mut();
                    if h.len() >= MAX_REGISTERED {
                        return Err(OfficeError::new("InvalidArgument", "too many handlers"));
                    }
                    h.push(f);
                    Ok(JsValue::UNDEFINED)
                }),
                Err(_) => Err(OfficeError::new("InvalidArgument", "addHandlerAsync needs a handler function")),
            }
        };
        finish_later(cb, options, r);
    })
    .into_js_value()
}

/// `removeHandlerAsync(eventType, options?, callback?)`: `options.handler`, or all of them.
fn remove_handler() -> JsValue {
    Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|kind: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let kind = kind.as_string().unwrap_or_default();
        let r = if kind == SELECTION_CHANGED {
            let which = Reflect::get(&options, &"handler".into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
            SELECTION_HANDLERS.with(|h| match which {
                Some(f) => h.borrow_mut().retain(|g| !Object::is(g, &f)),
                None => h.borrow_mut().clear(),
            });
            Ok(JsValue::UNDEFINED)
        } else {
            Err(not_supported(&format!("the event `{kind}`")))
        };
        finish_later(cb, options, r);
    })
    .into_js_value()
}

/// `globalThis.__craftOfficeEvent(json)`: the host tells the page something happened
/// (`{"type": "documentSelectionChanged"}`).
fn host_event(text: String) {
    let kind = serde_json::from_str::<Json>(&text).ok().and_then(|v| v.get("type").and_then(Json::as_str).map(str::to_string));
    if kind.as_deref() != Some(SELECTION_CHANGED) {
        return;
    }
    let office = Reflect::get(&js_sys::global(), &"Office".into()).unwrap_or(JsValue::UNDEFINED);
    let document = Reflect::get(&office, &"context".into()).and_then(|c| Reflect::get(&c, &"document".into())).unwrap_or(JsValue::UNDEFINED);
    let args = obj(&[("type", SELECTION_CHANGED.into()), ("document", document)]);
    for f in SELECTION_HANDLERS.with(|h| h.borrow().clone()) {
        let _ = f.call1(&JsValue::UNDEFINED, &args);
    }
}

/// `Office.context.document.settings`: kept here, saved to the host with `saveAsync`.
fn settings() -> Object {
    let get = Closure::<dyn Fn(String) -> JsValue>::new(|name: String| SETTINGS.with(|s| s.borrow().get(&name).map(to_js).unwrap_or(JsValue::NULL)));
    let set = Closure::<dyn Fn(String, JsValue)>::new(|name: String, v: JsValue| {
        let v = from_js(&v);
        SETTINGS.with(|s| s.borrow_mut().insert(name, v));
    });
    let remove = Closure::<dyn Fn(String)>::new(|name: String| {
        SETTINGS.with(|s| s.borrow_mut().remove(&name));
    });
    let save = Closure::<dyn Fn(JsValue, JsValue)>::new(|a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let all = SETTINGS.with(|s| Json::Object(s.borrow().clone()));
        spawn_local(async move {
            let r = match send(vec![json!({"op": "call", "path": [prop("settings")], "method": "save", "args": [all]})]).await {
                Ok(answer) => crate::core::results(&answer).map(|_| JsValue::UNDEFINED),
                Err(e) => Err(e),
            };
            finish(cb, options, r);
        });
    });
    obj(&[("get", get.into_js_value()), ("set", set.into_js_value()), ("remove", remove.into_js_value()), ("saveAsync", save.into_js_value())])
}

/// Reads the add-in's saved settings from the host (at start).
async fn load_settings() {
    if let Ok(answer) = send(vec![json!({"op": "load", "path": [prop("settings")], "props": []})]).await
        && let Some(Json::Object(saved)) = answer.get("results").and_then(|r| r.get(0)).cloned()
    {
        SETTINGS.with(|s| *s.borrow_mut() = saved);
    }
}

/// Sends the host an event (`{"event": "ready"}`, `{"event": "functionDone", …}`); no answer.
fn post_event(event: Json) {
    let text = event.to_string();
    if let Some(t) = TRANSPORT.with(|t| t.borrow().clone()) {
        let _ = t(text);
        return;
    }
    let host = Reflect::get(&js_sys::global(), &"__craftOffice".into()).unwrap_or(JsValue::UNDEFINED);
    if let Ok(post) = Reflect::get(&host, &"post".into()).and_then(|f| f.dyn_into::<Function>()) {
        let _ = post.call1(&host, &text.into());
    }
}

/// Resolves once the page has loaded (its scripts have run, so their functions are associated).
async fn page_loaded() {
    let global = js_sys::global();
    let doc = Reflect::get(&global, &"document".into()).unwrap_or(JsValue::UNDEFINED);
    if !doc.is_object() || Reflect::get(&doc, &"readyState".into()).ok().and_then(|s| s.as_string()).as_deref() == Some("complete") {
        return;
    }
    let loaded = Promise::new(&mut |resolve, _| match Reflect::get(&global, &"addEventListener".into()).and_then(|f| f.dyn_into::<Function>()) {
        Ok(add) => {
            let _ = add.call2(&global, &"load".into(), &resolve);
        }
        Err(_) => {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        }
    });
    let _ = JsFuture::from(loaded).await;
}

/// The function an `ExecuteFunction` command names: associated with `Office.actions.associate`,
/// else a global function (`name` or `a.b.name`), as function files without `associate` have.
fn find_function(name: &str) -> Option<Function> {
    if let Some(f) = ACTIONS.with(|a| a.borrow().get(name).cloned()) {
        return Some(f);
    }
    let mut at: JsValue = js_sys::global().into();
    for part in name.split('.') {
        at = Reflect::get(&at, &part.into()).ok().filter(|v| v.is_object() || v.is_function())?;
    }
    at.dyn_into::<Function>().ok()
}

fn error_text(e: &JsValue) -> String {
    Reflect::get(e, &"message".into()).ok().and_then(|m| m.as_string()).or_else(|| e.as_string()).unwrap_or_else(|| "the function failed".into())
}

/// `globalThis.__craftOfficeRunFunction({"call", "name", "control"})`: runs a function command
/// once the library and the page are ready. The function gets `event.source.id` (the control's id
/// in the manifest) and `event.completed()`, which tells the host it has finished.
fn run_function(text: String) {
    let call: Json = serde_json::from_str(&text).unwrap_or(Json::Null);
    let id = call.get("call").cloned().unwrap_or(Json::Null);
    let name = call.get("name").and_then(Json::as_str).unwrap_or_default().to_string();
    let control = call.get("control").and_then(Json::as_str).unwrap_or_default().to_string();
    let ready = READY.with(|r| r.borrow().clone());
    spawn_local(async move {
        if let Some(ready) = ready {
            let _ = JsFuture::from(ready).await;
        }
        page_loaded().await;
        let done = Rc::new(Cell::new(false));
        let fail = {
            let (done, id) = (done.clone(), id.clone());
            move |message: String| {
                if !done.replace(true) {
                    post_event(json!({"event": "functionDone", "call": id, "error": message}));
                }
            }
        };
        let Some(f) = find_function(&name) else {
            fail(format!("the add-in's function file has no function `{name}`"));
            return;
        };
        let completed = {
            let (done, id) = (done.clone(), id.clone());
            Closure::<dyn Fn(JsValue)>::new(move |_options: JsValue| {
                if !done.replace(true) {
                    post_event(json!({"event": "functionDone", "call": id}));
                }
            })
            .into_js_value()
        };
        let event = obj(&[("source", obj(&[("id", control.as_str().into())]).into()), ("completed", completed)]);
        match f.call1(&JsValue::UNDEFINED, &event) {
            Ok(v) => {
                if let Ok(p) = v.dyn_into::<Promise>()
                    && let Err(e) = JsFuture::from(p).await
                {
                    fail(error_text(&e));
                }
            }
            Err(e) => fail(error_text(&e)),
        }
    });
}

/// `Office.actions`: `associate(name, fn)` for function commands; the keyboard-shortcut methods
/// reject with `NotImplemented`.
fn actions() -> Object {
    let associate = Closure::<dyn Fn(JsValue, JsValue)>::new(|name: JsValue, f: JsValue| {
        if let (Some(name), Ok(f)) = (name.as_string(), f.dyn_into::<Function>()) {
            ACTIONS.with(|a| {
                let mut a = a.borrow_mut();
                if a.len() < MAX_REGISTERED || a.contains_key(&name) {
                    a.insert(name, f);
                }
            });
        }
    });
    obj(&[
        ("associate", associate.into_js_value()),
        ("areShortcutsInUse", rejecting("Office.actions.areShortcutsInUse")),
        ("getShortcuts", rejecting("Office.actions.getShortcuts")),
        ("replaceShortcuts", rejecting("Office.actions.replaceShortcuts")),
    ])
}

fn enums(pairs: &[(&str, &str)]) -> JsValue {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &(*k).into(), &(*v).into());
    }
    o.into()
}

/// The common API's enumerations, as the API reference lists them.
fn office_enums() -> Vec<(&'static str, JsValue)> {
    vec![
        ("AsyncResultStatus", enums(&[("Succeeded", "succeeded"), ("Failed", "failed")])),
        (
            "CoercionType",
            enums(&[
                ("Text", "text"),
                ("Matrix", "matrix"),
                ("Table", "table"),
                ("Html", "html"),
                ("Ooxml", "ooxml"),
                ("SlideRange", "slideRange"),
                ("Image", "image"),
                ("XmlSvg", "xmlSvg"),
            ]),
        ),
        (
            "HostType",
            enums(&[
                ("Word", "Word"),
                ("Excel", "Excel"),
                ("PowerPoint", "PowerPoint"),
                ("Outlook", "Outlook"),
                ("OneNote", "OneNote"),
                ("Project", "Project"),
                ("Access", "Access"),
            ]),
        ),
        (
            "PlatformType",
            enums(&[
                ("PC", "PC"),
                ("OfficeOnline", "OfficeOnline"),
                ("Mac", "Mac"),
                ("iOS", "iOS"),
                ("Android", "Android"),
                ("Universal", "Universal"),
            ]),
        ),
        (
            "EventType",
            enums(&[
                ("ActiveViewChanged", "activeViewChanged"),
                ("AppointmentTimeChanged", "olkAppointmentTimeChanged"),
                ("AttachmentsChanged", "olkAttachmentsChanged"),
                ("BindingDataChanged", "bindingDataChanged"),
                ("BindingSelectionChanged", "bindingSelectionChanged"),
                ("DialogEventReceived", "dialogEventReceived"),
                ("DialogMessageReceived", "dialogMessageReceived"),
                ("DialogParentMessageReceived", "dialogParentMessageReceived"),
                ("DocumentSelectionChanged", "documentSelectionChanged"),
                ("DragAndDropEvent", "olkDragAndDropEvent"),
                ("EnhancedLocationsChanged", "olkEnhancedLocationsChanged"),
                ("InfobarClicked", "olkInfobarClicked"),
                ("ItemChanged", "olkItemSelectedChanged"),
                ("NodeDeleted", "nodeDeleted"),
                ("NodeInserted", "nodeInserted"),
                ("NodeReplaced", "nodeReplaced"),
                ("OfficeThemeChanged", "officeThemeChanged"),
                ("RecipientsChanged", "olkRecipientsChanged"),
                ("RecurrenceChanged", "olkRecurrenceChanged"),
                ("ResourceSelectionChanged", "resourceSelectionChanged"),
                ("SelectedItemsChanged", "olkSelectedItemsChanged"),
                ("SensitivityLabelChanged", "olkSensitivityLabelChanged"),
                ("SettingsChanged", "settingsChanged"),
                ("SpamReporting", "olkSpamReporting"),
                ("TaskSelectionChanged", "taskSelectionChanged"),
                ("ViewSelectionChanged", "viewSelectionChanged"),
            ]),
        ),
        ("Index", enums(&[("First", "first"), ("Last", "last"), ("Next", "next"), ("Previous", "previous")])),
        ("GoToType", enums(&[("Binding", "binding"), ("NamedItem", "namedItem"), ("Slide", "slide"), ("Index", "index")])),
        ("BindingType", enums(&[("Text", "text"), ("Matrix", "matrix"), ("Table", "table")])),
        ("StartupBehavior", enums(&[("none", "None"), ("load", "Load")])),
        ("VisibilityMode", enums(&[("hidden", "Hidden"), ("taskpane", "Taskpane")])),
        ("InitializationReason", enums(&[("Inserted", "inserted"), ("DocumentOpened", "documentOpened")])),
        ("ValueFormat", enums(&[("Unformatted", "unformatted"), ("Formatted", "formatted")])),
        ("FilterType", enums(&[("All", "all"), ("OnlyVisible", "onlyVisible")])),
        ("SelectionMode", enums(&[("Default", "default"), ("Selected", "selected"), ("None", "none")])),
        ("DocumentMode", enums(&[("ReadOnly", "readOnly"), ("ReadWrite", "readWrite")])),
        ("ActiveView", enums(&[("Read", "read"), ("Edit", "edit")])),
        ("FileType", enums(&[("Text", "text"), ("Compressed", "compressed"), ("Pdf", "pdf")])),
        ("Table", enums(&[("All", "all"), ("Data", "data"), ("Headers", "headers")])),
    ]
}

/// Calls the page's `Office.initialize` (later, as Office does).
fn call_initialize() {
    if let Some(f) = INITIALIZE.with(|i| i.borrow().clone()) {
        spawn_local(async move {
            let _ = f.call1(&JsValue::UNDEFINED, &"inserted".into());
        });
    }
}

/// `Office.initialize`: kept, and called once the library is ready, or at once if it already is.
fn define_initialize(office: &Object) {
    let get = Closure::<dyn Fn() -> JsValue>::new(|| INITIALIZE.with(|i| i.borrow().clone().map(JsValue::from).unwrap_or(JsValue::UNDEFINED)));
    let set = Closure::<dyn Fn(JsValue)>::new(|v: JsValue| {
        let f = v.dyn_into::<Function>().ok();
        let call_now = f.is_some() && IS_READY.with(Cell::get);
        INITIALIZE.with(|i| *i.borrow_mut() = f);
        if call_now {
            call_initialize();
        }
    });
    let desc = obj(&[("get", get.into_js_value()), ("set", set.into_js_value()), ("configurable", false.into()), ("enumerable", true.into())]);
    let _ = Reflect::define_property(office, &"initialize".into(), &desc);
}

/// Makes `o`'s properties read-only (a later script can't swap `Office.onReady` and the rest).
fn lock(o: &Object) {
    for key in Object::keys(o).iter() {
        let desc = obj(&[("writable", false.into()), ("configurable", false.into())]);
        let _ = Reflect::define_property(o, &key, &desc);
    }
}

fn office() -> Object {
    let platform = platform();
    let info = obj(&[("host", "Excel".into()), ("platform", platform.as_str().into())]);
    let on_ready = {
        let info = info.clone();
        Closure::<dyn Fn(JsValue) -> Promise>::new(move |cb: JsValue| {
            let ready = READY.with(|r| r.borrow().clone()).unwrap_or_else(|| Promise::resolve(&JsValue::UNDEFINED));
            let info = info.clone();
            future_to_promise(async move {
                let _ = JsFuture::from(ready).await;
                if let Ok(f) = cb.dyn_into::<Function>() {
                    let _ = f.call1(&JsValue::UNDEFINED, &info);
                }
                Ok(info.into())
            })
        })
    };
    let is_set = Closure::<dyn Fn(String, JsValue) -> bool>::new(|name: String, v: JsValue| {
        let v = v.as_string().or_else(|| v.as_f64().map(|f| format!("{f}")));
        is_set_supported(&name, v.as_deref())
    });
    let dialog = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|_url: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        finish_later(cb, options, Err(OfficeError::not_implemented("displayDialogAsync")));
    });
    let context = obj(&[
        ("host", "Excel".into()),
        ("diagnostics", obj(&[("host", "Excel".into()), ("platform", platform.as_str().into()), ("version", "16.0 (GridCraft)".into())]).into()),
        ("requirements", obj(&[("isSetSupported", is_set.into_js_value())]).into()),
        ("document", document().into()),
        (
            "ui",
            obj(&[("displayDialogAsync", dialog.into_js_value()), ("messageParent", Closure::<dyn Fn(JsValue)>::new(|_| {}).into_js_value())]).into(),
        ),
        ("displayLanguage", "en-US".into()),
        ("contentLanguage", "en-US".into()),
        ("platform", platform.as_str().into()),
    ]);
    let addin = obj(&[
        ("showAsTaskpane", rejecting("Office.addin.showAsTaskpane")),
        ("hide", rejecting("Office.addin.hide")),
        ("getStartupBehavior", rejecting("Office.addin.getStartupBehavior")),
        ("setStartupBehavior", rejecting("Office.addin.setStartupBehavior")),
        ("onVisibilityModeChanged", rejecting("Office.addin.onVisibilityModeChanged")),
    ]);
    let ribbon = obj(&[
        ("requestUpdate", rejecting("Office.ribbon.requestUpdate")),
        ("requestCreateControls", rejecting("Office.ribbon.requestCreateControls")),
    ]);
    let mut members = vec![
        ("onReady", on_ready.into_js_value()),
        ("context", context.into()),
        ("actions", actions().into()),
        ("addin", addin.into()),
        ("ribbon", ribbon.into()),
    ];
    members.extend(office_enums());
    let o = obj(&members);
    lock(&o);
    define_initialize(&o);
    o
}

fn excel() -> Object {
    let run = Closure::<dyn Fn(JsValue, JsValue) -> Promise>::new(excel_run);
    // `new Excel.RequestContext()`: the exported class, reached through an instance.
    let request_context = Reflect::get(&JsValue::from(RequestContext::new()), &"constructor".into()).unwrap_or(JsValue::UNDEFINED);
    let o = obj(&[
        ("run", run.into_js_value()),
        ("RequestContext", request_context),
        ("ClearApplyTo", enums(&[("All", "All"), ("Formats", "Formats"), ("Contents", "Contents")])),
    ]);
    lock(&o);
    o
}

fn office_extension() -> Object {
    let codes = Object::new();
    for c in ["AccessDenied", "GeneralException", "InvalidArgument", "ItemNotFound", "NotImplemented", "PropertyNotLoaded"] {
        let _ = Reflect::set(&codes, &c.into(), &c.into());
    }
    // `OfficeExtension.Error` is `Error`: every error this library throws is one.
    let error = Reflect::get(&js_sys::global(), &"Error".into()).unwrap_or(JsValue::UNDEFINED);
    let o = obj(&[("Error", error), ("ErrorCodes", codes.into())]);
    lock(&o);
    o
}

/// Defines a global the page uses. With the bootstrap (a task pane), its property nobody can
/// replace takes the object; without (tests), a property that isn't writable.
fn define(name: &str, value: &JsValue) {
    let global: Object = js_sys::global();
    if let Ok(adopt) = Reflect::get(&global, &"__craftOfficeAdopt".into()).and_then(|f| f.dyn_into::<Function>()) {
        let _ = adopt.call2(&JsValue::UNDEFINED, &name.into(), value);
        return;
    }
    let desc = obj(&[("value", value.clone()), ("writable", false.into()), ("configurable", false.into()), ("enumerable", true.into())]);
    let _ = Reflect::define_property(&global, &name.into(), &desc);
}

/// Defines one of the library's entry points for the host (not enumerable, not replaceable).
fn define_entry(name: &str, f: JsValue) {
    let desc = obj(&[("value", f), ("writable", false.into()), ("configurable", false.into()), ("enumerable", false.into())]);
    let _ = Reflect::define_property(&js_sys::global(), &name.into(), &desc);
}

/// Defines `Office`, `Excel`, `OfficeExtension` and `OfficeRuntime` (not replaceable: a later
/// `office.js` can't take over), the host's entry points, and loads the add-in's settings;
/// `onReady` resolves after that, `Office.initialize` is called and the host hears
/// `{"event": "ready"}`. Called again, it only starts over the settings and `onReady`.
fn install_globals() {
    if !INSTALLED.with(|i| i.replace(true)) {
        define("Office", &office().into());
        define("Excel", &excel().into());
        define("OfficeExtension", &office_extension().into());
        define("OfficeRuntime", &Object::new().into());
        define_entry("__craftOfficeReply", Closure::<dyn Fn(String)>::new(reply).into_js_value());
        define_entry("__craftOfficeRunFunction", Closure::<dyn Fn(String)>::new(run_function).into_js_value());
        define_entry("__craftOfficeEvent", Closure::<dyn Fn(String)>::new(host_event).into_js_value());
    }
    SETTINGS.with(|s| s.borrow_mut().clear());
    IS_READY.with(|r| r.set(false));
    let ready = future_to_promise(async {
        load_settings().await;
        IS_READY.with(|r| r.set(true));
        // A page that set `Office.initialize` gets it called, as Office does.
        call_initialize();
        post_event(json!({"event": "ready"}));
        Ok(JsValue::UNDEFINED)
    });
    READY.with(|r| *r.borrow_mut() = Some(ready));
}

/// Installs the library in a task pane that talks to GridCraft through
/// `globalThis.__craftOffice.post`.
#[cfg_attr(not(test), wasm_bindgen(start))]
pub fn install() {
    TRANSPORT.with(|t| *t.borrow_mut() = None);
    install_globals();
}

/// Installs the library with a synchronous transport (tests: the engine in the same process).
pub fn install_with_transport(t: impl Fn(String) -> String + 'static) {
    TRANSPORT.with(|tr| *tr.borrow_mut() = Some(Rc::new(t)));
    install_globals();
}

#[cfg(test)]
mod tests;
