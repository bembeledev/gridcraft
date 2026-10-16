//! The document chat in the shared document (craft-collab `docs/chat.md`, protocol 2.1): the
//! root array `chat` of message maps and the root map `chatMeta` (`clearedAt`), the same in
//! every craft app. Readers apply the chat's rules ([`Chat::sanitized`]); writers only append,
//! change their own messages' keys, or clear everything in one transaction.

use std::collections::HashMap;

use gridcraft_model::{Chat, ChatAuthor, ChatMessage};
use serde_json::Value as Json;
use yrs::any::Number;
use yrs::{Any, Array, Map, MapPrelim, MapRef, Out, ReadTxn, TransactionMut, WriteTxn};

/// The root array of messages.
pub const ROOT: &str = "chat";
/// The root map with `clearedAt`.
pub const META: &str = "chatMeta";

fn string(v: Option<Out>) -> Option<String> {
    match v? {
        Out::Any(Any::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// An integer time; a fraction or a value outside 0..=2^53 isn't one.
fn ms(v: Option<Out>) -> Option<i64> {
    let n = match v? {
        Out::Any(Any::Number(Number::Int(i))) => i,
        Out::Any(Any::Number(Number::Float(f))) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 => f as i64,
        _ => return None,
    };
    (0..=(1i64 << 53)).contains(&n).then_some(n)
}

fn num(n: i64) -> Any {
    Any::Number(Number::Int(n))
}

/// One entry as a message, before the chat's rules (`None`: not a message at all).
fn entry<T: ReadTxn>(txn: &T, m: &MapRef) -> Option<ChatMessage> {
    let author = string(m.get(txn, "author")).and_then(|s| serde_json::from_str::<Json>(&s).ok()).and_then(|j| ChatAuthor::from_json(&j))?;
    Some(ChatMessage {
        id: string(m.get(txn, "id"))?,
        author,
        at: ms(m.get(txn, "at"))?,
        text: string(m.get(txn, "text")),
        reply_to: string(m.get(txn, "replyTo")),
        edited: ms(m.get(txn, "edited")),
        deleted: matches!(m.get(txn, "deleted"), Some(Out::Any(Any::Bool(true)))),
    })
}

/// The chat readers see.
pub fn read<T: ReadTxn>(txn: &T) -> Chat {
    let cleared = txn.get_map(META).and_then(|m| ms(m.get(txn, "clearedAt")));
    let Some(arr) = txn.get_array(ROOT) else { return Chat::sanitized(cleared, []) };
    let entries: Vec<ChatMessage> = arr.iter(txn).filter_map(|v| if let Out::YMap(m) = v { entry(txn, &m) } else { None }).collect();
    Chat::sanitized(cleared, entries)
}

/// The maps of the messages readers keep, by id.
fn maps<T: ReadTxn>(txn: &T) -> HashMap<String, MapRef> {
    let Some(arr) = txn.get_array(ROOT) else { return HashMap::new() };
    let mut out = HashMap::new();
    for v in arr.iter(txn) {
        let Out::YMap(m) = v else { continue };
        if let Some(msg) = entry(txn, &m).filter(ChatMessage::valid) {
            out.entry(msg.id).or_insert(m);
        }
    }
    out
}

/// Appends a message, its keys set one by one in a fixed order.
fn push(txn: &mut TransactionMut, m: &ChatMessage) {
    let arr = txn.get_or_insert_array(ROOT);
    let map = arr.push_back(txn, MapPrelim::default());
    map.insert(txn, "id", m.id.as_str());
    map.insert(txn, "author", m.author.to_json().to_string());
    map.insert(txn, "at", num(m.at));
    if let Some(t) = &m.text {
        map.insert(txn, "text", t.as_str());
    }
    if let Some(r) = &m.reply_to {
        map.insert(txn, "replyTo", r.as_str());
    }
    if let Some(e) = m.edited {
        map.insert(txn, "edited", num(e));
    }
    if m.deleted {
        map.insert(txn, "deleted", true);
    }
}

/// Writes a workbook's chat into an empty document (sharing). An empty chat writes nothing.
pub fn seed(txn: &mut TransactionMut, chat: &Chat) {
    if let Some(c) = chat.cleared_at {
        txn.get_or_insert_map(META).insert(txn, "clearedAt", num(c));
    }
    for m in &chat.messages {
        push(txn, m);
    }
}

/// Writes the change from `base` to `cur` (the workbook's chat before and after commands):
/// a clear removes every entry and sets `clearedAt`; new messages are appended in order; a
/// message whose text, `edited` or `deleted` changed gets those keys. Nothing else is written,
/// so concurrent messages from others stay.
pub fn write_diff(txn: &mut TransactionMut, base: &Chat, cur: &Chat) {
    if base == cur {
        return;
    }
    let cleared = cur.cleared_at != base.cleared_at && cur.cleared_at.is_some();
    if cleared {
        if let Some(arr) = txn.get_array(ROOT) {
            let len = arr.len(txn);
            if len > 0 {
                arr.remove_range(txn, 0, len);
            }
        }
        if let Some(c) = cur.cleared_at {
            txn.get_or_insert_map(META).insert(txn, "clearedAt", num(c));
        }
        for m in &cur.messages {
            push(txn, m);
        }
        return;
    }
    let before: HashMap<&str, &ChatMessage> = base.messages.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut changed: Vec<&ChatMessage> = Vec::new();
    for m in &cur.messages {
        match before.get(m.id.as_str()) {
            None => push(txn, m),
            Some(b) if *b != m => changed.push(m),
            Some(_) => {}
        }
    }
    if changed.is_empty() {
        return;
    }
    let maps = maps(txn);
    for m in changed {
        let Some(map) = maps.get(&m.id) else { continue };
        match &m.text {
            Some(t) => {
                map.insert(txn, "text", t.as_str());
            }
            None => {
                map.remove(txn, "text");
            }
        }
        match m.edited {
            Some(e) => {
                map.insert(txn, "edited", num(e));
            }
            None => {
                map.remove(txn, "edited");
            }
        }
        if m.deleted {
            map.insert(txn, "deleted", true);
        }
    }
}
