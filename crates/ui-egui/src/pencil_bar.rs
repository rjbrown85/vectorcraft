//! The Pencil Bar (View › Pencil Bar): a strip of finger-sized buttons over the canvas for drawing
//! with an Apple Pencil on an iPad, or any pen on a touch screen, without a keyboard.
//!
//! - The drawing tools a pen uses most, one tap each (the toolbar's buttons are mouse-sized, and
//!   its groups open on a long press).
//! - Sticky Shift and Option: tapped on, they stay held for the pen until tapped off, so a stroke
//!   can be constrained (Shift) or an object dragged as a copy (Option) with no keyboard attached.
//! - Undo and Redo (also two- and three-finger taps, [`crate::touch`]), Save, Download and the files
//!   saved in this browser.
//!
//! It shows by itself once a pen has touched the canvas in this page session
//! ([`crate::touch::pen_seen`]); View › Pencil Bar shows or hides it for good. Its buttons are
//! 44 points square, the least hit target Apple's Human Interface Guidelines give for touch
//! (<https://developer.apple.com/design/human-interface-guidelines/accessibility#Buttons-and-controls>).
//!
//! On the web, Save keeps the file in this browser (the shell's [`BrowserFiles`]) and Download
//! hands a copy to the device: the share sheet on an iPad (Save to Files, AirDrop to a Mac), a
//! download elsewhere. Saved Files lists what this browser keeps, to open, download or delete.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// A button's side, in points: Apple's least touch target.
const BUTTON: f32 = 44.0;
/// Between two buttons.
const GAP: f32 = 4.0;
/// Between two groups of buttons.
const GROUP_GAP: f32 = 12.0;
/// The bar's distance from the canvas's edge.
const MARGIN: f32 = 12.0;
/// A row of the Saved Files window and of the Download menu.
const ROW: f32 = 44.0;

/// The tools on the bar, in order.
pub const TOOLS: &[&str] = &["selection", "directSelection", "pencil", "paintbrush", "blobBrush", "pen", "eraser", "eyedropper"];

/// What Download offers: (format id, label). The VectorCraft file opens in VectorCraft on a Mac
/// with everything kept; SVG goes to any other vector app.
pub const DOWNLOAD_FORMATS: &[(&str, &str)] = &[("vectorcraft", "VectorCraft File"), ("svg", "SVG"), ("pdf", "PDF"), ("png", "PNG Image")];

/// One file kept in this browser.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedFile {
    pub name: String,
    pub size: usize,
    /// When it was saved, as the browser writes dates in its language.
    pub saved: String,
}

/// The web shell's files: those saved in this browser, and the way to hand one to the device.
pub trait BrowserFiles {
    /// The files kept, the last saved first.
    fn list(&self) -> Vec<SavedFile>;
    /// A kept file's bytes.
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    /// Forget a kept file.
    fn remove(&mut self, name: &str) -> Result<(), String>;
    /// Hand `bytes` to the device as a file named `name`: the share sheet where the device has
    /// one for files (an iPad: Save to Files, AirDrop), else a download. Call it while the user's
    /// tap is being handled: browsers open the share sheet only then.
    fn deliver(&mut self, name: &str, bytes: &[u8]) -> Result<(), String>;
}

/// The Pencil Bar's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PencilBar {
    /// Shown (`Some(true)`) or hidden (`Some(false)`) with View › Pencil Bar; `None`: it shows once a
    /// pen has been seen.
    pub show: Option<bool>,
    /// Along the top of the canvas instead of the bottom.
    pub top: bool,
    /// Shift held for the pen until tapped off.
    #[serde(skip)]
    pub shift: bool,
    /// Option (Alt) held for the pen until tapped off.
    #[serde(skip)]
    pub alt: bool,
    /// The sticky modifiers were held last frame (they are given back to the keyboard's once off).
    #[serde(skip)]
    stuck: bool,
    /// Download's format menu is open.
    #[serde(skip)]
    pub download_menu: bool,
    /// The Saved Files window is open.
    #[serde(skip)]
    pub saved_files: bool,
    /// The saved file whose Delete was tapped once (a second tap deletes it).
    #[serde(skip)]
    pub confirm_delete: Option<String>,
}

/// The bar shows: as View › Pencil Bar left it, else once a pen has been seen.
pub fn visible(app: &VectorcraftApp) -> bool {
    app.ui.pencil.show.unwrap_or_else(crate::touch::pen_seen)
}

/// The sticky modifiers held now.
fn sticky(app: &VectorcraftApp) -> egui::Modifiers {
    egui::Modifiers { shift: app.ui.pencil.shift, alt: app.ui.pencil.alt, ..egui::Modifiers::NONE }
}

/// `m` with `extra` held too.
fn with(m: egui::Modifiers, extra: egui::Modifiers) -> egui::Modifiers {
    egui::Modifiers { shift: m.shift || extra.shift, alt: m.alt || extra.alt, ..m }
}

/// Hold the sticky Shift and Option in this frame's input, after the host's own modifiers were
/// read (`VectorcraftApp::raw_input_hook`): egui keeps the modifiers of the last
/// `ModifiersChanged` (from frame to frame too), and presses, keys and wheel turns carry their own.
pub(crate) fn stick_modifiers(app: &mut VectorcraftApp, raw: &mut egui::RawInput) {
    let extra = sticky(app);
    let on = extra.any();
    if !on && !app.ui.pencil.stuck {
        return;
    }
    app.ui.pencil.stuck = on;
    for e in &mut raw.events {
        match e {
            egui::Event::ModifiersChanged(m) => *m = with(*m, extra),
            egui::Event::PointerButton { modifiers, .. } | egui::Event::Key { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => {
                *modifiers = with(*modifiers, extra);
            }
            _ => {}
        }
    }
    // First, so the frame starts with them held (or, once off, with the keyboard's alone).
    raw.events.insert(0, egui::Event::ModifiersChanged(with(app.host_modifiers, extra)));
}

/// The Pencil Bar's commands → `None` for others.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let name = || p.get("name").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "give the file's `name`".to_string());
    Some(match id {
        "view.pencilBar" => {
            let on = !visible(app);
            app.ui.pencil.show = Some(on);
            Ok(json!(on))
        }
        "view.pencilBarTop" => {
            app.ui.pencil.top = !app.ui.pencil.top;
            Ok(json!(app.ui.pencil.top))
        }
        "edit.stickyShift" => {
            app.ui.pencil.shift = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.pencil.shift);
            Ok(json!(app.ui.pencil.shift))
        }
        "edit.stickyOption" => {
            app.ui.pencil.alt = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.pencil.alt);
            Ok(json!(app.ui.pencil.alt))
        }
        "file.download" => download(app, p.get("format").and_then(Value::as_str).unwrap_or("vectorcraft")),
        "file.savedFiles" => {
            if app.services.browser_files.is_none() {
                Err("files are saved in the browser in the web app only".into())
            } else {
                app.ui.pencil.saved_files = !app.ui.pencil.saved_files;
                app.ui.pencil.confirm_delete = None;
                Ok(json!(app.ui.pencil.saved_files))
            }
        }
        "file.listSaved" => {
            files(app).map(|f| json!(f.list().iter().map(|s| json!({"name": s.name, "size": s.size, "saved": s.saved})).collect::<Vec<_>>()))
        }
        "file.openSaved" => name().and_then(|n| open_saved(app, &n)),
        "file.downloadSaved" => name().and_then(|n| download_saved(app, &n)),
        "file.deleteSaved" => name().and_then(|n| {
            files_mut(app)?.remove(&n)?;
            app.status(crate::i18n::fmt(tl!("Deleted “{name}” from this browser"), &[("name", &n)]));
            Ok(json!({ "deleted": n }))
        }),
        _ => return None,
    })
}

/// The menus' check mark of a Pencil Bar command → `None` for others.
pub(crate) fn checked(app: &VectorcraftApp, id: &str) -> Option<bool> {
    match id {
        "view.pencilBar" => Some(visible(app)),
        "view.pencilBarTop" => Some(app.ui.pencil.top),
        "edit.stickyShift" => Some(app.ui.pencil.shift),
        "edit.stickyOption" => Some(app.ui.pencil.alt),
        _ => None,
    }
}

/// Whether a Pencil Bar command can run now → `None` for others.
pub(crate) fn enabled(app: &VectorcraftApp, id: &str) -> Option<bool> {
    let web = app.services.browser_files.is_some();
    match id {
        "file.download" => Some(web && app.session.active().is_some()),
        "file.savedFiles" | "file.listSaved" | "file.openSaved" | "file.downloadSaved" | "file.deleteSaved" => Some(web),
        _ => None,
    }
}

fn files(app: &VectorcraftApp) -> Result<&dyn BrowserFiles, String> {
    app.services.browser_files.as_deref().ok_or_else(|| "files are saved in the browser in the web app only".into())
}

fn files_mut(app: &mut VectorcraftApp) -> Result<&mut Box<dyn BrowserFiles>, String> {
    app.services.browser_files.as_mut().ok_or_else(|| "files are saved in the browser in the web app only".into())
}

/// The name a download of the active document in format `f` gets: its file's name (or its title)
/// with `f`'s extension.
fn download_name(app: &VectorcraftApp, f: &fileio::Format) -> Option<String> {
    let st = app.session.active()?;
    let base = st.path.as_deref().map(fileio::file_name).unwrap_or_else(|| st.doc.title.clone());
    let stem = fileio::file_stem(&base);
    let stem = if stem.trim().is_empty() { "Untitled".to_string() } else { stem };
    Some(format!("{stem}.{}", f.extensions.first().copied().unwrap_or(f.id)))
}

/// Download (`file.download`): the active document written as `format` and handed to the device,
/// whether or not it was saved → `{name, size}`.
pub fn download(app: &mut VectorcraftApp, format: &str) -> Result<Value, String> {
    let f = fileio::format(format).filter(|f| f.write).ok_or_else(|| format!("can't download as “{format}”"))?;
    let name = download_name(app, f).ok_or("no document is open")?;
    let st = app.session.active().ok_or("no document is open")?;
    let bytes = fileio::encode(&st.doc, f.id, &json!({})).map_err(|e| e.to_string())?;
    files_mut(app)?.deliver(&name, &bytes)?;
    app.status(crate::i18n::fmt(tl!("Downloaded “{name}”"), &[("name", &name)]));
    Ok(json!({ "name": name, "size": bytes.len() }))
}

/// Open a file saved in this browser; saving it again replaces it there.
fn open_saved(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    let bytes = files(app)?.read(name).ok_or_else(|| format!("“{name}” isn't saved in this browser"))?;
    let r = crate::io::open_bytes(app, name, &bytes, Some(name.to_string()))?;
    app.ui.pencil.saved_files = false;
    Ok(r)
}

/// Hand a file saved in this browser to the device.
fn download_saved(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    let bytes = files(app)?.read(name).ok_or_else(|| format!("“{name}” isn't saved in this browser"))?;
    files_mut(app)?.deliver(name, &bytes)?;
    app.status(crate::i18n::fmt(tl!("Downloaded “{name}”"), &[("name", name)]));
    Ok(json!({ "name": name, "size": bytes.len() }))
}

/// Run `id` for a tap on the bar, showing a failure in the status bar.
fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.status(e);
    }
}

/// What a button on the bar shows.
enum Face {
    Icon(&'static str),
    Text(&'static str),
}

/// A bar button at `rect`: `on` draws it pressed. → tapped.
fn button(ui: &mut Ui, t: &Tokens, rect: Rect, face: Face, tip: &str, on: bool, enabled: bool) -> bool {
    let resp = ui.allocate_rect(rect, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if on {
        t.accent
    } else if resp.hovered() && enabled {
        t.hover
    } else {
        t.button
    };
    ui.painter().rect(rect, CornerRadius::same(8), fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    let ink = if !enabled {
        t.text_disabled
    } else if on {
        Color32::WHITE
    } else {
        t.text_strong
    };
    match face {
        Face::Icon(name) => crate::icons::paint(ui, name, Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), ink),
        Face::Text(s) => {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, tl!(s), FontId::proportional(11.0), ink);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, tip));
    resp.on_hover_text(tip).clicked()
}

/// One of the bar's buttons, laid out in groups.
#[derive(Clone, Copy)]
enum Slot {
    Tool(&'static str),
    Shift,
    Option,
    Undo,
    Redo,
    Save,
    Download,
    Saved,
    Flip,
    Hide,
}

fn groups(web: bool) -> Vec<Vec<Slot>> {
    let mut file = vec![Slot::Save];
    if web {
        file.extend([Slot::Download, Slot::Saved]);
    }
    vec![
        TOOLS.iter().map(|t| Slot::Tool(t)).collect(),
        vec![Slot::Shift, Slot::Option],
        vec![Slot::Undo, Slot::Redo],
        file,
        vec![Slot::Flip, Slot::Hide],
    ]
}

/// Where each slot goes, in rows no wider than `width`, a group kept whole on one row where it
/// fits. → (slot, its offset from the bar's top-left), the bar's size.
fn layout(groups: &[Vec<Slot>], width: f32) -> (Vec<(Slot, egui::Vec2)>, egui::Vec2) {
    let mut out = vec![];
    let (mut x, mut y, mut widest) = (0.0_f32, 0.0_f32, 0.0_f32);
    for g in groups {
        let need = g.len() as f32 * (BUTTON + GAP) - GAP;
        let lead = if x > 0.0 { GROUP_GAP } else { 0.0 };
        if x > 0.0 && x + lead + need > width {
            x = 0.0;
            y += BUTTON + GAP;
        } else {
            x += lead;
        }
        for s in g {
            if x > 0.0 && x + BUTTON > width {
                x = 0.0;
                y += BUTTON + GAP;
            }
            out.push((*s, vec2(x, y)));
            x += BUTTON + GAP;
            widest = widest.max(x - GAP);
        }
    }
    (out, vec2(widest, y + BUTTON))
}

/// Draw the bar over the canvas, and the Saved Files window when open.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    saved_files_window(app, ctx);
    if !visible(app) || crate::menus::home_showing(app) {
        app.ui.pencil.download_menu = false;
        return;
    }
    let Some(area) = app.canvas_rect else { return };
    let t = Tokens::get(ctx);
    let web = app.services.browser_files.is_some();
    let pad = 6.0;
    let (slots, size) = layout(&groups(web), (area.width() - 2.0 * (MARGIN + pad)).max(BUTTON));
    let outer = size + vec2(2.0 * pad, 2.0 * pad);
    let x = area.center().x - outer.x / 2.0;
    let y = if app.ui.pencil.top { area.top() + MARGIN } else { area.bottom() - MARGIN - outer.y };
    let at = pos2(x.max(area.left()), y.max(area.top()));
    let mut download_at = None;
    egui::Area::new(egui::Id::new("pencil-bar")).order(egui::Order::Middle).fixed_pos(at).show(ctx, |ui| {
        let frame = Rect::from_min_size(at, outer);
        ui.painter().rect(frame, CornerRadius::same(12), t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
        // The whole frame takes the pen, so a tap between two buttons doesn't reach the canvas.
        ui.allocate_rect(frame, Sense::click());
        let active = app.session.tool_id();
        let dirty_or_new = crate::menus::enabled(app, "file.save");
        let can_undo = crate::menus::enabled(app, "edit.undo");
        let can_redo = crate::menus::enabled(app, "edit.redo");
        let has_doc = app.session.active().is_some();
        for (slot, off) in slots {
            let rect = Rect::from_min_size(at + vec2(pad, pad) + off, vec2(BUTTON, BUTTON));
            match slot {
                Slot::Tool(id) => {
                    let Some(info) = vectorcraft_tools::tool_info(id) else { continue };
                    if button(ui, &t, rect, Face::Icon(crate::icons::tool_icon(info.icon)), &crate::toolbar::tip(info), active == id, has_doc) {
                        crate::toolbar::remember(app, id);
                        run(app, "tool.select", json!({ "tool": id }));
                    }
                }
                Slot::Shift => {
                    if button(ui, &t, rect, Face::Text("Shift"), tl!("Sticky Shift: held for the pen until tapped again"), app.ui.pencil.shift, true)
                    {
                        app.ui.pencil.shift = !app.ui.pencil.shift;
                    }
                }
                Slot::Option => {
                    if button(ui, &t, rect, Face::Text("Option"), tl!("Sticky Option: held for the pen until tapped again"), app.ui.pencil.alt, true)
                    {
                        app.ui.pencil.alt = !app.ui.pencil.alt;
                    }
                }
                Slot::Undo => {
                    if button(ui, &t, rect, Face::Icon("undo-2"), tl!("Undo"), false, can_undo) {
                        run(app, "edit.undo", json!({}));
                    }
                }
                Slot::Redo => {
                    if button(ui, &t, rect, Face::Icon("redo-2"), tl!("Redo"), false, can_redo) {
                        run(app, "edit.redo", json!({}));
                    }
                }
                Slot::Save => {
                    let tip = if web { tl!("Save in this browser") } else { tl!("Save") };
                    if button(ui, &t, rect, Face::Icon("save"), tip, false, dirty_or_new) {
                        run(app, "file.save", json!({}));
                    }
                }
                Slot::Download => {
                    let open = app.ui.pencil.download_menu;
                    if button(ui, &t, rect, Face::Icon("share-2"), tl!("Download a copy (Save to Files, AirDrop)"), open, has_doc) {
                        app.ui.pencil.download_menu = !open;
                    }
                    download_at = Some(rect);
                }
                Slot::Saved => {
                    if button(ui, &t, rect, Face::Icon("folder-open"), tl!("Files saved in this browser"), app.ui.pencil.saved_files, true) {
                        run(app, "file.savedFiles", json!({}));
                    }
                }
                Slot::Flip => {
                    let tip = if app.ui.pencil.top { tl!("Move the bar to the bottom") } else { tl!("Move the bar to the top") };
                    if button(ui, &t, rect, Face::Icon("arrow-up-down"), tip, false, true) {
                        app.ui.pencil.top = !app.ui.pencil.top;
                    }
                }
                Slot::Hide => {
                    if button(ui, &t, rect, Face::Icon("x"), tl!("Hide the Pencil Bar (View › Pencil Bar shows it)"), false, true) {
                        app.ui.pencil.show = Some(false);
                        app.ui.pencil.download_menu = false;
                    }
                }
            }
        }
    });
    if let Some(anchor) = download_at.filter(|_| app.ui.pencil.download_menu) {
        download_menu(app, ctx, anchor);
    }
}

/// Download's formats, in a list over (or under) its button: a tap downloads in that format.
fn download_menu(app: &mut VectorcraftApp, ctx: &egui::Context, anchor: Rect) {
    let t = Tokens::get(ctx);
    let w = 200.0;
    let h = DOWNLOAD_FORMATS.len() as f32 * ROW + 8.0;
    let x = (anchor.center().x - w / 2.0).max(ctx.content_rect().left() + 4.0);
    let y = if app.ui.pencil.top { anchor.bottom() + 12.0 } else { anchor.top() - 12.0 - h };
    let at = pos2(x, y);
    let mut picked = None;
    let r = egui::Area::new(egui::Id::new("pencil-bar-download")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        let frame = Rect::from_min_size(at, vec2(w, h));
        ui.painter().rect(frame, CornerRadius::same(10), t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
        for (i, (id, label)) in DOWNLOAD_FORMATS.iter().enumerate() {
            let row = Rect::from_min_size(at + vec2(4.0, 4.0 + i as f32 * ROW), vec2(w - 8.0, ROW));
            let resp = ui.allocate_rect(row, Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(row, CornerRadius::same(6), t.hover);
            }
            ui.painter().text(row.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, tl!(label), FontId::proportional(15.0), t.text_strong);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!(label)));
            if resp.clicked() {
                picked = Some(*id);
            }
        }
    });
    if let Some(format) = picked {
        app.ui.pencil.download_menu = false;
        run(app, "file.download", json!({ "format": format }));
    } else if r.response.clicked_elsewhere()
        && !ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| anchor.contains(p)))
    {
        app.ui.pencil.download_menu = false;
    }
}

/// "12 KB": a file's size.
fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// The Saved Files window: what this browser keeps, each to open, download or delete.
fn saved_files_window(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.ui.pencil.saved_files {
        return;
    }
    let Some(list) = app.services.browser_files.as_ref().map(|f| f.list()) else {
        app.ui.pencil.saved_files = false;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut action: Option<(&'static str, String)> = None;
    egui::Window::new(tl!("Saved in This Browser"))
        .id(egui::Id::new("pencil-saved-files"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(tl!("Save keeps your work here. Download sends a copy to Files or AirDrop, to finish it on your Mac."))
                    .color(t.text_dim),
            );
            ui.add_space(6.0);
            if list.is_empty() {
                ui.add_space(12.0);
                ui.label(tl!("Nothing is saved in this browser yet."));
                ui.add_space(12.0);
                return;
            }
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.6).show(ui, |ui| {
                for f in &list {
                    ui.horizontal(|ui| {
                        ui.set_min_height(ROW);
                        ui.vertical(|ui| {
                            ui.set_width(200.0);
                            ui.label(egui::RichText::new(&f.name).strong());
                            ui.label(egui::RichText::new(format!("{} · {}", size_label(f.size), f.saved)).color(t.text_dim).small());
                        });
                        let big = |s: &str| egui::Button::new(s).min_size(vec2(64.0, 36.0));
                        if ui.add(big(tl!("Open"))).clicked() {
                            action = Some(("file.openSaved", f.name.clone()));
                        }
                        if ui.add(big(tl!("Download"))).clicked() {
                            action = Some(("file.downloadSaved", f.name.clone()));
                        }
                        let confirming = app.ui.pencil.confirm_delete.as_deref() == Some(f.name.as_str());
                        let del = if confirming { tl!("Delete?") } else { tl!("Delete") };
                        let b = big(del).fill(if confirming { Color32::from_rgb(0xc0, 0x39, 0x2b) } else { t.button });
                        if ui.add(b).clicked() {
                            if confirming {
                                action = Some(("file.deleteSaved", f.name.clone()));
                            } else {
                                app.ui.pencil.confirm_delete = Some(f.name.clone());
                            }
                        }
                    });
                    ui.separator();
                }
            });
        });
    if !open {
        app.ui.pencil.saved_files = false;
        app.ui.pencil.confirm_delete = None;
    }
    if let Some((id, name)) = action {
        if id == "file.deleteSaved" {
            app.ui.pencil.confirm_delete = None;
        }
        run(app, id, json!({ "name": name }));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::Services;
    use vectorcraft_engine::Session;

    /// Files kept in memory, and what was delivered.
    #[derive(Default)]
    struct Memory {
        files: Vec<(String, Vec<u8>)>,
        delivered: Rc<RefCell<Vec<(String, usize)>>>,
    }

    impl BrowserFiles for Memory {
        fn list(&self) -> Vec<SavedFile> {
            self.files.iter().rev().map(|(n, b)| SavedFile { name: n.clone(), size: b.len(), saved: "today".into() }).collect()
        }
        fn read(&self, name: &str) -> Option<Vec<u8>> {
            self.files.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone())
        }
        fn remove(&mut self, name: &str) -> Result<(), String> {
            self.files.retain(|(n, _)| n != name);
            Ok(())
        }
        fn deliver(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
            self.delivered.borrow_mut().push((name.to_string(), bytes.len()));
            Ok(())
        }
    }

    fn web_app() -> (VectorcraftApp, Rc<RefCell<Vec<(String, usize)>>>) {
        let mem = Memory::default();
        let delivered = mem.delivered.clone();
        let services = Services { browser_files: Some(Box::new(mem)), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        (app, delivered)
    }

    #[test]
    fn download_delivers_the_document_in_each_format_without_saving_it() {
        let (mut app, delivered) = web_app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
        for (format, _) in DOWNLOAD_FORMATS {
            let r = app.run("file.download", json!({ "format": format })).unwrap();
            assert!(r["name"].as_str().unwrap().ends_with(&format!(".{}", fileio::format(format).unwrap().extensions[0])), "{r}");
        }
        let got = delivered.borrow();
        assert_eq!(got.len(), DOWNLOAD_FORMATS.len());
        assert!(got.iter().all(|(_, size)| *size > 0));
        assert!(app.session.active().unwrap().is_dirty(), "a download is a copy: the document stays unsaved");
        assert!(app.run("file.download", json!({"format": "nope"})).is_err());
    }

    #[test]
    fn saved_files_open_download_and_delete() {
        let (mut app, delivered) = web_app();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"/></svg>"#;
        // Kept by the shell's save; the test puts one in by hand.
        let mem = Memory { files: vec![("logo.svg".into(), svg.to_vec())], delivered: delivered.clone() };
        app.services.browser_files = Some(Box::new(mem));
        let list = app.run("file.listSaved", json!({})).unwrap();
        assert_eq!(list[0]["name"], "logo.svg");
        let docs = app.session.documents().len();
        app.run("file.openSaved", json!({"name": "logo.svg"})).unwrap();
        assert_eq!(app.session.documents().len(), docs + 1);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("logo.svg"), "saving again replaces it in the browser");
        app.run("file.downloadSaved", json!({"name": "logo.svg"})).unwrap();
        assert_eq!(delivered.borrow().as_slice(), [("logo.svg".to_string(), svg.len())]);
        app.run("file.deleteSaved", json!({"name": "logo.svg"})).unwrap();
        assert_eq!(app.run("file.listSaved", json!({})).unwrap(), json!([]));
        assert!(app.run("file.openSaved", json!({"name": "logo.svg"})).is_err());
    }

    #[test]
    fn web_saves_stay_in_the_browser_and_exports_download() {
        let kept = Rc::new(RefCell::new(Vec::<String>::new()));
        let downloaded = Rc::new(RefCell::new(Vec::<String>::new()));
        let (k, d) = (kept.clone(), downloaded.clone());
        let services = Services {
            keep: Some(Box::new(move |n: &str, _: &[u8]| {
                k.borrow_mut().push(n.to_string());
                Ok(())
            })),
            download: Some(Box::new(move |n: &str, _: &[u8]| {
                d.borrow_mut().push(n.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("file.save", json!({"path": "Sketch.vectorcraft"})).unwrap();
        assert_eq!(*kept.borrow(), ["Sketch.vectorcraft"]);
        assert!(downloaded.borrow().is_empty(), "Save doesn't download");
        assert!(!app.session.active().unwrap().is_dirty());
        app.run("shape.rectangle", json!({"x": 40, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("file.save", json!({})).unwrap();
        assert_eq!(*kept.borrow(), ["Sketch.vectorcraft", "Sketch.vectorcraft"], "saving again replaces it");
        // Anything else that is written (an export) still downloads.
        crate::io::write_to(&mut app.services, "Sketch.png", b"png").unwrap();
        assert_eq!(*downloaded.borrow(), ["Sketch.png"]);
    }

    #[test]
    fn desktop_has_no_browser_files() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        assert!(!crate::menus::enabled(&app, "file.download") && !crate::menus::enabled(&app, "file.savedFiles"));
        assert!(app.run("file.download", json!({})).is_err());
    }

    #[test]
    fn sticky_modifiers_join_the_keyboards_and_let_go() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        let pos = egui::pos2(5.0, 5.0);
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        app.raw_input_hook(&mut raw);
        assert_eq!(raw.events.len(), 1, "nothing sticky: the input is left alone");
        app.run("edit.stickyShift", json!({})).unwrap();
        app.run("edit.stickyOption", json!({"on": true})).unwrap();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        raw.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        app.raw_input_hook(&mut raw);
        assert!(matches!(raw.events[0], egui::Event::ModifiersChanged(m) if m.shift && m.alt));
        assert!(matches!(raw.events[1], egui::Event::ModifiersChanged(m) if m.shift && m.alt && m.command), "the keyboard's Cmd is kept");
        assert!(matches!(raw.events[2], egui::Event::PointerButton { modifiers, .. } if modifiers.shift && modifiers.alt));
        assert_eq!((menus_checked(&app, "edit.stickyShift"), menus_checked(&app, "edit.stickyOption")), (Some(true), Some(true)));
        // Tapped off: one frame gives the keyboard's modifiers back, then the input is left alone.
        app.run("edit.stickyShift", json!({})).unwrap();
        app.run("edit.stickyOption", json!({})).unwrap();
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&mut raw);
        assert!(matches!(raw.events.as_slice(), [egui::Event::ModifiersChanged(m)] if !m.shift && !m.alt));
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&mut raw);
        assert!(raw.events.is_empty());
    }

    fn menus_checked(app: &VectorcraftApp, id: &str) -> Option<bool> {
        crate::menus::checked(app, id, &json!({}))
    }

    #[test]
    fn the_bar_shows_once_a_pen_is_seen_unless_hidden() {
        crate::touch::set_pen_seen(false);
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        assert!(!visible(&app));
        crate::touch::set_pen_seen(true);
        assert!(visible(&app));
        assert_eq!(app.run("view.pencilBar", json!({})).unwrap(), json!(false));
        assert!(!visible(&app), "hidden for good, pen or not");
        assert_eq!(app.run("view.pencilBar", json!({})).unwrap(), json!(true));
        crate::touch::set_pen_seen(false);
        assert!(visible(&app), "shown for good");
    }

    #[test]
    fn layout_wraps_whole_groups_into_rows() {
        let g = groups(true);
        let count: usize = g.iter().map(Vec::len).sum();
        let (wide, size) = layout(&g, 2000.0);
        assert_eq!(wide.len(), count);
        assert_eq!(size.y, BUTTON, "one row when it fits");
        let (narrow, size) = layout(&g, 500.0);
        assert!(size.x <= 500.0 && size.y > BUTTON);
        assert!(narrow.iter().all(|(_, off)| off.x + BUTTON <= 500.0));
        // The undo pair starts and ends on one row.
        let undo = narrow.iter().position(|(s, _)| matches!(s, Slot::Undo)).unwrap();
        assert_eq!(narrow[undo].1.y, narrow[undo + 1].1.y);
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!((size_label(900), size_label(2048), size_label(3 * 1024 * 1024)), ("900 B".into(), "2 KB".into(), "3.0 MB".into()));
    }
}
