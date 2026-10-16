//! Document chat (craft-collab `docs/chat.md`, protocol 2.1): the people working on a workbook
//! talk beside it. The chat belongs to the workbook: it's shared live in a session, saved in the
//! file (XLSX part `/craft/chat.json`, the native JSON format's `chat`) and stays until someone
//! removes it. Chat changes are never undo steps.
//!
//! Every reader applies the same rules ([`Chat::sanitized`]): malformed messages, duplicate ids
//! after the first and messages beyond [`MAX_MESSAGES`] are left out.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json, json};

/// At most this many messages: readers ignore the rest, `send` refuses more.
pub const MAX_MESSAGES: usize = 10_000;
/// A message's text: 1 to this many UTF-16 units.
pub const MAX_TEXT: usize = 4_000;
/// What sending into a full chat says.
pub const CHAT_FULL: &str = "The chat is full: clear it to go on";
/// The file part's `version`.
pub const PART_VERSION: i64 = 1;
/// Times are integers from 0 to 2^53 (milliseconds since 1970 UTC).
const MAX_MS: i64 = 1 << 53;

/// Who wrote a message: the `user` of the live session, or this app's own user outside one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatAuthor {
    pub id: String,
    pub name: String,
    /// `#RRGGBB`.
    pub color: String,
}

impl ChatAuthor {
    /// Non-empty `id` and `name`.
    pub fn valid(&self) -> bool {
        !self.id.is_empty() && !self.name.is_empty()
    }

    pub fn to_json(&self) -> Json {
        json!({"id": self.id, "name": self.name, "color": self.color})
    }

    /// `{id, name, color}`, all strings, `id` and `name` non-empty.
    pub fn from_json(j: &Json) -> Option<ChatAuthor> {
        let s = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
        let a = ChatAuthor { id: s("id")?, name: s("name")?, color: s("color")? };
        a.valid().then_some(a)
    }
}

/// One message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatMessage {
    /// 8 characters of `0-9a-z`.
    pub id: String,
    pub author: ChatAuthor,
    /// When it was sent: the sender's clock, milliseconds since 1970 UTC.
    pub at: i64,
    /// `None` once deleted.
    pub text: Option<String>,
    /// The message it answers.
    pub reply_to: Option<String>,
    /// When the author last edited it.
    pub edited: Option<i64>,
    /// The author deleted it: it shows as "Message deleted".
    pub deleted: bool,
}

impl ChatMessage {
    /// Whether a reader keeps it (before the duplicate and count rules).
    pub fn valid(&self) -> bool {
        valid_id(&self.id)
            && self.author.valid()
            && valid_ms(self.at)
            && (self.deleted || self.text.as_deref().is_some_and(|t| valid_text(t).is_ok()))
    }

    /// The message as readers keep it: no text once deleted, optional keys that aren't valid dropped.
    fn normalized(mut self) -> ChatMessage {
        if self.deleted {
            self.text = None;
        }
        self.reply_to = self.reply_to.filter(|r| valid_id(r));
        self.edited = self.edited.filter(|e| valid_ms(*e));
        self
    }

    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        o.insert("id".into(), json!(self.id));
        o.insert("author".into(), self.author.to_json());
        o.insert("at".into(), json!(self.at));
        if let Some(t) = &self.text {
            o.insert("text".into(), json!(t));
        }
        if let Some(r) = &self.reply_to {
            o.insert("replyTo".into(), json!(r));
        }
        if let Some(e) = self.edited {
            o.insert("edited".into(), json!(e));
        }
        if self.deleted {
            o.insert("deleted".into(), json!(true));
        }
        Json::Object(o)
    }

    /// A message in the JSON form (`author` an object); `None` when it isn't one.
    pub fn from_json(j: &Json) -> Option<ChatMessage> {
        let s = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
        Some(ChatMessage {
            id: s("id")?,
            author: ChatAuthor::from_json(j.get("author")?)?,
            at: json_ms(j.get("at")?)?,
            text: s("text"),
            reply_to: s("replyTo"),
            edited: j.get("edited").and_then(json_ms),
            deleted: j.get("deleted").and_then(Json::as_bool) == Some(true),
        })
    }

    /// Whether the text names `name` as `@name` (case-insensitive, not inside a longer word).
    pub fn mentions(&self, name: &str) -> bool {
        let Some(text) = &self.text else { return false };
        !name.trim().is_empty() && !mention_spans(text, &[name]).is_empty()
    }
}

/// The byte ranges of `@Name` mentions of the given names in `text` (longest name first where
/// two start alike): `@` at the start or after a character that isn't a letter or digit, the
/// name in any case, then no letter or digit.
pub fn mention_spans(text: &str, names: &[&str]) -> Vec<(usize, usize)> {
    let lower = text.to_lowercase();
    // Lower-casing that changes the length (a few scripts) would move the offsets: compare as is.
    let hay = if lower.len() == text.len() { lower.as_str() } else { text };
    let mut sorted: Vec<String> = names.iter().map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect();
    sorted.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let mut out = Vec::new();
    for (at, _) in text.match_indices('@') {
        if text.get(..at).and_then(|b| b.chars().next_back()).is_some_and(char::is_alphanumeric) {
            continue;
        }
        let Some(rest) = hay.get(at + 1..) else { continue };
        for n in &sorted {
            if rest.starts_with(n.as_str()) {
                let end = at + 1 + n.len();
                if hay.get(end..).and_then(|r| r.chars().next()).is_none_or(|c| !c.is_alphanumeric()) {
                    out.push((at, end));
                    break;
                }
            }
        }
    }
    out
}

/// A workbook's chat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    /// When someone last cleared it.
    pub cleared_at: Option<i64>,
    /// Oldest first.
    pub messages: Vec<ChatMessage>,
}

/// 8 characters of `0-9a-z`.
pub fn valid_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// 1 to [`MAX_TEXT`] UTF-16 units.
pub fn valid_text(text: &str) -> Result<(), String> {
    match text.encode_utf16().count() {
        0 => Err("A message can't be empty".into()),
        n if n > MAX_TEXT => Err(format!("A message holds at most {MAX_TEXT} characters ({n} given)")),
        _ => Ok(()),
    }
}

fn valid_ms(n: i64) -> bool {
    (0..=MAX_MS).contains(&n)
}

fn json_ms(v: &Json) -> Option<i64> {
    let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64))?;
    valid_ms(n).then_some(n)
}

impl Chat {
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.cleared_at.is_none()
    }

    /// The chat readers keep from `messages` as found (the shared document's entries, a file's
    /// list): valid ones, the first of each id, at most [`MAX_MESSAGES`].
    pub fn sanitized(cleared_at: Option<i64>, messages: impl IntoIterator<Item = ChatMessage>) -> Chat {
        let mut seen = HashSet::new();
        let messages = messages
            .into_iter()
            .filter(ChatMessage::valid)
            .filter(|m| seen.insert(m.id.clone()))
            .take(MAX_MESSAGES)
            .map(ChatMessage::normalized)
            .collect();
        Chat { cleared_at: cleared_at.filter(|c| valid_ms(*c)), messages }
    }

    pub fn get(&self, id: &str) -> Option<&ChatMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// `{clearedAt?, messages}`.
    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        if let Some(c) = self.cleared_at {
            o.insert("clearedAt".into(), json!(c));
        }
        o.insert("messages".into(), Json::Array(self.messages.iter().map(ChatMessage::to_json).collect()));
        Json::Object(o)
    }

    /// The JSON form, or the file part (`version` 1). `Err` when it isn't a chat at all; bad
    /// messages are left out ([`Chat::sanitized`]).
    pub fn from_json(j: &Json) -> Result<Chat, String> {
        let o = j.as_object().ok_or("the chat isn't a JSON object")?;
        if let Some(v) = o.get("version")
            && v.as_i64() != Some(PART_VERSION)
        {
            return Err(format!("unknown chat version {v}"));
        }
        let list = o.get("messages").and_then(Json::as_array).ok_or("the chat has no list of messages")?;
        Ok(Chat::sanitized(o.get("clearedAt").and_then(json_ms), list.iter().filter_map(ChatMessage::from_json)))
    }

    /// The file part's bytes: `{"version": 1, "clearedAt"?, "messages": […]}`.
    pub fn to_part(&self) -> Vec<u8> {
        let mut j = self.to_json();
        if let Some(o) = j.as_object_mut() {
            o.insert("version".into(), json!(PART_VERSION));
        }
        serde_json::to_vec_pretty(&j).unwrap_or_default()
    }

    /// Reads a file part. `Err` (the caller warns and opens the file without a chat) when it
    /// isn't JSON or isn't a chat.
    pub fn from_part(bytes: &[u8]) -> Result<Chat, String> {
        let j: Json = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
        Chat::from_json(&j)
    }

    /// `chat.send`: appends `m` (its text checked, its reply existing, the chat not full).
    pub fn send(&mut self, m: ChatMessage) -> Result<(), String> {
        valid_text(m.text.as_deref().unwrap_or(""))?;
        if !valid_id(&m.id) || !m.author.valid() || !valid_ms(m.at) {
            return Err("A message needs an id, an author and a time".into());
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(CHAT_FULL.into());
        }
        if self.get(&m.id).is_some() {
            return Err(format!("There's already a message {}", m.id));
        }
        if let Some(r) = &m.reply_to
            && self.get(r).is_none()
        {
            return Err(format!("There's no message {r} to reply to"));
        }
        self.messages.push(m.normalized());
        Ok(())
    }

    fn own(&mut self, id: &str, author: &str) -> Result<&mut ChatMessage, String> {
        let m = self.messages.iter_mut().find(|m| m.id == id).ok_or_else(|| format!("There's no message {id}"))?;
        if m.author.id != author {
            return Err("Only its author can change a message".into());
        }
        if m.deleted {
            return Err("The message was deleted".into());
        }
        Ok(m)
    }

    /// `chat.edit`: the author only, not a deleted message.
    pub fn edit(&mut self, id: &str, author: &str, text: &str, at: i64) -> Result<(), String> {
        valid_text(text)?;
        let m = self.own(id, author)?;
        m.text = Some(text.to_string());
        m.edited = Some(at.clamp(0, MAX_MS));
        Ok(())
    }

    /// `chat.delete`: the author only; the entry stays, without its text.
    pub fn delete(&mut self, id: &str, author: &str) -> Result<(), String> {
        let m = self.own(id, author)?;
        m.text = None;
        m.deleted = true;
        Ok(())
    }

    /// `chat.clear`: every message out, `clearedAt` set. The number of messages removed.
    pub fn clear(&mut self, at: i64) -> usize {
        let n = self.messages.len();
        self.messages.clear();
        self.cleared_at = Some(at.clamp(0, MAX_MS));
        n
    }

    /// `chat.list {since?, limit?}`: the messages after the one whose id is `since` (all without
    /// it, or when it's gone), the newest `limit` of those.
    pub fn list(&self, since: Option<&str>, limit: Option<usize>) -> &[ChatMessage] {
        let start = since.and_then(|id| self.messages.iter().position(|m| m.id == id)).map(|i| i + 1).unwrap_or(0);
        let after = self.messages.get(start..).unwrap_or_default();
        let from = after.len().saturating_sub(limit.unwrap_or(usize::MAX));
        after.get(from..).unwrap_or_default()
    }

    /// The chat as plain text, one message per paragraph: `[2026-10-16 09:30 UTC] Name: text`
    /// (`offset_min` minutes from UTC shift the times; 0 writes UTC).
    pub fn to_text(&self, offset_min: i32) -> String {
        let mut out = String::new();
        for m in &self.messages {
            let when = format_time(m.at, offset_min);
            let reply = m.reply_to.as_deref().and_then(|r| self.get(r)).map(|r| format!(" (replying to {})", r.author.name)).unwrap_or_default();
            let body = match (&m.text, m.deleted) {
                (_, true) | (None, _) => "Message deleted".to_string(),
                (Some(t), false) => t.replace('\n', "\n    "),
            };
            let edited = if m.edited.is_some() && !m.deleted { " (edited)" } else { "" };
            out.push_str(&format!("[{when}] {}{reply}: {body}{edited}\n", m.author.name));
        }
        out
    }
}

/// `2026-10-16 09:30 UTC` (or `+02:00` with an offset).
pub fn format_time(ms: i64, offset_min: i32) -> String {
    let local = ms.saturating_add(i64::from(offset_min) * 60_000);
    let days = local.div_euclid(86_400_000);
    let mins = local.rem_euclid(86_400_000) / 60_000;
    let (y, mo, d) = gridcraft_core::date::civil_from_days(days);
    let zone = if offset_min == 0 {
        "UTC".to_string()
    } else {
        let sign = if offset_min < 0 { '-' } else { '+' };
        let a = offset_min.unsigned_abs();
        format!("{sign}{:02}:{:02}", a / 60, a % 60)
    };
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02} {zone}", mins / 60, mins % 60)
}

impl Serialize for Chat {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Chat {
    /// Lenient: a chat that isn't one reads as an empty chat, never failing the workbook.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Chat, D::Error> {
        let j = Json::deserialize(d)?;
        Ok(Chat::from_json(&j).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(id: &str) -> ChatAuthor {
        ChatAuthor { id: id.into(), name: id.to_uppercase(), color: "#112233".into() }
    }

    fn msg(id: &str, who: &str, text: &str) -> ChatMessage {
        ChatMessage { id: id.into(), author: author(who), at: 1_760_000_000_000, text: Some(text.into()), ..ChatMessage::default() }
    }

    #[test]
    fn send_edit_delete_clear() {
        let mut c = Chat::default();
        c.send(msg("aaaaaaaa", "a", "hi")).unwrap();
        assert!(c.send(msg("aaaaaaaa", "a", "again")).is_err(), "duplicate id");
        assert!(c.send(msg("bbbbbbbb", "a", "")).is_err(), "empty");
        assert!(c.send(msg("bbbbbbbb", "a", &"x".repeat(MAX_TEXT + 1))).is_err(), "too long");
        assert!(c.send(msg("BAD", "a", "x")).is_err(), "bad id");
        assert!(c.send(ChatMessage { reply_to: Some("zzzzzzzz".into()), ..msg("bbbbbbbb", "a", "x") }).is_err(), "no such reply");
        c.send(msg("bbbbbbbb", "b", &"é".repeat(MAX_TEXT))).unwrap();
        assert!(c.edit("aaaaaaaa", "b", "not mine", 1).is_err());
        c.edit("aaaaaaaa", "a", "hello", 7).unwrap();
        assert!(c.delete("bbbbbbbb", "a").is_err());
        c.delete("bbbbbbbb", "b").unwrap();
        assert!(c.edit("bbbbbbbb", "b", "back", 8).is_err(), "deleted");
        assert_eq!(c.messages[0].text.as_deref(), Some("hello"));
        assert_eq!(c.messages[0].edited, Some(7));
        assert!(c.messages[1].deleted && c.messages[1].text.is_none());
        assert_eq!(Chat::from_part(&c.to_part()).unwrap(), c);
        assert_eq!(c.clear(9), 2);
        assert_eq!(c, Chat { cleared_at: Some(9), messages: vec![] });
        assert!(!c.is_empty(), "a cleared chat remembers when");
    }

    #[test]
    fn the_limit() {
        let mut c = Chat::default();
        for i in 0..MAX_MESSAGES {
            c.messages.push(msg(&format!("{i:08}"), "a", "m"));
        }
        assert_eq!(c.send(msg("zzzzzzzz", "a", "one more")), Err(CHAT_FULL.to_string()));
        c.delete("00000000", "a").unwrap();
        assert_eq!(c.send(msg("zzzzzzzz", "a", "one more")), Err(CHAT_FULL.to_string()), "a deleted message still counts");
        assert_eq!(c.clear(1), MAX_MESSAGES);
        c.send(msg("zzzzzzzz", "a", "one more")).unwrap();
        // Readers keep the first 10,000 valid messages.
        let many = (0..MAX_MESSAGES + 5).map(|i| msg(&format!("{i:08}"), "a", "m"));
        let read = Chat::sanitized(None, std::iter::once(msg("BAD", "a", "x")).chain(many));
        assert_eq!(read.messages.len(), MAX_MESSAGES);
        assert_eq!(read.messages.last().map(|m| m.id.as_str()), Some("00009999"));
    }

    #[test]
    fn malformed_parts_are_refused_and_bad_messages_left_out() {
        assert!(Chat::from_part(b"not json").is_err());
        assert!(Chat::from_part(b"[]").is_err());
        assert!(Chat::from_part(br#"{"version": 2, "messages": []}"#).is_err());
        assert!(Chat::from_part(br#"{"version": 1}"#).is_err());
        let c = Chat::from_part(
            br##"{"version": 1, "clearedAt": "x", "messages": [
            {"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "ok"},
            {"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "duplicate"},
            {"id": "AAAAAAAA", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "bad id"},
            {"id": "bbbbbbbb", "author": "A", "at": 1, "text": "bad author"},
            {"id": "cccccccc", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": -1, "text": "bad at"},
            {"id": "dddddddd", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1},
            {"id": "eeeeeeee", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "deleted": true, "text": "gone", "replyTo": 5},
            3
        ]}"##,
        )
        .unwrap();
        assert_eq!(c.cleared_at, None);
        assert_eq!(c.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["aaaaaaaa", "eeeeeeee"]);
        assert!(c.messages[1].text.is_none() && c.messages[1].reply_to.is_none());
    }

    #[test]
    fn list_mentions_and_text() {
        let mut c = Chat { cleared_at: None, messages: ["aaaaaaaa", "bbbbbbbb", "cccccccc"].iter().map(|id| msg(id, "a", "x")).collect() };
        assert_eq!(c.list(Some("aaaaaaaa"), Some(1)).iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["cccccccc"]);
        assert_eq!(c.list(None, None).len(), 3);
        assert_eq!(c.list(Some("gone0000"), None).len(), 3);
        c.messages[0].text = Some("Thanks @Ann Lee, and @bob.".into());
        assert!(c.messages[0].mentions("ann lee") && c.messages[0].mentions("Bob"));
        assert!(!c.messages[0].mentions("Ann Leeds") && !c.messages[0].mentions("bo"));
        assert!(!msg("dddddddd", "a", "mail me at x@bob.org").mentions("bob"));
        c.messages[1].reply_to = Some("aaaaaaaa".into());
        c.messages[2].deleted = true;
        c.messages[2].text = None;
        let text = c.to_text(0);
        assert!(text.starts_with("[2025-10-09 08:53 UTC] A: Thanks @Ann Lee, and @bob.\n"), "{text}");
        assert!(text.contains("A (replying to A): x") && text.contains("A: Message deleted"), "{text}");
        assert_eq!(format_time(0, 120), "1970-01-01 02:00 +02:00");
    }

    #[test]
    fn serde_is_lenient() {
        let c: Chat = serde_json::from_str(r#"{"messages": "nope"}"#).unwrap();
        assert!(c.is_empty());
        let c: Chat = serde_json::from_str(
            r##"{"clearedAt": 5, "messages": [{"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "ok"}]}"##,
        )
        .unwrap();
        assert_eq!(c.messages.len(), 1);
        assert_eq!(serde_json::to_value(&c).unwrap(), c.to_json());
    }
}
