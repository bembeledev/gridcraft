//! craft-live mode (`index.html#craft-live=1&ws=…&room=…&token=…`): GridCraft as the live
//! spreadsheet editor of Nextcloud Office, framed by craft-live's shell page.
//!
//! The contract (craft-live `docs/design.md` §9) lives in the engine
//! ([`gridcraft_engine::live::LiveClient`], tested natively); this module is its browser I/O:
//! the shell's `postMessage` (same origin, `craft-live:*` objects), the room's
//! `web_sys::WebSocket` (binary y-websocket frames) and `fetch` (the file to seed from, the
//! uploads the room asks for). Browser callbacks only queue [`Event`]s; the frame loop hands them
//! to the client, so the session is only ever touched from the frame.
//!
//! In this mode Save asks the room to save (Nextcloud gets the file); exports still download.
//! The title bar gets a Close button (Nextcloud shows none), and File › Close closes the
//! document in Nextcloud too.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use gridcraft_engine::live::{Action, HttpRequest, HttpResponse, LiveClient, LiveConfig, Phase, TRANSPORT_LABEL};
use gridcraft_ui_egui::{HostSession, Services, SheetApp};
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

/// Without a usable configuration this long after hello, the page says so.
const CONFIG_WAIT_MS: f64 = 20_000.0;

/// What the browser said, for the next frame.
enum Event {
    Open(u64),
    Message(u64, Vec<u8>),
    Close(u64, u16, String),
    Http(u64, Result<HttpResponse, String>),
    Shell(Json),
}

type Queue = Rc<RefCell<VecDeque<Event>>>;

/// One WebSocket and the callbacks it keeps alive.
struct Socket {
    conn: u64,
    ws: web_sys::WebSocket,
    _callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        self.ws.set_onerror(None);
    }
}

pub struct Live {
    client: Option<LiveClient>,
    /// The fragment couldn't be used: waiting for `craft-live:config`.
    config_error: Option<String>,
    events: Queue,
    ctx: Rc<RefCell<Option<egui::Context>>>,
    socket: Option<Socket>,
    parent: Option<web_sys::Window>,
    origin: String,
    started: f64,
    gave_up: bool,
    close_asked: Rc<Cell<bool>>,
    save_asked: Rc<Cell<bool>>,
}

/// The page's fragment, when it is craft-live's.
pub fn detect() -> Option<Result<LiveConfig, String>> {
    let hash = web_sys::window()?.location().hash().ok()?;
    LiveConfig::from_fragment(&hash)
}

fn now() -> f64 {
    js_sys::Date::now()
}

impl Live {
    /// Starts listening to the shell and says hello (as soon as the app runs, §9.2).
    pub fn new(config: Result<LiveConfig, String>) -> Live {
        let window = web_sys::window();
        let parent = window.as_ref().and_then(|w| w.parent().ok().flatten()).filter(|p| window.as_ref().is_some_and(|w| !js_sys::Object::is(p, w)));
        let origin = window.as_ref().and_then(|w| w.location().origin().ok()).unwrap_or_default();
        let (client, config_error) = match config {
            Ok(cfg) => (Some(LiveClient::new(cfg)), None),
            Err(e) => (None, Some(e)),
        };
        let live = Live {
            client,
            config_error,
            events: Queue::default(),
            ctx: Rc::default(),
            socket: None,
            parent,
            origin,
            started: now(),
            gave_up: false,
            close_asked: Rc::default(),
            save_asked: Rc::default(),
        };
        live.listen();
        live.post(&LiveClient::hello());
        live
    }

    /// Save (a workbook) asks the room to save instead of downloading; Close closes the document
    /// in Nextcloud.
    pub fn connect(&self, services: &mut Services, ctx: egui::Context) {
        *self.ctx.borrow_mut() = Some(ctx);
        let close = self.close_asked.clone();
        services.close = Some(Box::new(move || close.set(true)));
        let save = self.save_asked.clone();
        let download = services.download.take();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            if matches!(ext.as_str(), "xlsx" | "xlsm" | "xltx" | "xltm") {
                save.set(true);
            } else if let Some(d) = &download {
                d(name, bytes);
            }
        }));
    }

    /// Nextcloud's interface language, when it names one the web build can show.
    pub fn language(&self) -> Option<gridcraft_ui_egui::i18n::Language> {
        use gridcraft_ui_egui::i18n::Language;
        let lang = Language::parse(&self.client.as_ref()?.cfg.lang)?;
        // No CJK fonts are bundled with the web build.
        (!matches!(lang, Language::Zh | Language::Ja | Language::Ko)).then_some(lang)
    }

    fn listen(&self) {
        let Some(window) = web_sys::window() else { return };
        let Some(parent) = self.parent.clone() else { return };
        let origin = self.origin.clone();
        let events = self.events.clone();
        let ctx = self.ctx.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let Some(text) = js_sys::JSON::stringify(&event.data()).ok().and_then(|t| t.as_string()) else { return };
            if text.len() > 1 << 20 {
                return;
            }
            let Ok(v) = serde_json::from_str::<Json>(&text) else { return };
            if !v.get("type").and_then(Json::as_str).is_some_and(|t| t.starts_with("craft-live:")) {
                return;
            }
            events.borrow_mut().push_back(Event::Shell(v));
            if let Some(c) = ctx.borrow().as_ref() {
                c.request_repaint();
            }
        });
        if window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Posts an object to the shell.
    fn post(&self, msg: &Json) {
        let Some(parent) = &self.parent else { return };
        match js_sys::JSON::parse(&msg.to_string()) {
            Ok(obj) => {
                if let Err(e) = parent.post_message(&obj, &self.origin) {
                    log::warn!("craft-live: couldn't post to the shell: {e:?}");
                }
            }
            Err(e) => log::warn!("craft-live: {e:?}"),
        }
    }

    /// Each frame, after the interface ran: what the browser said goes to the client, the client
    /// runs the room, and its actions are carried out.
    pub fn frame(&mut self, app: &mut SheetApp) {
        let t = now();
        let events: Vec<Event> = self.events.borrow_mut().drain(..).collect();
        for e in events {
            if let Event::Shell(m) = &e
                && self.client.is_none()
                && m.get("type").and_then(Json::as_str) == Some("craft-live:config")
            {
                match LiveConfig::from_json(m.get("config").unwrap_or(&Json::Null)) {
                    Ok(cfg) => {
                        self.client = Some(LiveClient::new(cfg));
                        self.config_error = None;
                    }
                    Err(e) => self.config_error = Some(e),
                }
                continue;
            }
            let Some(client) = self.client.as_mut() else { continue };
            let s = &mut app.session;
            match e {
                Event::Open(conn) => client.on_open(conn),
                Event::Message(conn, data) => client.on_frame(s, conn, &data),
                Event::Close(conn, code, reason) => client.on_close(s, conn, code, &reason),
                Event::Http(id, answer) => client.on_http(s, id, answer),
                Event::Shell(m) => client.on_shell(s, &m),
            }
        }
        let Some(client) = self.client.as_mut() else {
            if !self.gave_up && t - self.started > CONFIG_WAIT_MS {
                self.gave_up = true;
                let why = format!(
                    "GridCraft couldn't start the live session: {}",
                    self.config_error.clone().unwrap_or_else(|| "craft-live sent no configuration".into())
                );
                self.post(&json!({"type": "craft-live:error", "message": why}));
                app.message = Some(("GridCraft".into(), why));
            }
            return;
        };
        let ms = t.max(0.0) as u64;
        if client.phase() == Phase::Starting {
            client.start(&mut app.session, ms);
        }
        if self.close_asked.take() {
            client.close(&mut app.session, true);
        }
        if self.save_asked.take() {
            client.save_now();
        }
        let changed = client.poll(&mut app.session, ms);
        let actions = client.take_actions();
        app.host_session = Some(HostSession { label: TRANSPORT_LABEL.into(), lines: client.summary() });
        let busy = !actions.is_empty();
        for a in actions {
            self.act(app, a);
        }
        if changed {
            app.after_engine();
        }
        if busy && let Some(c) = self.ctx.borrow().as_ref() {
            c.request_repaint();
        }
    }

    fn act(&mut self, app: &mut SheetApp, a: Action) {
        match a {
            Action::Connect { conn, url } => self.open_socket(conn, &url),
            Action::Send { conn, frame } => {
                if let Some(s) = self.socket.as_ref().filter(|s| s.conn == conn)
                    && s.ws.ready_state() == web_sys::WebSocket::OPEN
                    && let Err(e) = s.ws.send_with_u8_array(&frame)
                {
                    log::warn!("craft-live: send failed: {e:?}");
                }
            }
            Action::Disconnect { conn, code, reason } => {
                if let Some(s) = self.socket.take_if(|s| s.conn == conn) {
                    let _ = s.ws.close_with_code_and_reason(code, &reason);
                }
            }
            Action::Http(r) => self.fetch(r),
            Action::Shell(m) => self.post(&m),
            Action::Notice { text, .. } => app.status_message = Some((text, gridcraft_ui_egui::now_ms())),
            Action::Alert(text) => app.message = Some(("GridCraft".into(), text)),
        }
    }

    fn open_socket(&mut self, conn: u64, url: &str) {
        self.socket = None;
        let events = self.events.clone();
        let ws = match web_sys::WebSocket::new(url) {
            Ok(ws) => ws,
            Err(e) => {
                log::warn!("craft-live: can't open the WebSocket: {e:?}");
                events.borrow_mut().push_back(Event::Close(conn, 1006, "can't connect".into()));
                return;
            }
        };
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let repaint = {
            let ctx = self.ctx.clone();
            move || {
                if let Some(c) = ctx.borrow().as_ref() {
                    c.request_repaint();
                }
            }
        };
        let mut callbacks = Vec::new();
        let (ev, rp) = (events.clone(), repaint.clone());
        let on_open = Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| {
            ev.borrow_mut().push_back(Event::Open(conn));
            rp();
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        callbacks.push(on_open);
        let (ev, rp) = (events.clone(), repaint.clone());
        let on_message = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            let Ok(e) = e.dyn_into::<web_sys::MessageEvent>() else { return };
            let data = e.data();
            if data.is_instance_of::<js_sys::ArrayBuffer>() {
                ev.borrow_mut().push_back(Event::Message(conn, js_sys::Uint8Array::new(&data).to_vec()));
                rp();
            }
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        callbacks.push(on_message);
        let (ev, rp) = (events.clone(), repaint.clone());
        let on_close = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            let (code, reason) = e.dyn_into::<web_sys::CloseEvent>().map(|c| (c.code(), c.reason())).unwrap_or((1006, String::new()));
            ev.borrow_mut().push_back(Event::Close(conn, code, reason));
            rp();
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        callbacks.push(on_close);
        // An error is followed by `close`, which reconnects.
        let on_error = Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| log::warn!("craft-live: WebSocket error"));
        ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        callbacks.push(on_error);
        self.socket = Some(Socket { conn, ws, _callbacks: callbacks });
    }

    fn fetch(&self, r: HttpRequest) {
        let events = self.events.clone();
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let id = r.id;
            let answer = fetch(r).await;
            events.borrow_mut().push_back(Event::Http(id, answer));
            if let Some(c) = ctx.borrow().as_ref() {
                c.request_repaint();
            }
        });
    }
}

async fn fetch(r: HttpRequest) -> Result<HttpResponse, String> {
    let js = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(r.method);
    // The bearer token is the credential: no cookies, nothing cached.
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_cache(web_sys::RequestCache::NoStore);
    let headers = web_sys::Headers::new().map_err(js)?;
    for (k, v) in &r.headers {
        headers.set(k, v).map_err(js)?;
    }
    init.set_headers(&headers);
    if let Some(body) = &r.body {
        init.set_body(&js_sys::Uint8Array::from(body.as_slice()));
    }
    let request = web_sys::Request::new_with_str_and_init(&r.url, &init).map_err(js)?;
    let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "not a response".to_string())?;
    let status = response.status();
    let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok(HttpResponse { status, body: js_sys::Uint8Array::new(&buffer).to_vec() })
}
