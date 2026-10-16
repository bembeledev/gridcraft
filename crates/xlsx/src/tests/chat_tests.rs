//! The document chat's part (`/craft/chat.json`, craft-collab `docs/chat.md`): written when the
//! chat isn't empty, read back the same, and a damaged one skipped with a warning.

use std::io::Read;
use std::sync::Arc;

use gridcraft_model::{Chat, ChatAuthor, ChatMessage, Workbook};

use super::make_zip;
use crate::{CHAT_CONTENT_TYPE, CHAT_PART, CHAT_REL, read_xlsx, write_xlsx};

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn chat() -> Chat {
    let ann = ChatAuthor { id: "u-ann".into(), name: "Ann Lee".into(), color: "#3B7DD8".into() };
    let bob = ChatAuthor { id: "u-bob".into(), name: "Bob".into(), color: "#D8513B".into() };
    let mut c = Chat::default();
    c.send(ChatMessage {
        id: "aaaaaaaa".into(),
        author: ann.clone(),
        at: 1_760_000_000_000,
        text: Some("Totals first?\n@Bob".into()),
        ..Default::default()
    })
    .unwrap();
    c.send(ChatMessage {
        id: "bbbbbbbb".into(),
        author: bob.clone(),
        at: 1_760_000_060_000,
        text: Some("Yes — see https://example.org".into()),
        reply_to: Some("aaaaaaaa".into()),
        ..Default::default()
    })
    .unwrap();
    c.send(ChatMessage { id: "cccccccc".into(), author: bob, at: 1_760_000_120_000, text: Some("oops".into()), ..Default::default() }).unwrap();
    c.edit("aaaaaaaa", "u-ann", "Totals first? @Bob", 1_760_000_180_000).unwrap();
    c.delete("cccccccc", "u-bob").unwrap();
    c
}

#[test]
fn the_chat_round_trips() {
    let mut wb = Workbook::new();
    wb.chat = Arc::new(chat());
    let bytes = write_xlsx(&wb).unwrap();
    let rels = part(&bytes, "_rels/.rels").unwrap();
    assert!(rels.contains(&format!("Type=\"{CHAT_REL}\" Target=\"{CHAT_PART}\"")), "{rels}");
    let ct = part(&bytes, "[Content_Types].xml").unwrap();
    assert!(ct.contains(&format!("PartName=\"/{CHAT_PART}\" ContentType=\"{CHAT_CONTENT_TYPE}\"")), "{ct}");
    let json: serde_json::Value = serde_json::from_str(&part(&bytes, CHAT_PART).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["messages"][2]["deleted"], true);
    assert!(json["messages"][2].get("text").is_none());
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(*back.chat, chat());
    // Saved again: the same part once (it isn't also kept as unknown content).
    let again = write_xlsx(&back).unwrap();
    assert_eq!(part(&again, "_rels/.rels").unwrap().matches(CHAT_REL).count(), 1);
    assert_eq!(*read_xlsx(&again).unwrap().0.chat, chat());
}

#[test]
fn an_empty_chat_writes_no_part_but_a_cleared_one_does() {
    let bytes = write_xlsx(&Workbook::new()).unwrap();
    assert!(part(&bytes, CHAT_PART).is_none());
    assert!(!part(&bytes, "_rels/.rels").unwrap().contains(CHAT_REL));
    let mut wb = Workbook::new();
    let mut c = chat();
    c.clear(1_760_000_240_000);
    wb.chat = Arc::new(c.clone());
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    assert_eq!(*back.chat, c);
}

fn with_chat_part(content: &[u8]) -> Vec<u8> {
    let wb = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let wb_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#;
    let root = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId9" Type="urn:craft:relationships:chat" Target="/craft/chat.json"/></Relationships>"#;
    make_zip(&[
        ("[Content_Types].xml", super::CT.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("craft/chat.json", content),
    ])
}

#[test]
fn a_damaged_chat_is_skipped_with_a_warning() {
    for bad in [&b"{not json"[..], b"[1, 2]", br#"{"version": 9, "messages": []}"#, br#"{"version": 1, "messages": {}}"#, b"\xff\xfe"] {
        let (wb, report) = read_xlsx(&with_chat_part(bad)).unwrap();
        assert!(wb.chat.is_empty());
        assert!(report.warnings.iter().any(|w| w.contains("document chat")), "{:?}", report.warnings);
        assert_eq!(wb.sheets[0].cells.len(), 1, "the workbook still opens");
    }
    // Bad messages inside a good part are left out, the rest kept.
    let mixed = br##"{"version": 1, "messages": [
        {"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 5, "text": "kept"},
        {"id": "short", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 5, "text": "bad id"},
        {"id": "bbbbbbbb", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 5, "text": ""}
    ]}"##;
    let (wb, report) = read_xlsx(&with_chat_part(mixed)).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("chat")), "{:?}", report.warnings);
    assert_eq!(wb.chat.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["aaaaaaaa"]);
}
