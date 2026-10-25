//! The files saved in this browser, and handing a file to the device.
//!
//! - Save, Save As, Save a Copy and Save as Template keep the file in the browser's IndexedDB
//!   (<https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API>), under its name: saving
//!   again replaces it. The files are read into memory before the first frame, so the UI lists and
//!   opens them at once; each write goes to IndexedDB in the background. The page asks the browser
//!   to keep them (`navigator.storage.persist()`): Safari otherwise may clear a site's storage
//!   after seven days without a visit, unless the site was added to the Home Screen
//!   (<https://webkit.org/blog/14403/updates-to-storage-policy/>).
//! - Download hands a copy to the device: on a touch device with a share sheet for files (an iPad:
//!   Save to Files, AirDrop to a Mac) the Web Share API
//!   (<https://developer.mozilla.org/en-US/docs/Web/API/Navigator/share>), else, or when sharing is
//!   refused, a plain download.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_ui_egui::pencil_bar::{BrowserFiles, SavedFile};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

const DB: &str = "vectorcraft-files";
const STORE: &str = "files";
/// Files kept at most (the oldest go first).
const MAX_FILES: usize = 200;

/// A file kept in the browser.
struct Kept {
    name: String,
    bytes: Vec<u8>,
    /// Milliseconds since the Unix epoch (the browser's clock).
    saved: f64,
}

/// The kept files (the last saved last) and the database they are written to.
#[derive(Default)]
pub struct Library {
    files: Vec<Kept>,
    /// `None`: the browser keeps no files (IndexedDB turned off, as in some private windows).
    db: Option<web_sys::IdbDatabase>,
}

pub type SharedLibrary = Rc<RefCell<Library>>;

fn js_err(e: JsValue) -> String {
    Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string()).unwrap_or_else(|| format!("{e:?}"))
}

/// Wait for an IndexedDB request → its result.
async fn finished(req: &web_sys::IdbRequest) -> Result<JsValue, String> {
    let promise = Promise::new(&mut |resolve, reject| {
        let r = req.clone();
        let ok = Closure::once_into_js(move |_: web_sys::Event| {
            // Nothing waits on the promise's own result.
            let _ = resolve.call1(&JsValue::NULL, &r.result().unwrap_or(JsValue::UNDEFINED));
        });
        let r = req.clone();
        let failed = Closure::once_into_js(move |_: web_sys::Event| {
            let why: JsValue = r.error().ok().flatten().map_or_else(|| "the browser storage refused".into(), JsValue::from);
            let _ = reject.call1(&JsValue::NULL, &why);
        });
        req.set_onsuccess(Some(ok.unchecked_ref()));
        req.set_onerror(Some(failed.unchecked_ref()));
    });
    JsFuture::from(promise).await.map_err(js_err)
}

/// Open (or make) the database.
async fn open_db() -> Result<web_sys::IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(js_err)?.ok_or("this browser has no IndexedDB")?;
    let req = factory.open_with_u32(DB, 1).map_err(js_err)?;
    let r = req.clone();
    let upgrade = Closure::once_into_js(move |_: web_sys::Event| {
        let made = r.result().and_then(|db| db.dyn_into::<web_sys::IdbDatabase>()).and_then(|db| db.create_object_store(STORE));
        if let Err(e) = made {
            log::error!("vectorcraft-web: couldn't make the saved files' store: {}", js_err(e));
        }
    });
    req.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    finished(&req).await?.dyn_into::<web_sys::IdbDatabase>().map_err(|_| "IndexedDB opened no database".into())
}

fn store(db: &web_sys::IdbDatabase, mode: web_sys::IdbTransactionMode) -> Result<web_sys::IdbObjectStore, String> {
    db.transaction_with_str_and_mode(STORE, mode).map_err(js_err)?.object_store(STORE).map_err(js_err)
}

/// A stored record → the file it keeps.
fn from_record(v: &JsValue) -> Option<Kept> {
    let name = Reflect::get(v, &"name".into()).ok()?.as_string()?;
    let bytes = Reflect::get(v, &"bytes".into()).ok()?.dyn_into::<Uint8Array>().ok()?.to_vec();
    let saved = Reflect::get(v, &"saved".into()).ok()?.as_f64().unwrap_or(0.0);
    Some(Kept { name, bytes, saved })
}

fn record(k: &Kept) -> Result<Object, String> {
    let o = Object::new();
    Reflect::set(&o, &"name".into(), &k.name.as_str().into()).map_err(js_err)?;
    Reflect::set(&o, &"bytes".into(), &Uint8Array::from(k.bytes.as_slice())).map_err(js_err)?;
    Reflect::set(&o, &"saved".into(), &k.saved.into()).map_err(js_err)?;
    Ok(o)
}

/// Ask the browser to keep the site's storage (best effort: the answer only says whether it will).
fn ask_to_persist() {
    let Some(window) = web_sys::window() else { return };
    let navigator: JsValue = window.navigator().into();
    let Some(storage) = Reflect::get(&navigator, &"storage".into()).ok().filter(|s| s.is_object()) else { return };
    let Some(persist) = Reflect::get(&storage, &"persist".into()).ok().and_then(|f| f.dyn_into::<Function>().ok()) else { return };
    if let Ok(p) = persist.call0(&storage).and_then(|p| p.dyn_into::<Promise>()) {
        wasm_bindgen_futures::spawn_local(async move {
            match JsFuture::from(p).await.map(|v| v.as_bool()) {
                Ok(Some(true)) => log::info!("vectorcraft-web: the browser keeps the saved files"),
                _ => log::info!("vectorcraft-web: the browser may clear the saved files after a while: add the page to the Home Screen"),
            }
        });
    }
}

impl Library {
    /// The kept files read from the browser (before the first frame).
    pub async fn open() -> SharedLibrary {
        let mut lib = Library::default();
        match open_db().await {
            Ok(db) => {
                let all = match store(&db, web_sys::IdbTransactionMode::Readonly).and_then(|s| s.get_all().map_err(js_err)) {
                    Ok(req) => finished(&req).await,
                    Err(e) => Err(e),
                };
                match all {
                    Ok(v) => lib.files = Array::from(&v).iter().filter_map(|r| from_record(&r)).collect(),
                    Err(e) => log::error!("vectorcraft-web: couldn't read the saved files: {e}"),
                }
                lib.files.sort_by(|a, b| a.saved.total_cmp(&b.saved));
                lib.db = Some(db);
                ask_to_persist();
            }
            Err(e) => log::warn!("vectorcraft-web: files can't be saved in this browser: {e}"),
        }
        Rc::new(RefCell::new(lib))
    }

    /// Keep `bytes` as `name`, replacing a file of that name.
    pub fn keep(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let db = self.db.clone().ok_or("this browser doesn't keep files (a private window?): use Download to keep a copy")?;
        let k = Kept { name: name.to_string(), bytes: bytes.to_vec(), saved: js_sys::Date::now() };
        let req = store(&db, web_sys::IdbTransactionMode::Readwrite)?.put_with_key(&JsValue::from(record(&k)?), &name.into()).map_err(js_err)?;
        self.files.retain(|f| f.name != name);
        self.files.push(k);
        let extra = self.files.len().saturating_sub(MAX_FILES);
        for old in self.files.drain(..extra) {
            if let Err(e) = store(&db, web_sys::IdbTransactionMode::Readwrite).and_then(|s| s.delete(&old.name.as_str().into()).map_err(js_err)) {
                log::error!("vectorcraft-web: couldn't let go of {}: {e}", old.name);
            }
        }
        let name = name.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = finished(&req).await {
                log::error!("vectorcraft-web: {name} wasn't written to the browser's storage: {e} (use Download to keep a copy)");
            }
        });
        Ok(())
    }
}

/// [`BrowserFiles`] for the UI.
pub struct WebFiles(pub SharedLibrary);

impl BrowserFiles for WebFiles {
    fn list(&self) -> Vec<SavedFile> {
        self.0
            .borrow()
            .files
            .iter()
            .rev()
            .map(|f| SavedFile {
                name: f.name.clone(),
                size: f.bytes.len(),
                saved: js_sys::Date::new(&f.saved.into()).to_locale_string("default", &JsValue::UNDEFINED).into(),
            })
            .collect()
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.borrow().files.iter().find(|f| f.name == name).map(|f| f.bytes.clone())
    }

    fn remove(&mut self, name: &str) -> Result<(), String> {
        let mut lib = self.0.borrow_mut();
        let db = lib.db.clone().ok_or("this browser doesn't keep files")?;
        store(&db, web_sys::IdbTransactionMode::Readwrite)?.delete(&name.into()).map_err(js_err)?;
        lib.files.retain(|f| f.name != name);
        Ok(())
    }

    fn deliver(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        deliver(name, bytes)
    }
}

/// Hand `bytes` to the device as `name`: the share sheet on a touch device that shares files, else
/// a download.
pub fn deliver(name: &str, bytes: &[u8]) -> Result<(), String> {
    if crate::api::capture(name, bytes) {
        return Ok(());
    }
    if share(name, bytes)? {
        return Ok(());
    }
    crate::web::download(name, bytes)
}

/// Open the share sheet with the file → `false` where the page can't share it (no touch screen,
/// no Web Share, a type the browser won't share).
fn share(name: &str, bytes: &[u8]) -> Result<bool, String> {
    let window = web_sys::window().ok_or("no window")?;
    let navigator = window.navigator();
    // A Mac or a PC downloads: their share sheets are not where files are kept.
    if navigator.max_touch_points() < 2 {
        return Ok(false);
    }
    let nav: &JsValue = navigator.as_ref();
    let method = |m: &str| Reflect::get(nav, &m.into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
    let (Some(can_share), Some(share)) = (method("canShare"), method("share")) else { return Ok(false) };
    let opts = web_sys::FilePropertyBag::new();
    opts.set_type(fileio::format_for_name(name).map_or("application/octet-stream", |f| f.mime));
    let file = web_sys::File::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), name, &opts).map_err(js_err)?;
    let data = Object::new();
    Reflect::set(&data, &"files".into(), &Array::of1(&file)).map_err(js_err)?;
    if !can_share.call1(nav, &data).ok().and_then(|v| v.as_bool()).unwrap_or(false) {
        return Ok(false);
    }
    let promise = share.call1(nav, &data).map_err(js_err)?.dyn_into::<Promise>().map_err(|_| "share() answered no promise")?;
    let (name, bytes) = (name.to_string(), bytes.to_vec());
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = JsFuture::from(promise).await {
            // The sheet closed without sharing: nothing to do. Refused (no tap to share from, or a
            // sheet already open): a download instead.
            if Reflect::get(&e, &"name".into()).ok().and_then(|n| n.as_string()).as_deref() != Some("AbortError") {
                log::warn!("vectorcraft-web: sharing {name} was refused ({}): downloading it", js_err(e));
                if let Err(e) = crate::web::download(&name, &bytes) {
                    log::error!("vectorcraft-web: download of {name} failed: {e}");
                }
            }
        }
    });
    Ok(true)
}
