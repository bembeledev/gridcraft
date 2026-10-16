//! The document chat in the interface (craft-collab `docs/chat.md`): the title bar's chat
//! button (beside the people's circles, with the unread count, flashing when someone mentions
//! you) and the Chat pane (where the task panes show): messages with their author's circle,
//! name and time, replies quoted, "edited" and "Message deleted", links and `@Name` mentions;
//! a box that sends with Enter (Shift+Enter: a new line); a menu with Export, Clear chat… and
//! the note that Office drops the chat.
//!
//! The engine holds the chat (`Workbook::chat`) and changes it with the `chat.*` commands;
//! this only shows it and runs them.

use std::collections::HashMap;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, Key, Layout, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::model::{Chat, ChatMessage};
use serde_json::json;

use crate::SheetApp;
use crate::icons::Icon;
use crate::theme::{self, Tokens};

/// How long the chat button flashes after a message that mentions you (milliseconds).
const MENTION_FLASH_MS: f64 = 2_400.0;
/// The chat button's size.
pub const BUTTON_W: f32 = 30.0;
/// The note the menu shows: the chat is a part Office doesn't know.
pub const OFFICE_NOTE: &str = "Kept in the file; Office drops it when it saves";
/// What Clear chat… asks.
pub const CLEAR_QUESTION: &str = "Remove the whole chat from the document for everyone?";

/// The interface's chat state (per window).
#[derive(Debug, Default)]
pub struct ChatState {
    /// The message being written.
    pub draft: String,
    /// Replying to this message.
    pub reply_to: Option<String>,
    /// Editing this message of yours (the box holds its text).
    pub editing: Option<String>,
    /// Messages seen per workbook (its uid): those after them are unread.
    pub seen: HashMap<u64, usize>,
    /// Messages known per workbook at the last frame (to notice new ones that mention you).
    known: HashMap<u64, usize>,
    /// The chat button flashes since then (a mention of you).
    pub flash_at: Option<f64>,
    /// Clear chat… is asking.
    pub confirm_clear: bool,
    /// What `chat.typing` was last told.
    typing_sent: bool,
    /// Minutes from UTC for times older than today (the host sets it; 0 shows UTC).
    pub utc_offset_min: i32,
}

fn color(hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x80_80_80);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Who "you" are in the active workbook's chat: the session's user, or File › Options' one.
pub fn me(app: &SheetApp) -> gridcraft_engine::collab::User {
    match app.session.active_room() {
        Some(r) => r.user.clone(),
        None => gridcraft_engine::collab::local_user(&app.session),
    }
}

/// The names `@` mentions are highlighted for: the people in the session (you outside one).
fn names(app: &SheetApp) -> Vec<String> {
    let mut v: Vec<String> = app.session.active_room().map(|r| r.peers().into_iter().map(|p| p.user.name).collect()).unwrap_or_default();
    v.push(me(app).name);
    v
}

fn chat(app: &SheetApp) -> Option<(u64, std::sync::Arc<Chat>)> {
    app.session.active().map(|d| (d.uid, d.wb.chat.clone()))
}

/// Unread messages: those from others after the ones seen.
pub fn unread(app: &SheetApp) -> usize {
    let Some((uid, c)) = chat(app) else { return 0 };
    let seen = app.chat.seen.get(&uid).copied().unwrap_or(c.messages.len()).min(c.messages.len());
    let me = me(app).id;
    c.messages.get(seen..).unwrap_or_default().iter().filter(|m| m.author.id != me && !m.deleted).count()
}

/// Whether the chat pane is open.
pub fn open(app: &SheetApp) -> bool {
    app.grid.pane.as_deref() == Some("chat")
}

/// Keeps the unread count and the mention flash up to date (every frame).
pub fn track(app: &mut SheetApp, now: f64) {
    let Some((uid, c)) = chat(app) else { return };
    let n = c.messages.len();
    let me = me(app);
    // A workbook seen for the first time: its chat (from the file) counts as read.
    let seen = *app.chat.seen.entry(uid).or_insert(n);
    if open(app) || seen > n {
        app.chat.seen.insert(uid, n);
    }
    let known = app.chat.known.insert(uid, n).unwrap_or(n);
    if n > known && !open(app) && c.messages.get(known.min(n)..).unwrap_or_default().iter().any(|m| m.author.id != me.id && m.mentions(&me.name)) {
        app.chat.flash_at = Some(now);
    }
    if app.chat.flash_at.is_some_and(|t| now - t > MENTION_FLASH_MS) {
        app.chat.flash_at = None;
    }
}

/// Whether the title bar shows the chat button: in a live session, or when the workbook has a
/// chat (notes left in the file), or while the pane is open.
pub fn button_visible(app: &SheetApp) -> bool {
    app.session.active_room().is_some() || app.session.active().is_some_and(|d| !d.wb.chat.messages.is_empty()) || open(app)
}

/// Shows or hides the chat pane (`view.chat`).
pub fn toggle(app: &mut SheetApp, on: Option<bool>) -> bool {
    let on = on.unwrap_or(!open(app));
    if on {
        if app.session.comments_pane {
            app.run_or_alert("review.showComments", json!({"on": false}));
        }
        app.grid.pane = Some("chat".into());
    } else if open(app) {
        app.grid.pane = None;
    }
    on
}

/// The title bar's chat button: the unread count as a badge, a flash for a mention of you.
pub fn title_button(app: &mut SheetApp, ui: &mut Ui, h: f32) {
    let t = Tokens::get(ui.ctx());
    let now = crate::now_ms();
    track(app, now);
    let count = unread(app);
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, h), Sense::click());
    let flash = app.chat.flash_at.map(|at| ((now - at) / 300.0).sin().abs() as f32);
    let fill = if let Some(f) = flash {
        ui.ctx().request_repaint();
        t.accent.gamma_multiply(0.25 + 0.5 * f)
    } else if open(app) || resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, fill);
    crate::icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), Icon::Chat, t.text_dim);
    if count > 0 {
        let text = if count > 99 { "99+".to_string() } else { count.to_string() };
        let galley = ui.painter().layout_no_wrap(text, theme::ui_bold(9.5), Color32::WHITE);
        let w = (galley.size().x + 7.0).max(14.0);
        let badge = Rect::from_center_size(pos2(rect.right() - w / 2.0 + 2.0, rect.top() + 7.0), vec2(w, 14.0));
        ui.painter().rect_filled(badge, 7.0, t.danger);
        ui.painter().rect_stroke(badge, 7.0, Stroke::new(1.5, t.window), StrokeKind::Outside);
        ui.painter().galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
    let label = if count > 0 { format!("Chat, {count} unread") } else { "Chat".to_string() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    if resp.on_hover_text(if count > 0 { label.clone() } else { "Chat with the people working on this workbook".into() }).clicked() {
        toggle(app, None);
    }
}

// ---------------------------------------------------------------- the pane

/// The pane's menu (beside its title): Export, Clear chat… and the Office note.
pub fn menu(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let more = crate::widgets::icon_button(ui, Icon::More, t.text_dim, "Chat options", vec2(22.0, 22.0));
    more.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Chat options"));
    egui::Popup::menu(&more).width(250.0).show(|ui| {
        if ui.button("Export as Text").clicked() {
            export(app, ui, "text");
        }
        if ui.button("Export as JSON").clicked() {
            export(app, ui, "json");
        }
        ui.separator();
        let any = app.session.active().is_some_and(|d| !d.wb.chat.messages.is_empty());
        if ui.add_enabled(any, egui::Button::new("Clear chat…")).clicked() {
            app.chat.confirm_clear = true;
        }
        ui.separator();
        ui.label(egui::RichText::new(OFFICE_NOTE).font(theme::ui_font(11.5)).color(t.text_dim));
    });
}

fn export(app: &mut SheetApp, ui: &Ui, format: &str) {
    match app.run("chat.export", json!({"format": format, "utcOffsetMinutes": app.chat.utc_offset_min})) {
        Ok(r) => {
            ui.ctx().copy_text(r.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string());
            app.status_message = Some(("The chat was copied to the clipboard".into(), crate::now_ms()));
        }
        Err(e) => app.message = Some(("GridCraft".into(), e)),
    }
}

/// "just now", "5 min ago", "2 h ago" for today; else the date and time.
pub fn when(at: i64, now: f64, offset_min: i32) -> String {
    let ago = (now as i64).saturating_sub(at);
    let day = |ms: i64| ms.saturating_add(i64::from(offset_min) * 60_000).div_euclid(86_400_000);
    if (0..60_000).contains(&ago) || (-60_000..0).contains(&ago) {
        return "just now".into();
    }
    if ago > 0 && day(at) == day(now as i64) {
        return if ago < 3_600_000 { format!("{} min ago", ago / 60_000) } else { format!("{} h ago", ago / 3_600_000) };
    }
    let full = gridcraft_engine::model::chat::format_time(at, offset_min);
    // "2026-10-14 09:30 UTC" → "Oct 14, 09:30" (with the year when it isn't this one).
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let year_now = gridcraft_engine::model::chat::format_time(now as i64, offset_min).get(..4).map(str::to_string);
    let (y, m, d, hm) = (full.get(..4), full.get(5..7).and_then(|m| m.parse::<usize>().ok()), full.get(8..10), full.get(11..16));
    match (y, m, d, hm) {
        (Some(y), Some(m), Some(d), Some(hm)) => {
            let mon = month.get(m.saturating_sub(1)).copied().unwrap_or("");
            let d = d.trim_start_matches('0');
            if year_now.as_deref() == Some(y) { format!("{mon} {d}, {hm}") } else { format!("{mon} {d} {y}, {hm}") }
        }
        _ => full,
    }
}

/// The links (`http://`, `https://`) in a text: byte ranges, without trailing punctuation.
pub fn links(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rest) = text.get(i..) {
        let Some(off) = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min() else { break };
        let start = i + off;
        let len = text.get(start..).map(|r| r.find(char::is_whitespace).unwrap_or(r.len())).unwrap_or(0);
        let mut end = start + len;
        while end > start && text.get(..end).and_then(|s| s.chars().next_back()).is_some_and(|c| ".,;:!?)]}'\"".contains(c)) {
            end -= text.get(..end).and_then(|s| s.chars().next_back()).map(char::len_utf8).unwrap_or(1);
        }
        if end > start + 8 {
            out.push((start, end));
        }
        i = (start + len).max(i + 1);
    }
    out
}

/// A message's text as a layout: links underlined in the accent colour, mentions highlighted.
fn text_job(text: &str, names: &[String], t: &Tokens, width: f32) -> (LayoutJob, Vec<(usize, usize, String)>) {
    let font = theme::ui_font(13.0);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mentions = gridcraft_engine::model::chat::mention_spans(text, &refs);
    let links = links(text);
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: width, ..Default::default() }, ..Default::default() };
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for (a, b) in mentions.iter().chain(links.iter()) {
        cuts.push(*a);
        cuts.push(*b);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut urls = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w.first().copied().unwrap_or(0), w.get(1).copied().unwrap_or(0));
        let Some(part) = text.get(a..b) else { continue };
        let mut f = TextFormat { font_id: font.clone(), color: t.text, ..Default::default() };
        if let Some((la, lb)) = links.iter().find(|(la, lb)| *la <= a && b <= *lb) {
            f.color = t.accent;
            f.underline = Stroke::new(1.0, t.accent);
            if let Some(url) = text.get(*la..*lb) {
                urls.push((job.text.len(), job.text.len() + part.len(), url.to_string()));
            }
        } else if mentions.iter().any(|(ma, mb)| *ma <= a && b <= *mb) {
            f.color = t.accent_dark;
            f.background = t.accent_soft;
            f.font_id = theme::ui_bold(13.0);
        }
        job.append(part, 0.0, f);
    }
    (job, urls)
}

fn circle(ui: &mut Ui, name: &str, c: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 12.0, c);
    let initials = crate::collab_ui::initials(name);
    let size = if initials.chars().count() > 1 { 9.5 } else { 11.0 };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initials, theme::ui_bold(size), crate::collab_ui::text_on(c));
}

/// What a message's context menu asked for.
enum Action {
    Reply(String),
    Edit(String, String),
    Delete(String),
}

fn message(ui: &mut Ui, c: &Chat, m: &ChatMessage, me: &str, names: &[String], now: f64, offset: i32, t: &Tokens) -> Option<Action> {
    let mut action = None;
    let inner = ui.horizontal_top(|ui| {
        circle(ui, &m.author.name, color(&m.author.color));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(&m.author.name).font(theme::ui_bold(12.5)).color(t.text));
                ui.label(egui::RichText::new(when(m.at, now, offset)).font(theme::ui_font(11.0)).color(t.text_dim));
                if m.edited.is_some() && !m.deleted {
                    ui.label(egui::RichText::new("edited").font(theme::ui_font(11.0)).italics().color(t.text_dim));
                }
            });
            if let Some(r) = &m.reply_to {
                let (who, what) = match c.get(r) {
                    Some(q) if q.deleted => (q.author.name.clone(), "Message deleted".to_string()),
                    Some(q) => (q.author.name.clone(), q.text.clone().unwrap_or_default()),
                    None => (String::new(), "A message no longer in the chat".to_string()),
                };
                let first: String = what.lines().next().unwrap_or("").chars().take(80).collect();
                let bar = c.get(r).map(|q| color(&q.author.color)).unwrap_or(t.separator);
                egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 4, top: 2, bottom: 2 }).show(ui, |ui| {
                    let text = if who.is_empty() { first } else { format!("{who}: {first}") };
                    let resp = ui.add(egui::Label::new(egui::RichText::new(text).font(theme::ui_font(11.5)).color(t.text_dim)).truncate());
                    let r = resp.rect;
                    ui.painter().line_segment([pos2(r.left() - 6.0, r.top()), pos2(r.left() - 6.0, r.bottom())], Stroke::new(2.5, bar));
                });
            }
            match (&m.text, m.deleted) {
                (Some(text), false) => {
                    let width = ui.available_width();
                    let (job, urls) = text_job(text, names, t, width);
                    let galley = ui.painter().layout_job(job.clone());
                    let resp = ui.add(egui::Label::new(job.clone()).selectable(true));
                    if !urls.is_empty() {
                        let pos = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
                        let at = pos.map(|p| galley.cursor_from_pos(p - resp.rect.min).index);
                        let url = at.and_then(|i| {
                            let byte = job.text.char_indices().nth(i.into()).map(|(b, _)| b).unwrap_or(job.text.len());
                            urls.iter().find(|(a, b, _)| *a <= byte && byte < *b).map(|(_, _, u)| u.clone())
                        });
                        if let Some(u) = &url {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(u));
                            }
                        }
                    }
                }
                _ => {
                    ui.label(egui::RichText::new("Message deleted").font(theme::ui_font(12.5)).italics().color(t.text_dim));
                }
            }
        });
    });
    let id = m.id.clone();
    let mine = m.author.id == me && !m.deleted;
    inner.response.interact(Sense::click()).context_menu(|ui| {
        if !m.deleted && ui.button("Reply").clicked() {
            action = Some(Action::Reply(id.clone()));
        }
        if !m.deleted && ui.button("Copy Text").clicked() {
            ui.ctx().copy_text(m.text.clone().unwrap_or_default());
        }
        if mine && ui.button("Edit").clicked() {
            action = Some(Action::Edit(id.clone(), m.text.clone().unwrap_or_default()));
        }
        if mine && ui.button("Delete").clicked() {
            action = Some(Action::Delete(id.clone()));
        }
    });
    action
}

/// Sends (or saves the edit of) what the box holds.
pub fn submit(app: &mut SheetApp) {
    let text = app.chat.draft.trim_end_matches(['\n', '\r']).to_string();
    if text.trim().is_empty() {
        return;
    }
    let r = match app.chat.editing.clone() {
        Some(id) => app.run("chat.edit", json!({"id": id, "text": text})),
        None => {
            let mut p = json!({"text": text});
            if let Some(r) = &app.chat.reply_to {
                p["replyTo"] = json!(r);
            }
            app.run("chat.send", p)
        }
    };
    match r {
        Ok(_) => {
            app.chat.draft.clear();
            app.chat.reply_to = None;
            app.chat.editing = None;
            app.chat.typing_sent = false;
        }
        Err(e) => app.status_message = Some((e, crate::now_ms())),
    }
}

/// The pane's content: the messages, who's typing, the box.
pub fn panel(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let now = crate::now_ms();
    track(app, now);
    let Some((_, c)) = chat(app) else { return };
    let me = me(app);
    let names = names(app);
    let typing = app.session.active_room().map(|r| r.typing_names()).unwrap_or_default();
    let offset = app.chat.utc_offset_min;
    let input_id = egui::Id::new("chat_input");
    let mut actions = Vec::new();
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        // The box (at the bottom).
        let focused = ui.memory(|m| m.has_focus(input_id));
        // Enter alone sends (consume_key would also take Shift+Enter, the new line).
        let enter = focused
            && ui.input_mut(|i| {
                let at = i.events.iter().position(
                    |e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift && !modifiers.alt && !modifiers.command && !modifiers.ctrl),
                );
                at.map(|p| i.events.remove(p)).is_some()
            });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let send_w = 54.0;
            let edit = egui::TextEdit::multiline(&mut app.chat.draft)
                .id(input_id)
                .desired_rows(2)
                .desired_width(ui.available_width() - send_w - 8.0)
                .hint_text("Write a message…")
                .font(theme::ui_font(13.0));
            let resp = ui.add(edit);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Chat message"));
            resp.on_hover_text("Enter sends; Shift+Enter starts a new line");
            let label = if app.chat.editing.is_some() { "Save" } else { "Send" };
            let can = !app.chat.draft.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(label).min_size(vec2(send_w, 26.0))).clicked() || enter {
                submit(app);
                ui.memory_mut(|m| m.request_focus(input_id));
            }
        });
        // Replying or editing.
        let banner = match (&app.chat.editing, &app.chat.reply_to) {
            (Some(_), _) => Some("Editing your message".to_string()),
            (None, Some(r)) => Some(format!("Replying to {}", c.get(r).map(|m| m.author.name.as_str()).unwrap_or("a message"))),
            _ => None,
        };
        if let Some(b) = banner {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(b).font(theme::ui_font(11.5)).color(t.text_dim));
                if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, "Cancel", vec2(16.0, 16.0)).clicked() {
                    if app.chat.editing.take().is_some() {
                        app.chat.draft.clear();
                    }
                    app.chat.reply_to = None;
                }
            });
        }
        let line = match typing.as_slice() {
            [] => String::new(),
            [one] => format!("{one} is typing…"),
            [a, b] => format!("{a} and {b} are typing…"),
            _ => "Several people are typing…".into(),
        };
        ui.label(egui::RichText::new(line).font(theme::ui_font(11.5)).italics().color(t.text_dim));
        ui.separator();
        // The messages (the rest of the pane, newest at the bottom).
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                if c.messages.is_empty() {
                    let hint = if app.session.active_room().is_some() {
                        "No messages yet. Say hello to the people working on this workbook."
                    } else {
                        "No messages yet. Messages are kept in the workbook, for whoever opens it next."
                    };
                    ui.label(egui::RichText::new(hint).font(theme::ui_font(12.5)).italics().color(t.text_dim));
                }
                for m in &c.messages {
                    if let Some(a) = message(ui, &c, m, &me.id, &names, now, offset, &t) {
                        actions.push(a);
                    }
                    ui.add_space(6.0);
                }
            });
        });
    });
    for a in actions {
        match a {
            Action::Reply(id) => {
                app.chat.editing = None;
                app.chat.reply_to = Some(id);
                ui.memory_mut(|m| m.request_focus(input_id));
            }
            Action::Edit(id, text) => {
                app.chat.reply_to = None;
                app.chat.editing = Some(id);
                app.chat.draft = text;
                ui.memory_mut(|m| m.request_focus(input_id));
            }
            Action::Delete(id) => app.run_or_alert("chat.delete", json!({"id": id})),
        }
    }
    // Typing: told to the session when it starts and stops.
    let typing_now = app.session.active_room().is_some() && !app.chat.draft.trim().is_empty() && ui.memory(|m| m.has_focus(input_id));
    if typing_now != app.chat.typing_sent && app.session.active_room().is_some() {
        app.chat.typing_sent = typing_now;
        let _ = app.run("chat.typing", json!({"typing": typing_now}));
    }
    confirm_clear(app, ui);
}

/// Clear chat…'s question.
fn confirm_clear(app: &mut SheetApp, ui: &mut Ui) {
    if !app.chat.confirm_clear {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("chat_clear")).show(ui.ctx(), |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new("Clear the chat").font(theme::ui_bold(15.0)));
        ui.add_space(6.0);
        ui.label(CLEAR_QUESTION);
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                app.chat.confirm_clear = false;
            }
            if ui.button("Clear chat").clicked() {
                app.chat.confirm_clear = false;
                app.run_or_alert("chat.clear", json!({}));
            }
        });
    });
    if modal.should_close() {
        app.chat.confirm_clear = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_relative_today() {
        let now = 1_760_605_200_000.0; // 2025-10-16 09:00 UTC
        assert_eq!(when(1_760_605_190_000, now, 0), "just now");
        assert_eq!(when(1_760_604_900_000, now, 0), "5 min ago");
        assert_eq!(when(1_760_594_400_000, now, 0), "3 h ago");
        assert_eq!(when(1_760_518_800_000, now, 0), "Oct 15, 09:00");
        assert_eq!(when(1_729_069_200_000, now, 0), "Oct 16 2024, 09:00");
    }

    #[test]
    fn links_lose_trailing_punctuation() {
        let t = "See https://example.org/a?b=c, and (http://x.io/y).";
        let l: Vec<&str> = links(t).iter().map(|(a, b)| &t[*a..*b]).collect();
        assert_eq!(l, ["https://example.org/a?b=c", "http://x.io/y"]);
        assert!(links("no links: https:// alone").is_empty());
    }
}
