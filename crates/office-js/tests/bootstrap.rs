//! The bootstrap (`bootstrap.js`, the script task panes get before their own) with the library,
//! in a process of its own: Microsoft's office.js never runs (its `<script>` gets an empty
//! source), a script that assigns its own `Office` doesn't replace GridCraft's, and what a page
//! does before the wasm has started (`onReady`, `initialize`, `Office.actions.associate`) reaches
//! the library. The DOM is a small stand-in (Node has none): script elements with `src` and
//! `setAttribute`, and a `MutationObserver` the test triggers as the HTML parser would. Run with
//! `wasm-bindgen-test-runner` (Node).
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

const BOOTSTRAP: &str = include_str!("../bootstrap.js");
const EMPTY_PREFIX: &str = "data:text/javascript,";

/// A `<script>` element: `src` (a property and an attribute) and its tag name.
#[wasm_bindgen]
pub struct FakeScript {
    src: RefCell<String>,
}

#[wasm_bindgen]
impl FakeScript {
    #[wasm_bindgen(constructor)]
    pub fn new() -> FakeScript {
        FakeScript { src: RefCell::new(String::new()) }
    }
    #[wasm_bindgen(getter)]
    pub fn src(&self) -> String {
        self.src.borrow().clone()
    }
    #[wasm_bindgen(setter)]
    pub fn set_src(&self, v: String) {
        *self.src.borrow_mut() = v;
    }
    #[wasm_bindgen(getter, js_name = tagName)]
    pub fn tag_name(&self) -> String {
        "SCRIPT".into()
    }
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, name: String, value: String) {
        if name == "src" {
            *self.src.borrow_mut() = value;
        }
    }
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: String) -> Option<String> {
        (name == "src").then(|| self.src.borrow().clone())
    }
}

impl Default for FakeScript {
    fn default() -> Self {
        FakeScript::new()
    }
}

thread_local! {
    static OBSERVERS: RefCell<Vec<Function>> = const { RefCell::new(Vec::new()) };
}

/// `new MutationObserver(callback).observe(...)`: the test calls the callbacks.
#[wasm_bindgen]
pub struct FakeObserver {
    callback: Function,
}

#[wasm_bindgen]
impl FakeObserver {
    #[wasm_bindgen(constructor)]
    pub fn new(callback: Function) -> FakeObserver {
        FakeObserver { callback }
    }
    pub fn observe(&self, _target: JsValue, _options: JsValue) {
        OBSERVERS.with(|o| o.borrow_mut().push(self.callback.clone()));
    }
}

fn global() -> Object {
    js_sys::global()
}

fn g(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap()
}

fn call(o: &JsValue, name: &str, args: &[JsValue]) -> JsValue {
    let f: Function = g(o, name).dyn_into().unwrap_or_else(|_| panic!("{name} is a function"));
    f.apply(o, &args.iter().cloned().collect::<Array>()).unwrap()
}

fn class_of(instance: JsValue) -> JsValue {
    g(&instance, "constructor")
}

/// A function the page would pass, and a promise of its first argument.
fn recorder() -> (Function, Promise) {
    let slot: Rc<RefCell<Option<Function>>> = Default::default();
    let p = {
        let slot = slot.clone();
        Promise::new(&mut |resolve, _| *slot.borrow_mut() = Some(resolve))
    };
    let f = Closure::<dyn Fn(JsValue)>::new(move |v: JsValue| {
        if let Some(r) = slot.borrow().as_ref() {
            let _ = r.call1(&JsValue::UNDEFINED, &v);
        }
    });
    (f.into_js_value().dyn_into().unwrap(), p)
}

fn src(script: &JsValue) -> String {
    g(script, "src").as_string().unwrap_or_default()
}

#[wasm_bindgen_test]
async fn the_bootstrap_keeps_microsofts_office_js_out_and_hands_the_page_to_the_library() {
    // The DOM stand-in, then the bootstrap, as the web view runs it before the page.
    let script_class = class_of(FakeScript::new().into());
    let observer_class = class_of(FakeObserver::new(Closure::<dyn Fn()>::new(|| {}).into_js_value().unchecked_into()).into());
    let document = Object::new();
    Reflect::set(&document, &"baseURI".into(), &"https://addin.example/pane.html".into()).unwrap();
    Reflect::set(&document, &"readyState".into(), &"complete".into()).unwrap();
    for (name, value) in
        [("Element", &script_class), ("HTMLScriptElement", &script_class), ("MutationObserver", &observer_class), ("document", &document.into())]
    {
        Reflect::set(&global(), &name.into(), value).unwrap();
    }
    Function::new_no_args(BOOTSTRAP).call0(&JsValue::UNDEFINED).unwrap();
    assert_eq!(OBSERVERS.with(|o| o.borrow().len()), 1, "the bootstrap watches the page's HTML");

    // Scripts the page's code makes: Microsoft's office.js gets an empty script.
    let made = |url: &str, how: &str| {
        let s: JsValue = FakeScript::new().into();
        match how {
            "property" => assert!(Reflect::set(&s, &"src".into(), &url.into()).unwrap()),
            _ => {
                call(&s, "setAttribute", &["src".into(), url.into()]);
            }
        }
        src(&s)
    };
    assert!(made("https://appsforoffice.microsoft.com/lib/1/hosted/office.js", "property").starts_with(EMPTY_PREFIX));
    assert!(made("https://officeapis.public.onecdn.static.microsoft/1/office.js", "attribute").starts_with(EMPTY_PREFIX));
    assert!(made("https://appsforoffice.microsoft.com/lib/1/hosted/excel-win32-16.01.js", "attribute").starts_with(EMPTY_PREFIX));
    assert!(made("/node_modules/@microsoft/office-js/dist/office.js", "property").starts_with(EMPTY_PREFIX));
    assert_eq!(made("https://addin.example/taskpane.js", "property"), "https://addin.example/taskpane.js");
    assert_eq!(made("office.js", "attribute"), "office.js", "the add-in's own file of that name is its business");
    // A <script> in the page's HTML: the parser inserts it, the observer runs before it would.
    let parsed = FakeScript::new();
    parsed.set_attribute("src".into(), "https://officeapis.public.onecdn.static.microsoft/1/office.js".into());
    let parsed: JsValue = parsed.into();
    let own = FakeScript::new();
    own.set_attribute("src".into(), "taskpane.js".into());
    let own: JsValue = own.into();
    let record = Object::new();
    Reflect::set(&record, &"addedNodes".into(), &Array::of2(&parsed, &own)).unwrap();
    let callback = OBSERVERS.with(|o| o.borrow()[0].clone());
    callback.call1(&JsValue::UNDEFINED, &Array::of1(&record)).unwrap();
    assert!(src(&parsed).starts_with(EMPTY_PREFIX), "{}", src(&parsed));
    assert_eq!(src(&own), "taskpane.js");

    // Before the wasm starts: the page registers onReady, associates a function, and another
    // script assigns its own `Office` (with an `initialize`), which is ignored.
    let office = g(&global().into(), "Office");
    let ready = call(&office, "onReady", &[]).dyn_into::<Promise>().unwrap();
    let (early, early_called) = recorder();
    call(&g(&office, "actions"), "associate", &["early".into(), early.into()]);
    let (init, init_called) = recorder();
    let foreign = Object::new();
    Reflect::set(&foreign, &"initialize".into(), &init.into()).unwrap();
    Reflect::set(&foreign, &"onReady".into(), &JsValue::from(1)).unwrap();
    let _ = Reflect::set(&global(), &"Office".into(), &foreign);
    assert!(Object::is(&g(&global().into(), "Office"), &office), "a foreign Office doesn't replace the stand-in");
    assert!(!Reflect::delete_property(&global(), &"Office".into()).unwrap_or(false), "nor can it be deleted");

    // The wasm starts: the library hands its objects over, and gets what the page did.
    let start = Closure::<dyn Fn(JsValue) -> Promise>::new(|_options: JsValue| {
        gridcraft_office_js::install_with_transport(|request| {
            let id = serde_json::from_str::<serde_json::Value>(&request).ok().and_then(|v| v.get("id").cloned()).unwrap_or_default();
            serde_json::json!({"id": id, "results": [{}]}).to_string()
        });
        Promise::resolve(&JsValue::UNDEFINED)
    });
    call(&global().into(), "__craftOfficeStart", &[start.into_js_value(), "".into()]);
    let info = JsFuture::from(ready).await.unwrap();
    assert_eq!(g(&info, "host").as_string().as_deref(), Some("Excel"), "onReady registered early answers");
    assert_eq!(JsFuture::from(init_called).await.unwrap().as_string().as_deref(), Some("inserted"), "the foreign object's initialize is called");
    let office = g(&global().into(), "Office");
    assert_eq!(g(&g(&office, "context"), "host").as_string().as_deref(), Some("Excel"), "the library's Office");
    call(&global().into(), "__craftOfficeRunFunction", &[r#"{"call": 1, "name": "early", "control": "Tests.Early"}"#.into()]);
    let event = JsFuture::from(early_called).await.unwrap();
    assert_eq!(g(&g(&event, "source"), "id").as_string().as_deref(), Some("Tests.Early"), "associated before the wasm started");
    // Microsoft's office.js run anyway (say, from a copy the observer missed) can't take over.
    let _ = Reflect::set(&global(), &"Office".into(), &Object::new());
    let _ = Reflect::set(&global(), &"Excel".into(), &JsValue::from(1));
    assert!(Object::is(&g(&global().into(), "Office"), &office));
    assert!(g(&g(&global().into(), "Excel"), "run").is_function());
    assert!(g(&global().into(), "OfficeRuntime").is_object());
    // The library's entry points can't be replaced either.
    let _ = Reflect::set(&global(), &"__craftOfficeReply".into(), &JsValue::from(1));
    assert!(g(&global().into(), "__craftOfficeReply").is_function());
}
