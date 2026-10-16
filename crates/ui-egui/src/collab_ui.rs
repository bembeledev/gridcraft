//! Live sessions (co-editing, plugin protocol 2) in the interface: the engine's pump each frame,
//! Share and the people in the title bar (with the session's popover), the Share dialog, others'
//! selections drawn in their colours with name tags (fading after they stop moving), others' edits briefly highlighted, and the
//! room's state kept for recovery.
//!
//! The engine does the work (`gridcraft_engine::collab`); this only shows it.

use egui::{Align2, Color32, Painter, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::collab::RoomState;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::Sheet;
use serde_json::json;

use crate::SheetApp;
use crate::grid::Geo;
use crate::theme::{self, Tokens};

/// How long others' edits stay highlighted (milliseconds).
const HIGHLIGHT_MS: f64 = 2_000.0;
/// How long a person's name tag stays after they last moved their selection, then how long it
/// takes to fade out (milliseconds).
const TAG_MS: f64 = 3_000.0;
const TAG_FADE_MS: f64 = 400.0;

/// Where a person's selection was, and when it last moved (for their name tag).
#[derive(Clone, Debug, PartialEq)]
pub struct Moved {
    pub selection: Option<(usize, Vec<RangeRef>)>,
    pub cursor: Option<(usize, CellRef)>,
    pub at: f64,
}

/// How often the rooms' state is written for recovery.
const RECOVERY_MS: f64 = 30_000.0;

fn color(hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x80_80_80);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Runs the engine's co-editing between frames.
pub fn pump(app: &mut SheetApp, ctx: &egui::Context) {
    if app.session.collab.rooms.is_empty() && !gridcraft_engine::plugin::push::pending() {
        return;
    }
    let now = crate::now_ms();
    if app.session.collab_poll(now.max(0.0) as u64) {
        ctx.request_repaint();
    }
    for (doc, sheet, cell) in app.session.collab.take_remote_edits() {
        app.collab_highlights.push((doc, sheet, cell, now));
    }
    app.collab_highlights.retain(|h| now - h.3 < HIGHLIGHT_MS);
    if app.collab_highlights.len() > 2_000 {
        let extra = app.collab_highlights.len() - 2_000;
        app.collab_highlights.drain(..extra);
    }
    note_moves(app, now);
    for n in app.session.collab_take_notices() {
        app.status_message = Some((n, now));
    }
    if now - app.collab_recovery_at > RECOVERY_MS {
        app.collab_recovery_at = now;
        app.session.save_collab_recovery();
    }
    app.after_engine();
    if !app.session.collab.rooms.is_empty() {
        // Remote messages arrive between frames: keep looking.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Notes when each person in the active room last moved their selection.
fn note_moves(app: &mut SheetApp, now: f64) {
    let peers = app.session.active_room().map(|r| r.peers()).unwrap_or_default();
    app.collab_moved.retain(|id, _| peers.iter().any(|p| p.client_id == *id));
    for p in peers {
        let moved = app.collab_moved.get(&p.client_id).is_none_or(|m| m.selection != p.selection || m.cursor != p.cursor);
        if moved {
            app.collab_moved.insert(p.client_id, Moved { selection: p.selection, cursor: p.cursor, at: now });
        }
    }
}

/// Why a workbook in a live session can't be edited now, or what went wrong, for the status bar.
/// The session's state and its people are in the title bar ([`share_button`], [`people`]).
pub fn status(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(room) = app.session.active_room() else { return };
    let why = gridcraft_engine::collab::read_only(&app.session);
    let problem = room.refused.clone().or_else(|| room.message.clone());
    if let Some(why) = why.filter(|w| Some(w) != problem.as_ref()) {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(why).font(theme::ui_font(12.5)).color(t.text_dim));
    }
    if let Some(m) = problem {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(m).font(theme::ui_font(12.5)).color(t.danger));
    }
}

// ---------------------------------------------------------------- the title bar

/// What the title bar's Share button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareState {
    /// Not in a live session: Share opens the Share dialog.
    Off,
    /// Connecting, or joining (waiting for the room's document).
    Connecting,
    Online,
    /// The transport is offline or gone; editing goes on here.
    Offline,
}

impl ShareState {
    /// The button's accessible name (its tooltip says more).
    pub fn label(self) -> &'static str {
        match self {
            ShareState::Off => "Share",
            ShareState::Connecting => "Share, connecting",
            ShareState::Online => "Share, live",
            ShareState::Offline => "Share, offline",
        }
    }
    fn says(self) -> &'static str {
        match self {
            ShareState::Off => "Share this workbook in a live session, or join one",
            ShareState::Connecting => "Live session: connecting…",
            ShareState::Online => "Live session: everyone's edits arrive as they happen",
            ShareState::Offline => "Live session: offline. Editing goes on here and merges when it's back",
        }
    }
    fn color(self, t: &Tokens) -> Color32 {
        match self {
            ShareState::Off => t.text_dim,
            ShareState::Connecting => t.presence_connecting,
            ShareState::Online => t.presence_online,
            ShareState::Offline => t.text_disabled,
        }
    }
}

/// The active workbook's live session, as the Share button shows it.
pub fn share_state(app: &SheetApp) -> ShareState {
    match app.session.active_room() {
        None => ShareState::Off,
        Some(r) if r.joining => ShareState::Connecting,
        Some(r) => match r.state {
            RoomState::Connecting => ShareState::Connecting,
            RoomState::Online => ShareState::Online,
            RoomState::Offline | RoomState::Ended => ShareState::Offline,
        },
    }
}

/// Someone in the active workbook's live session, as the title bar shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: String,
    pub color: Color32,
    /// This is the person using this GridCraft.
    pub you: bool,
    /// Their awareness client id (`None` for yourself).
    pub client: Option<u64>,
}

/// The people in the active workbook's live session: the others by name, then yourself.
pub fn people(app: &SheetApp) -> Vec<Person> {
    let Some(room) = app.session.active_room() else { return vec![] };
    let mut others: Vec<Person> = room
        .peers()
        .into_iter()
        .map(|p| Person { name: p.user.name.clone(), color: color(&p.user.color), you: false, client: Some(p.client_id) })
        .collect();
    others.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.client.cmp(&b.client)));
    others.push(Person { name: room.user.name.clone(), color: color(&room.user.color), you: true, client: None });
    others
}

/// Up to two initials: the first letters of the first and last words ("Ann Lee" → "AL").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_alphanumeric)).collect();
    let first = |w: &&str| w.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    match (words.first(), words.len()) {
        (None, _) => name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into()),
        (Some(w), 1) => first(w),
        (Some(w), _) => format!("{}{}", first(w), words.last().map(first).unwrap_or_default()),
    }
}

/// Black or white, whichever reads better on `bg` (by its relative luminance).
pub fn text_on(bg: Color32) -> Color32 {
    let lin = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let l = 0.2126 * lin(bg.r()) + 0.7152 * lin(bg.g()) + 0.0722 * lin(bg.b());
    // Contrast with white is 1.05 / (l + 0.05), with black (l + 0.05) / 0.05; they're equal at
    // l ≈ 0.18. White reads better on mid tones than that suggests (bold initials on a saturated
    // colour), so it keeps colours up to l = 0.4 (contrast ≥ 2.3; bright colours get black).
    if l <= 0.4 { Color32::WHITE } else { Color32::BLACK }
}

/// How many circles to draw (yourself included) and how many more people the "+N" stands for,
/// for `total` people and room for `slots` circles ("+N" takes one). At most four circles.
pub fn circles(total: usize, slots: usize) -> (usize, usize) {
    if slots == 0 {
        return (0, 0);
    }
    if total <= slots.min(MAX_CIRCLES) {
        return (total, 0);
    }
    let shown = (slots - 1).min(MAX_CIRCLES);
    (shown, total - shown)
}

/// Circles shown before "+N".
pub const MAX_CIRCLES: usize = 4;
/// A circle's diameter, and how far each one is from the previous (they overlap).
pub const CIRCLE: f32 = if cfg!(target_os = "macos") { 22.0 } else { 26.0 };
/// A light overlap: the next circle's ring covers only the edge of the previous one, so two
/// initials stay clear of it.
pub const CIRCLE_STEP: f32 = CIRCLE - 1.5;

/// The width the people's circles take: `shown` circles and a "+N" when `more` > 0.
pub fn people_width(shown: usize, more: usize) -> f32 {
    let n = shown + usize::from(more > 0);
    if n == 0 { 0.0 } else { CIRCLE + CIRCLE_STEP * (n - 1) as f32 }
}

fn person_circle(ui: &Ui, center: egui::Pos2, c: Color32, text: &str, ring: Color32) {
    let r = CIRCLE / 2.0;
    ui.painter().circle_filled(center, r + 1.5, ring);
    ui.painter().circle_filled(center, r, c);
    let size = if text.chars().count() > 1 { CIRCLE * 0.38 } else { CIRCLE * 0.46 };
    ui.painter().text(center, Align2::CENTER_CENTER, text, theme::ui_bold(size), text_on(c));
}

/// The people in the session as overlapping circles (the others, "+N", then yourself), at most
/// `slots` circles. Clicking one shows their selection.
pub fn people_circles(app: &mut SheetApp, ui: &mut Ui, slots: usize) {
    let t = Tokens::get(ui.ctx());
    let everyone = people(app);
    if everyone.is_empty() {
        return;
    }
    let (shown, more) = circles(everyone.len(), slots);
    if shown + more == 0 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(people_width(shown, more), CIRCLE + 3.0), egui::Sense::hover());
    // The others shown, "+N", then you (unless only "+N" fits).
    let others_shown = shown.saturating_sub(1);
    let others: Vec<&Person> = everyone.iter().filter(|p| !p.you).collect();
    let you = everyone.iter().find(|p| p.you).filter(|_| shown > 0);
    let mut x = rect.left() + CIRCLE / 2.0;
    let y = rect.center().y;
    let mut clicked = None;
    for p in others.iter().take(others_shown) {
        let center = pos2(x, y);
        person_circle(ui, center, p.color, &initials(&p.name), t.window);
        let id = ui.id().with(("person", p.client));
        let resp = ui.interact(Rect::from_center_size(center, vec2(CIRCLE, CIRCLE)), id, egui::Sense::click());
        let name = p.name.clone();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        if resp.on_hover_text(&p.name).clicked() {
            clicked = Some((*p).clone());
        }
        x += CIRCLE_STEP;
    }
    if more > 0 {
        let center = pos2(x, y);
        let label = format!("+{more}");
        person_circle(ui, center, t.hover, "", t.window);
        ui.painter().circle_stroke(center, CIRCLE / 2.0, Stroke::new(1.0, t.separator));
        ui.painter().text(center, Align2::CENTER_CENTER, &label, theme::ui_bold(CIRCLE * 0.38), t.text);
        let resp = ui.interact(Rect::from_center_size(center, vec2(CIRCLE, CIRCLE)), ui.id().with("people_more"), egui::Sense::hover());
        let mut hidden: Vec<String> = others.iter().skip(others_shown).map(|p| p.name.clone()).collect();
        if shown == 0
            && let Some(p) = everyone.iter().find(|p| p.you)
        {
            hidden.push(format!("{} (you)", p.name));
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
        resp.on_hover_text(hidden.join("\n"));
        x += CIRCLE_STEP;
    }
    if let Some(p) = you {
        let center = pos2(x, y);
        person_circle(ui, center, p.color, &initials(&p.name), t.window);
        let resp = ui.interact(Rect::from_center_size(center, vec2(CIRCLE, CIRCLE)), ui.id().with("person_you"), egui::Sense::click());
        let name = format!("{} (you)", p.name);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        if resp.on_hover_text(&name).clicked() {
            clicked = Some(p.clone());
        }
    }
    if let Some(p) = clicked {
        reveal(app, &p);
    }
}

/// How long a person's selection flashes after a click on them (milliseconds).
const FLASH_MS: f64 = 1_400.0;

/// A person's selection flashing: (document uid, sheet, ranges, their colour, since when).
#[derive(Clone, Debug)]
pub struct Flash {
    pub doc: u64,
    pub sheet: usize,
    pub ranges: Vec<RangeRef>,
    pub color: Color32,
    pub at: f64,
}

/// Scrolls to `p`'s selection (switching sheet if needed) and flashes it.
pub fn reveal(app: &mut SheetApp, p: &Person) {
    let Some(d) = app.session.active() else { return };
    let doc = d.uid;
    let target = if p.you {
        Some((d.wb.active_sheet, d.selection.ranges.clone()))
    } else {
        let Some(room) = app.session.active_room() else { return };
        room.peers().into_iter().find(|x| Some(x.client_id) == p.client).and_then(|peer| match (peer.selection, peer.cursor) {
            (Some(sel), _) => Some(sel),
            (None, Some((si, cell))) => Some((si, vec![RangeRef::cell(cell)])),
            _ => None,
        })
    };
    let Some((sheet, ranges)) = target.filter(|(_, r)| !r.is_empty()) else {
        app.status_message = Some((format!("{} hasn't selected anything yet", p.name), crate::now_ms()));
        return;
    };
    if app.session.active().is_some_and(|d| d.wb.active_sheet != sheet) {
        app.run_or_alert("sheet.activate", json!({"sheet": sheet}));
    }
    if p.you {
        app.grid.ensure_visible = true;
    } else {
        app.grid.reveal = ranges.first().map(|r| r.start);
    }
    app.collab_flash = Some(Flash { doc, sheet, ranges, color: p.color, at: crate::now_ms() });
}

/// The Share button's width.
pub fn share_button_width(app: &SheetApp, ui: &Ui) -> f32 {
    let label = app.ui.language.tr("Share").to_string();
    ui.painter().layout_no_wrap(label, theme::ui_font(12.5), Color32::WHITE).size().x + 38.0
}

/// The title bar's Share button: not in a session, it opens the Share dialog; in one, it shows
/// the session's state and opens the session's popover.
pub fn share_button(app: &mut SheetApp, ui: &mut Ui, h: f32) {
    let t = Tokens::get(ui.ctx());
    let state = share_state(app);
    let font = theme::ui_font(12.5);
    let label = app.ui.language.tr("Share").to_string();
    let (rect, resp) = ui.allocate_exact_size(vec2(share_button_width(app, ui), h - 2.0), egui::Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        t.ribbon
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    let mark = pos2(rect.left() + 15.0, rect.center().y);
    if state == ShareState::Off {
        crate::icons::paint(ui.painter(), Rect::from_center_size(mark, vec2(16.0, 16.0)), crate::icons::Icon::Share, t.text);
    } else {
        let c = state.color(&t);
        ui.painter().circle_filled(mark, 4.5, c);
        if state == ShareState::Connecting {
            // A ring around the dot while it connects.
            ui.painter().circle_stroke(mark, 6.5, Stroke::new(1.0, c.gamma_multiply(0.6)));
        }
    }
    ui.painter().text(pos2(rect.left() + 27.0, rect.center().y), Align2::LEFT_CENTER, &label, font, t.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, state.label()));
    let resp = resp.on_hover_text(state.says());
    if state == ShareState::Off {
        if resp.clicked() {
            app.run_or_alert("collab.share", json!({}));
        }
        return;
    }
    egui::Popup::menu(&resp).id(egui::Id::new("live_session_popover")).width(260.0).show(|ui| session_popover(app, ui, state));
}

/// The session's popover: the room, who's here, Copy invitation and Leave.
fn session_popover(app: &mut SheetApp, ui: &mut Ui, state: ShareState) {
    let t = Tokens::get(ui.ctx());
    let Some(room) = app.session.active_room() else { return };
    let room_id = room.room.clone();
    let title = room.title().filter(|x| !x.is_empty()).or_else(|| app.session.active().map(|d| d.display_title())).unwrap_or_default();
    let host = app.host_session.clone();
    let transport = match &host {
        Some(h) if !h.label.is_empty() => h.label.clone(),
        _ => gridcraft_engine::plugin::get(&room.plugin)
            .map(|p| p.manifest.collab.as_ref().map(|c| c.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| p.manifest.name.clone()))
            .unwrap_or_else(|| room.plugin.clone()),
    };
    let encrypted = room.encrypted();
    let problem = room.refused.clone().or_else(|| room.message.clone());
    let joining = room.joining;
    let view_only = room.view_only;
    ui.set_min_width(240.0);
    let title = if title.is_empty() { "Live session".to_string() } else { title };
    ui.add(egui::Label::new(egui::RichText::new(&title).font(theme::ui_bold(13.0)).color(t.text)).truncate()).on_hover_text(&title);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, state.color(&t));
        let what = match state {
            _ if joining => "Joining…",
            ShareState::Online => "Live",
            ShareState::Connecting => "Connecting…",
            _ => "Offline: editing goes on here",
        };
        ui.label(egui::RichText::new(what).font(theme::ui_font(12.5)).color(t.text_dim));
    });
    ui.label(egui::RichText::new(format!("Room {room_id} · {transport}")).font(theme::ui_font(11.5)).color(t.text_dim));
    let mut notes = vec![];
    if encrypted {
        notes.push("Encrypted end to end");
    }
    if view_only {
        notes.push("You can view, not edit");
    }
    if !notes.is_empty() {
        ui.label(egui::RichText::new(notes.join(" · ")).font(theme::ui_font(11.5)).color(t.text_dim));
    }
    if let Some(m) = problem {
        ui.label(egui::RichText::new(m).font(theme::ui_font(11.5)).color(t.danger));
    }
    for line in host.iter().flat_map(|h| h.lines.iter()) {
        ui.label(egui::RichText::new(line).font(theme::ui_font(11.5)).color(t.text_dim));
    }
    ui.separator();
    let everyone = people(app);
    ui.label(egui::RichText::new(format!("In this session ({})", everyone.len())).font(theme::ui_font(11.5)).color(t.text_dim));
    let mut clicked = None;
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for p in &everyone {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover);
            }
            let center = pos2(r.left() + 4.0 + CIRCLE / 2.0 * 0.8, r.center().y);
            let small = CIRCLE * 0.8;
            ui.painter().circle_filled(center, small / 2.0, p.color);
            ui.painter().text(center, Align2::CENTER_CENTER, initials(&p.name), theme::ui_bold(small * 0.38), text_on(p.color));
            let name = if p.you { format!("{} (you)", p.name) } else { p.name.clone() };
            ui.painter().text(pos2(center.x + small / 2.0 + 8.0, r.center().y), Align2::LEFT_CENTER, &name, theme::ui_font(12.5), t.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
            if resp.on_hover_text("Show their selection").clicked() {
                clicked = Some(p.clone());
            }
        }
    });
    ui.separator();
    if let Some(link) = app.collab_invites.get(&room_id).cloned()
        && ui.button(app.ui.language.tr("Copy invitation")).on_hover_text("Copy the invitation link the add-in made, to send to others").clicked()
    {
        ui.ctx().copy_text(link);
        app.status_message = Some(("Invitation copied".into(), crate::now_ms()));
    }
    if host.is_some() {
        // The hosting page owns the session: closing the document is how one leaves it.
        if app.services.close.is_some()
            && ui.button(app.ui.language.tr("Close")).on_hover_text("Close the document (changes are saved in Nextcloud)").clicked()
        {
            app.run_or_alert("file.close", json!({}));
        }
    } else if ui.button(app.ui.language.tr("Leave")).on_hover_text("Leave the live session (the workbook stays open with every edit)").clicked() {
        app.run_or_alert("collab.leave", json!({}));
    }
    if let Some(p) = clicked {
        reveal(app, &p);
    }
}

/// Others' selections (outlined in their colours, with their names) and their recent edits.
pub fn paint_remote(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, rows: (u32, u32), cols: (u32, u32)) {
    let Some(d) = app.session.active() else { return };
    let si = d.wb.active_sheet;
    let visible = |r: &RangeRef| r.start.row <= rows.1 && r.end.row >= rows.0 && r.start.col <= cols.1 && r.end.col >= cols.0;
    let now = crate::now_ms();
    for (doc, sheet, cell, at) in &app.collab_highlights {
        let r = RangeRef::cell(*cell);
        if *doc != d.uid || *sheet != si || !visible(&r) {
            continue;
        }
        let fade = (1.0 - (now - at) / HIGHLIGHT_MS).clamp(0.0, 1.0);
        let rect = geo.cell_rect(sh, *cell);
        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(255, 196, 0, (90.0 * fade) as u8));
    }
    // A person's selection flashing after a click on them in the title bar: two pulses.
    if let Some(f) = app.collab_flash.as_ref().filter(|f| f.doc == d.uid && f.sheet == si && now - f.at < FLASH_MS) {
        let k = ((now - f.at) / FLASH_MS).clamp(0.0, 1.0) as f32;
        let pulse = (0.5 - 0.5 * (k * std::f32::consts::TAU * 2.0).cos()) * (1.0 - k * 0.5);
        for r in f.ranges.iter().filter(|r| visible(r)) {
            let rect = geo.range_rect(sh, *r);
            p.rect_filled(rect, 0.0, f.color.gamma_multiply(0.35 * pulse));
            p.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.0 + 2.0 * pulse, f.color), StrokeKind::Outside);
        }
        p.ctx().request_repaint();
    }
    let Some(room) = app.session.active_room() else { return };
    for peer in room.peers() {
        let c = color(&peer.user.color);
        if let Some((s, ranges)) = &peer.selection
            && *s == si
        {
            for r in ranges.iter().filter(|r| visible(r)) {
                let rect = geo.range_rect(sh, *r);
                p.rect_filled(rect, 0.0, c.gamma_multiply(0.12));
                p.rect_stroke(rect, 0.0, Stroke::new(2.0, c), StrokeKind::Inside);
            }
        }
        let corner = match (&peer.cursor, &peer.selection) {
            (Some((s, cell)), _) if *s == si => Some(*cell),
            (_, Some((s, ranges))) if *s == si => ranges.first().map(|r| r.start),
            _ => None,
        };
        if let Some(cell) = corner.filter(|c| visible(&RangeRef::cell(*c))) {
            p.rect_stroke(geo.cell_rect(sh, cell), 0.0, Stroke::new(2.0, c), StrokeKind::Inside);
        }
    }
}

/// How visible a name tag is: fully for [`TAG_MS`] after the person last moved, then fading out
/// over [`TAG_FADE_MS`]; fully again while the pointer is over their outline.
pub fn tag_alpha(since_move_ms: f64, hovered: bool) -> f32 {
    if hovered {
        return 1.0;
    }
    (1.0 - (since_move_ms - TAG_MS) / TAG_FADE_MS).clamp(0.0, 1.0) as f32
}

/// Others' name tags, painted after the headers: above the top edge of their selection (the
/// range holding their active cell), in the column headers when it's at the top of the view, so
/// they don't cover the cells. A tag fades out a few seconds after its person stops moving and
/// comes back while the pointer is over their outline. `quads` are the panes (rows, columns,
/// clip) as painted.
pub fn paint_remote_tags(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, quads: &[((u32, u32), (u32, u32), Rect)]) {
    let Some(d) = app.session.active() else { return };
    let si = d.wb.active_sheet;
    let Some(room) = app.session.active_room() else { return };
    let now = crate::now_ms();
    let pointer = p.ctx().pointer_hover_pos().map(|q| geo.mirror_pos(q)).filter(|q| geo.cells.contains(*q));
    let area = Rect::from_min_max(pos2(geo.cells.left(), geo.cells.top() - geo.header_h), geo.cells.max);
    let p = p.with_clip_rect(area);
    // A range's visible part: its top-left-most piece over the panes.
    let shown = |r: &RangeRef| -> Option<Rect> {
        quads
            .iter()
            .filter(|(rows, cols, _)| r.start.row <= rows.1 && r.end.row >= rows.0 && r.start.col <= cols.1 && r.end.col >= cols.0)
            .map(|(_, _, clip)| geo.range_rect(sh, *r).intersect(*clip))
            .filter(|v| v.is_positive())
            .min_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())))
    };
    for peer in room.peers() {
        let Some((s, ranges)) = &peer.selection else { continue };
        if *s != si {
            continue;
        }
        // Over the outline: near the edge of one of their ranges (a small one: anywhere on it).
        let hovered = pointer.is_some_and(|q| {
            ranges
                .iter()
                .filter_map(&shown)
                .any(|v| v.expand(4.0).contains(q) && (v.width() < 40.0 || v.height() < 30.0 || !v.shrink(4.0).contains(q)))
        });
        let since = app.collab_moved.get(&peer.client_id).map_or(0.0, |m| now - m.at);
        let alpha = tag_alpha(since, hovered);
        if alpha <= 0.0 {
            continue;
        }
        let active = peer.cursor.filter(|(cs, _)| *cs == si).map(|(_, c)| c);
        let Some(anchor) = active.and_then(|c| ranges.iter().find(|r| r.contains(c))).or(ranges.first()) else { continue };
        let Some(vis) = shown(anchor) else { continue };
        let c = color(&peer.user.color);
        let galley = p.layout_no_wrap(peer.user.name.clone(), theme::ui_font(10.5), text_on(c).gamma_multiply(alpha));
        let size = galley.size() + vec2(8.0, 2.0);
        let left = vis.left().min(geo.cells.right() - size.x).max(geo.cells.left());
        let tag = Rect::from_min_size(pos2(left, vis.top() - size.y), size);
        let corners = egui::CornerRadius { nw: 3, ne: 3, sw: if left == vis.left() { 0 } else { 3 }, se: 3 };
        p.rect_filled(tag, corners, c.gamma_multiply(alpha));
        p.galley(tag.min + vec2(4.0, 1.0), galley, Color32::WHITE);
    }
}

/// The Share dialog's defaults: the transports, this person, a room name.
pub fn share_defaults(app: &mut SheetApp, mode: &str) -> serde_json::Value {
    let status = app.session.run("collab.status", json!({})).unwrap_or_default();
    let transports: Vec<serde_json::Value> = status["transports"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| {
                    let id = t["id"].as_str().unwrap_or("").to_string();
                    let commands: Vec<String> =
                        gridcraft_engine::plugin::get(&id).map(|p| p.manifest.commands.iter().map(|c| c.id.clone()).collect()).unwrap_or_default();
                    let has = |c: &str| commands.iter().any(|x| x == c);
                    json!({"id": id, "label": t["label"], "share": has("share"), "join": has("join")})
                })
                .collect()
        })
        .unwrap_or_default();
    let first = transports.first().and_then(|t| t["id"].as_str()).unwrap_or("").to_string();
    let user = &status["user"];
    json!({
        "mode": mode,
        "plugin": first,
        "room": "",
        "name": user["name"],
        "color": user["color"],
        "userId": user["id"],
        "encrypt": false,
        "key": "",
        "transports": transports,
    })
}

/// The add-in that GridCraft's documentation suggests for live sessions.
const EXAMPLE_ADDIN: &str = "https://github.com/sirmmo/craft-collab";

/// The Share dialog: start or join a live session through a transport add-in, or, with none
/// installed, what's needed and a way to Automate › Add-ins.
pub fn share_dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut crate::dialogs::Dialog, confirm: &mut bool, open: &mut bool) {
    use serde_json::Value as Json;
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let transports: Vec<Json> = d.values.get("transports").and_then(Json::as_array).cloned().unwrap_or_default();
    let join = d.values.get("mode").and_then(Json::as_str) == Some("join");
    if transports.is_empty() {
        ui.label(egui::RichText::new(lang.tr("A live session needs an add-in")).font(theme::ui_bold(14.0)).color(t.text));
        ui.add_space(4.0);
        ui.label(
            "Several people can edit this workbook at the same time, each in their own GridCraft. A live-session add-in \
             carries the edits between you (through an MQTT broker, a relay or a server); GridCraft merges them. No such add-in is installed.",
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label("For example, ");
            ui.label(egui::RichText::new("craft-mqtt").strong());
            ui.label(" from ");
            ui.hyperlink_to("sirmmo/craft-collab", EXAMPLE_ADDIN);
            ui.label(" works through any MQTT broker, encrypted end to end.");
        });
        ui.add_space(10.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add_ins = ui
                .add(egui::Button::new(egui::RichText::new(format!("  {}  ", lang.tr("Add-ins…"))).color(Color32::WHITE)).fill(t.accent))
                .on_hover_text("Automate › Add-ins: install an add-in from GitHub or a folder");
            if add_ins.clicked() {
                *open = false;
                app.open_dialog("addIns", json!({}));
            }
            if ui.button(format!(" {} ", lang.tr("Close"))).clicked() {
                *open = false;
            }
        });
        return;
    }
    ui.label(if join {
        "Join a live session someone shared with you. GridCraft opens it in a new workbook."
    } else {
        "Edit this workbook with other people, live. The add-in carries the edits; with a room key they're encrypted end to end."
    });
    ui.add_space(6.0);
    // The transport.
    let current = d.values.get("plugin").and_then(Json::as_str).unwrap_or("").to_string();
    let label_of = |t: &Json| t["label"].as_str().filter(|l| !l.is_empty()).or(t["id"].as_str()).unwrap_or("").to_string();
    let chosen = transports.iter().find(|t| t["id"].as_str() == Some(current.as_str())).or(transports.first()).cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(lang.tr("Add-in:"));
        if transports.len() == 1 {
            ui.label(egui::RichText::new(label_of(&chosen)).strong());
        } else {
            egui::ComboBox::from_id_salt("collab_transport").selected_text(label_of(&chosen)).width(220.0).show_ui(ui, |ui| {
                for tr in &transports {
                    let id = tr["id"].as_str().unwrap_or("").to_string();
                    if ui.selectable_label(id == current, label_of(tr)).clicked() {
                        d.values.insert("plugin".into(), json!(id));
                    }
                }
            });
        }
    });
    let plugin = chosen["id"].as_str().unwrap_or("").to_string();
    let (can_share, can_join) = (chosen["share"].as_bool() == Some(true), chosen["join"].as_bool() == Some(true));
    // The add-in's own Share and Join (they make and read invitations).
    if can_share || can_join {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let primary =
                |ui: &mut Ui, text: &str| ui.add(egui::Button::new(egui::RichText::new(format!("  {text}  ")).color(Color32::WHITE)).fill(t.accent));
            let (first, second) = if join { ("join", "share") } else { ("share", "join") };
            for (i, which) in [first, second].into_iter().enumerate() {
                let ok = if which == "share" { can_share } else { can_join };
                if !ok {
                    continue;
                }
                let text = if which == "share" { lang.tr("Start a Live Session") } else { lang.tr("Join from an Invitation…") };
                let resp = if i == 0 { primary(ui, text) } else { ui.button(text) };
                if resp.clicked() {
                    *open = false;
                    app.run_or_alert("plugins.run", json!({"plugin": plugin, "command": which}));
                }
            }
        });
    }
    // A room by name, through any transport.
    ui.add_space(6.0);
    egui::CollapsingHeader::new(lang.tr("Room name and key")).default_open(!(can_share || can_join)).show(ui, |ui| {
        fn text(ui: &mut Ui, values: &mut serde_json::Map<String, Json>, key: &str, label: &str, password: bool) {
            ui.label(label);
            let mut s = values.get(key).and_then(Json::as_str).unwrap_or("").to_string();
            if ui.add(egui::TextEdit::singleline(&mut s).password(password).desired_width(320.0)).changed() {
                values.insert(key.into(), json!(s));
            }
        }
        let mode = d.values.get("mode").and_then(Json::as_str).unwrap_or("share").to_string();
        ui.horizontal(|ui| {
            for (value, label) in [("share", "Share this workbook"), ("join", "Join (a new workbook)")] {
                if ui.radio(mode == value, lang.tr(label)).clicked() {
                    d.values.insert("mode".into(), json!(value));
                }
            }
        });
        text(ui, &mut d.values, "room", "Room:", false);
        text(ui, &mut d.values, "name", "Your name:", false);
        text(ui, &mut d.values, "color", "Your colour (#RRGGBB):", false);
        let mut encrypt = d.values.get("encrypt").and_then(Json::as_bool).unwrap_or(false);
        if ui.checkbox(&mut encrypt, "Encrypt end to end (a new room key)").changed() {
            d.values.insert("encrypt".into(), json!(encrypt));
        }
        text(ui, &mut d.values, "key", "Room key (base64, from the invitation):", true);
        ui.add_space(4.0);
        let go = if mode == "join" { lang.tr("Join") } else { lang.tr("Start") };
        if ui.button(format!("  {go}  ")).clicked() {
            d.values.insert("plugin".into(), json!(plugin));
            *confirm = true;
        }
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(format!(" {} ", lang.tr("Cancel"))).clicked() {
            *open = false;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_four_circles_then_plus_n() {
        assert_eq!(circles(1, 5), (1, 0));
        assert_eq!(circles(4, 5), (4, 0));
        assert_eq!(circles(5, 5), (4, 1));
        assert_eq!(circles(9, 5), (4, 5));
        // Less room: fewer circles, "+N" takes a slot.
        assert_eq!(circles(6, 3), (2, 4));
        assert_eq!(circles(2, 2), (2, 0));
        assert_eq!(circles(3, 1), (0, 3));
        assert_eq!(circles(1, 1), (1, 0));
        assert_eq!(circles(3, 0), (0, 0));
        assert_eq!(people_width(0, 0), 0.0);
        assert_eq!(people_width(4, 1), CIRCLE + 4.0 * CIRCLE_STEP);
    }

    #[test]
    fn initials_and_readable_text() {
        assert_eq!(initials("Ann Lee"), "AL");
        assert_eq!(initials("bob"), "B");
        assert_eq!(initials("Maria de la Cruz"), "MC");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("(x)"), "(");
        assert_eq!(initials("élodie roux"), "ÉR");
        assert_eq!(text_on(Color32::from_rgb(0xF2, 0xC9, 0x4C)), Color32::BLACK, "dark text on yellow");
        assert_eq!(text_on(Color32::from_rgb(0x3B, 0x7D, 0xD8)), Color32::WHITE, "white on blue");
        assert_eq!(text_on(Color32::WHITE), Color32::BLACK);
        assert_eq!(text_on(Color32::BLACK), Color32::WHITE);
    }

    #[test]
    fn name_tags_fade_after_three_seconds_and_come_back_on_hover() {
        assert_eq!(tag_alpha(0.0, false), 1.0);
        assert_eq!(tag_alpha(TAG_MS, false), 1.0);
        assert!((tag_alpha(TAG_MS + TAG_FADE_MS / 2.0, false) - 0.5).abs() < 1e-4);
        assert_eq!(tag_alpha(TAG_MS + TAG_FADE_MS, false), 0.0);
        assert_eq!(tag_alpha(60_000.0, true), 1.0);
    }
}
