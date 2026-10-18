//! Audit a `.xlsx` file for hidden information before sharing.
//!
//! [`audit_bytes`] scans the raw OPC package (the ZIP bytes) and reports what a
//! recipient of the file could learn: hidden rows, columns and sheets (the data
//! stays in the file), external workbook links, author names on comments,
//! creator/last-modified-by in document properties, pivot caches that embed
//! source data, macro/embedded-object parts, and package parts nothing
//! references.
//!
//! The scan reads raw package bytes, never the `gridcraft-xlsx` document
//! model: the model drops unmodeled parts on round-trip, so a model-based
//! audit could miss exactly the things being looked for.
//!
//! The audit is visibility only: it never modifies the file. Sanitizing
//! (removing what it finds) is a separate, future step.
//!
//! Detection notes (documented, not hidden):
//!
//! - Matching is by namespace URI, not by prefix, so a file that spells the
//!   SpreadsheetML namespace with an unusual prefix is still scanned.
//! - `hidden` on rows, columns and defined names counts when its value is
//!   true-ish (`1`, `true`, `on`, `yes`); absent or false-ish is not flagged.
//! - `state` on a sheet counts when it is `hidden` or `veryHidden`
//!   (ECMA-376 §18.2.20 `ST_SheetState`).
//! - Threaded-comment elements are accepted in the main SpreadsheetML
//!   namespace and in the 2018 threaded-comments namespace, since producers
//!   vary; everything else is matched strictly by namespace URI.
//! - A `dc:creator` / `cp:lastModifiedBy` value is reported as-is; producers
//!   that anonymize are reported too, since the value is what a recipient sees.
//!
//! The audit fails closed: a part that cannot be read or parsed is an error,
//! never a silent pass.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use serde::Serialize;

#[cfg(test)]
mod tests;

// ECMA-376 namespaces the audit understands.
const NS_X: &[u8] = b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_THREADED: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
const NS_DC: &[u8] = b"http://purl.org/dc/elements/1.1/";
const NS_CP: &[u8] = b"http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const NS_EP: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const NS_R: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_RELS: &[u8] = b"http://schemas.openxmlformats.org/package/2006/relationships";

/// Cap on ZIP entries read: a package is a handful of parts, not millions.
const MAX_PARTS: usize = 100_000;
/// Cap on decompressed bytes read from a single part (zip-bomb guard).
const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
/// Cap on XML elements walked in a single part.
const MAX_XML_ELEMENTS: u64 = 2_000_000;

/// What the audit found, as a stable machine-readable vocabulary.
///
/// The [`FindingKind::id`] strings are the crate's public contract: they appear
/// in CLI output and serialized reports. Do not rename them; add a new variant
/// instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// A `row` with `hidden` set in a worksheet (ECMA-376 §18.3.1.73):
    /// the row's cells stay in the file.
    HiddenRow,
    /// A `col` with `hidden` set in a worksheet (ECMA-376 §18.3.1.17):
    /// the columns' cells stay in the file.
    HiddenColumn,
    /// A `sheet` with `state="hidden"` or `"veryHidden"` in `xl/workbook.xml`
    /// (ECMA-376 §18.2.28, `ST_SheetState` §18.18.68).
    HiddenSheet,
    /// A `definedName` with `hidden` set in `xl/workbook.xml`
    /// (ECMA-376 §18.2.6).
    HiddenDefinedName,
    /// An `externalReference` in `xl/workbook.xml` (ECMA-376 §18.2.8):
    /// the workbook pulls data from another file.
    ExternalLink,
    /// An `author` on a legacy comment in `xl/comments/*.xml`
    /// (ECMA-376 §18.7.3).
    CommentAuthor,
    /// A `displayName` on a threaded comment in
    /// `xl/threadedComments/*.xml`, resolved through `xl/persons/person.xml`.
    ThreadedCommentAuthor,
    /// `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml`
    /// (ECMA-376 Part 2 §12.3.1).
    CorePropertiesAuthor,
    /// `Company` / `Manager` in `docProps/app.xml` (ECMA-376 Part 2 §12.3.2).
    ExtendedProperties,
    /// A part under `xl/pivotCache/`: the cache can embed the source data
    /// the pivot was built from.
    PivotCache,
    /// An `xl/vbaProject.bin` part: the workbook carries macros.
    VbaProject,
    /// A part under `xl/embeddings/`: an embedded (often OLE) object.
    OleEmbedding,
    /// A ZIP entry no relationship targets: leftover content nothing references.
    OrphanPart,
}

impl FindingKind {
    /// The stable machine-readable ID for this finding class.
    pub fn id(self) -> &'static str {
        match self {
            FindingKind::HiddenRow => "hidden/row",
            FindingKind::HiddenColumn => "hidden/column",
            FindingKind::HiddenSheet => "hidden/sheet",
            FindingKind::HiddenDefinedName => "hidden/defined-name",
            FindingKind::ExternalLink => "external-link",
            FindingKind::CommentAuthor => "author-pii/comment",
            FindingKind::ThreadedCommentAuthor => "author-pii/threaded-comment",
            FindingKind::CorePropertiesAuthor => "author-pii/core-properties",
            FindingKind::ExtendedProperties => "author-pii/extended-properties",
            FindingKind::PivotCache => "pivot-cache",
            FindingKind::VbaProject => "active-content/vba",
            FindingKind::OleEmbedding => "active-content/ole",
            FindingKind::OrphanPart => "package/orphan-part",
        }
    }

    /// The human-readable title for this finding class.
    pub fn title(self) -> &'static str {
        match self {
            FindingKind::HiddenRow => "hidden row",
            FindingKind::HiddenColumn => "hidden column",
            FindingKind::HiddenSheet => "hidden worksheet",
            FindingKind::HiddenDefinedName => "hidden defined name",
            FindingKind::ExternalLink => "external workbook link",
            FindingKind::CommentAuthor => "author name on comment",
            FindingKind::ThreadedCommentAuthor => "author name on threaded comment",
            FindingKind::CorePropertiesAuthor => "author name in document properties",
            FindingKind::ExtendedProperties => "organization name in extended properties",
            FindingKind::PivotCache => "pivot cache with source data",
            FindingKind::VbaProject => "VBA macro project",
            FindingKind::OleEmbedding => "embedded object",
            FindingKind::OrphanPart => "orphan package part",
        }
    }

    /// How much attention this class deserves: `warn` for identifying,
    /// executable or data-carrying content, `info` for leftovers worth
    /// knowing about.
    pub fn severity(self) -> &'static str {
        match self {
            FindingKind::HiddenDefinedName | FindingKind::ExtendedProperties | FindingKind::OrphanPart => "info",
            _ => "warn",
        }
    }
}

/// One thing the audit found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditFinding {
    /// Stable finding-class ID (see [`FindingKind::id`]).
    pub id: String,
    /// Human-readable title (see [`FindingKind::title`]).
    pub title: String,
    /// `"warn"` or `"info"`.
    pub severity: String,
    /// Where it was found: the part name plus the element or attribute,
    /// e.g. `xl/worksheets/sheet1.xml <row>`.
    pub location: String,
    /// The matched value: a row number, an author name, a part name.
    pub detail: String,
}

impl AuditFinding {
    fn new(kind: FindingKind, location: String, detail: String) -> AuditFinding {
        AuditFinding { id: kind.id().to_string(), title: kind.title().to_string(), severity: kind.severity().to_string(), location, detail }
    }
}

/// Why the audit could not run.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// The bytes are not a readable ZIP archive.
    #[error("not a readable ZIP archive: {0}")]
    Zip(String),
    /// A package part needed for the audit could not be parsed as XML.
    #[error("part {0}: {1}")]
    Xml(String, String),
    /// A hostile-input guard tripped (too many parts, part too large, …).
    #[error("{0}")]
    Limit(String),
}

/// Scan the raw `.xlsx` (OPC ZIP) bytes and return every finding.
///
/// Findings come back in a deterministic order (by class, location, detail).
/// Fails closed: unreadable or unparsable input is an [`AuditError`], never an
/// empty finding list that looks like a clean bill of health.
pub fn audit_bytes(bytes: &[u8]) -> Result<Vec<AuditFinding>, AuditError> {
    let names = part_names(bytes)?;
    let mut parts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for name in names.iter().filter(|n| is_scanned_part(n)) {
        if let Some(data) = read_part(bytes, name)? {
            parts.insert(name.clone(), data);
        }
    }

    let mut findings = Vec::new();
    let workbook = match parts.get("xl/workbook.xml") {
        Some(xml) => xml,
        None => {
            return Err(AuditError::Xml("xl/workbook.xml".to_string(), "missing: not a spreadsheet package".to_string()));
        }
    };
    let sheet_names = scan_workbook(workbook, &names, &parts, &mut findings)?;
    for (part, data) in parts.iter().filter(|(n, _)| n.starts_with("xl/worksheets/") && n.ends_with(".xml")) {
        let sheet_label = sheet_names.get(part).map(|n| format!(" ({n})")).unwrap_or_default();
        scan_worksheet(part, &sheet_label, data, &mut findings)?;
    }
    for (part, data) in parts.iter().filter(|(n, _)| n.starts_with("xl/comments/") && n.ends_with(".xml")) {
        scan_comments(part, data, &mut findings)?;
    }
    let persons = collect_persons(&parts)?;
    for (part, data) in parts.iter().filter(|(n, _)| n.starts_with("xl/threadedComments/") && n.ends_with(".xml")) {
        scan_threaded_comments(part, data, &persons, &mut findings)?;
    }
    if let Some(xml) = parts.get("docProps/core.xml") {
        scan_core_properties(xml, &mut findings)?;
    }
    if let Some(xml) = parts.get("docProps/app.xml") {
        scan_extended_properties(xml, &mut findings)?;
    }
    scan_package_parts(&names, &mut findings);
    scan_orphans(&names, &parts, &mut findings)?;

    findings.sort_by(|a, b| (&a.id, &a.location, &a.detail).cmp(&(&b.id, &b.location, &b.detail)));
    Ok(findings)
}

/// Parts the audit parses as XML. Everything else is only listed by name.
fn is_scanned_part(name: &str) -> bool {
    name == "xl/workbook.xml"
        || (name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
        || (name.starts_with("xl/comments/") && name.ends_with(".xml"))
        || (name.starts_with("xl/threadedComments/") && name.ends_with(".xml"))
        || (name.starts_with("xl/persons/") && name.ends_with(".xml"))
        || name == "docProps/core.xml"
        || name == "docProps/app.xml"
        || name.ends_with(".rels")
}

/// Every non-directory entry name in the ZIP, in archive order.
fn part_names(bytes: &[u8]) -> Result<Vec<String>, AuditError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| AuditError::Zip(e.to_string()))?;
    let len = archive.len();
    if len > MAX_PARTS {
        return Err(AuditError::Limit(format!("too many ZIP entries: {len}")));
    }
    let mut names = Vec::with_capacity(len.min(1024));
    for i in 0..len {
        let file = archive.by_index(i).map_err(|e| AuditError::Zip(e.to_string()))?;
        let name = file.name().to_string();
        if !name.ends_with('/') {
            names.push(name);
        }
    }
    Ok(names)
}

/// Read one part, capped against decompression bombs. `Ok(None)` when the part
/// vanished between listing and reading (the archive is not re-read otherwise).
fn read_part(bytes: &[u8], name: &str) -> Result<Option<Vec<u8>>, AuditError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| AuditError::Zip(e.to_string()))?;
    let mut file = match archive.by_name(name) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let mut data = Vec::new();
    let mut limited = (&mut file).take(MAX_PART_BYTES + 1);
    limited.read_to_end(&mut data).map_err(|e| AuditError::Zip(format!("{name}: {e}")))?;
    if data.len() as u64 > MAX_PART_BYTES {
        return Err(AuditError::Limit(format!("part too large: {name}")));
    }
    Ok(Some(data))
}

/// One element the walker saw: namespace URI, local name, decoded attributes.
struct Element {
    ns: Vec<u8>,
    local: String,
    /// (namespace URI, local name, decoded value) per attribute.
    attrs: Vec<(Vec<u8>, String, String)>,
}

/// Call `f` for every start/empty element in `xml`, resolving names by
/// namespace URI so producer-chosen prefixes do not matter.
fn for_each_element(xml: &[u8], part: &str, f: &mut impl FnMut(&Element) -> Result<(), AuditError>) -> Result<(), AuditError> {
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        config.check_end_names = false;
    }
    let mut buf = Vec::new();
    let mut count: u64 = 0;
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        // Copy the namespace URI out so the reader borrow ends here; the
        // attribute resolver below needs `&mut reader` again.
        let ns: Vec<u8> = match &resolution {
            ResolveResult::Bound(n) => n.0.to_vec(),
            _ => Vec::new(),
        };
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                f(&element_of(&mut reader, &ns, e, part)?)?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

/// Resolve one start tag into an [`Element`].
fn element_of(reader: &mut NsReader<&[u8]>, ns: &[u8], e: &BytesStart<'_>, part: &str) -> Result<Element, AuditError> {
    let ns = ns.to_vec();
    let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|err| AuditError::Xml(part.to_string(), err.to_string()))?;
        let key = attr.key;
        if key.as_ref() == b"xmlns" || key.as_ref().starts_with(b"xmlns:") {
            continue;
        }
        let (aresolution, alocal) = reader.resolver_mut().resolve_attribute(key);
        let ans = match aresolution {
            ResolveResult::Bound(ns) => ns.0.to_vec(),
            _ => Vec::new(),
        };
        let alocal = String::from_utf8_lossy(alocal.as_ref()).into_owned();
        let value = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned());
        attrs.push((ans, alocal, value));
    }
    Ok(Element { ns, local, attrs })
}

/// The value of the attribute with namespace `ns` and local name `local`, if present.
/// SpreadsheetML attributes like `hidden`, `state` and `r` are unqualified:
/// pass `b""` as the namespace.
fn attr<'a>(el: &'a Element, ns: &[u8], local: &str) -> Option<&'a str> {
    el.attrs.iter().find(|(ans, aname, _)| ans.as_slice() == ns && aname == local).map(|(_, _, v)| v.as_str())
}

/// Boolean-ish attribute values: absent is false; only true-ish spellings count.
fn is_trueish(value: Option<&str>) -> bool {
    matches!(value.map(|v| v.to_ascii_lowercase()).as_deref(), Some("1" | "true" | "on" | "yes"))
}

/// Collect the text content of every element matching (`ns`, `local`).
fn texts_of(xml: &[u8], part: &str, ns: &[u8], local: &str) -> Result<Vec<String>, AuditError> {
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        config.check_end_names = false;
    }
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    let mut current = String::new();
    let mut count: u64 = 0;
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                let el_ns: &[u8] = match &resolution {
                    ResolveResult::Bound(n) => n.0,
                    _ => b"",
                };
                let el_local = e.local_name();
                let el_local = String::from_utf8_lossy(el_local.as_ref());
                if depth == 0 && el_ns == ns && el_local.as_ref() == local {
                    depth = 1;
                    current.clear();
                } else if depth > 0 {
                    depth = depth.saturating_add(1);
                }
                if matches!(event, Event::Empty(_)) && depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        out.push(std::mem::take(&mut current));
                    }
                }
            }
            Event::End(_) => {
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        out.push(std::mem::take(&mut current));
                    }
                }
            }
            Event::Text(ref t) => {
                if depth > 0 {
                    let decoded = t.xml10_content().map(|v| v.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(t.as_ref()).into_owned());
                    current.push_str(&decoded);
                }
            }
            Event::CData(ref t) => {
                if depth > 0 {
                    current.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// 1-based column index to A1 letters: 1 -> "A", 28 -> "AB".
fn col_letters(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push((b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    letters.iter().rev().collect()
}

/// `xl/workbook.xml`: hidden/veryHidden sheets, hidden defined names, external
/// references. Returns the sheet part -> sheet name map for worksheet locations.
fn scan_workbook(
    xml: &[u8],
    names: &[String],
    parts: &BTreeMap<String, Vec<u8>>,
    findings: &mut Vec<AuditFinding>,
) -> Result<BTreeMap<String, String>, AuditError> {
    // r:id -> relationship target, from xl/_rels/workbook.xml.rels.
    let mut rel_targets: BTreeMap<String, String> = BTreeMap::new();
    if let Some(rels) = parts.get("xl/_rels/workbook.xml.rels") {
        for_each_element(rels, "xl/_rels/workbook.xml.rels", &mut |el| {
            if el.ns.as_slice() == NS_RELS
                && el.local == "Relationship"
                && let (Some(id), Some(target)) = (attr(el, b"", "Id"), attr(el, b"", "Target"))
                && attr(el, b"", "TargetMode") != Some("External")
            {
                rel_targets.insert(id.to_string(), resolve_target("xl", target));
            }
            Ok(())
        })?;
    }

    let mut sheet_names: BTreeMap<String, String> = BTreeMap::new();
    let mut external_targets: BTreeSet<String> = BTreeSet::new();
    for_each_element(xml, "xl/workbook.xml", &mut |el| {
        if el.ns.as_slice() != NS_X {
            return Ok(());
        }
        match el.local.as_str() {
            "sheet" => {
                let name = attr(el, b"", "name").unwrap_or("?");
                if let Some(rid) = attr(el, NS_R, "id")
                    && let Some(target) = rel_targets.get(rid)
                {
                    sheet_names.insert(target.clone(), name.to_string());
                }
                match attr(el, b"", "state") {
                    Some("hidden") | Some("veryHidden") => {
                        findings.push(AuditFinding::new(
                            FindingKind::HiddenSheet,
                            format!("xl/workbook.xml <sheet name=\"{name}\">"),
                            format!("state=\"{}\"", attr(el, b"", "state").unwrap_or("?")),
                        ));
                    }
                    _ => {}
                }
            }
            "definedName" => {
                if is_trueish(attr(el, b"", "hidden")) {
                    let name = attr(el, b"", "name").unwrap_or("?");
                    findings.push(AuditFinding::new(
                        FindingKind::HiddenDefinedName,
                        "xl/workbook.xml <definedName>".to_string(),
                        format!("name=\"{name}\""),
                    ));
                }
            }
            "externalReference" => {
                if let Some(rid) = attr(el, NS_R, "id") {
                    let target = rel_targets.get(rid).cloned().unwrap_or_else(|| format!("(unresolved {rid})"));
                    external_targets.insert(target);
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    for target in &external_targets {
        findings.push(AuditFinding::new(FindingKind::ExternalLink, "xl/workbook.xml <externalReference>".to_string(), target.clone()));
    }
    // Belt and suspenders: an externalLink part nothing references still counts.
    for name in names.iter().filter(|n| n.starts_with("xl/externalLinks/")) {
        if !external_targets.contains(name) {
            findings.push(AuditFinding::new(FindingKind::ExternalLink, name.clone(), "external workbook link part".to_string()));
        }
    }
    Ok(sheet_names)
}

/// `xl/worksheets/*.xml`: hidden rows and hidden columns. The cells stay in
/// the file; hiding is presentation, not removal.
fn scan_worksheet(part: &str, sheet_label: &str, xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    let here = format!("{part}{sheet_label}");
    for_each_element(xml, part, &mut |el| {
        if el.ns.as_slice() != NS_X {
            return Ok(());
        }
        match el.local.as_str() {
            "row" => {
                if is_trueish(attr(el, b"", "hidden")) {
                    let detail = match attr(el, b"", "r") {
                        Some(r) => format!("hidden row {r}"),
                        None => "hidden row".to_string(),
                    };
                    findings.push(AuditFinding::new(FindingKind::HiddenRow, format!("{here} <row>"), detail));
                }
            }
            "col" if is_trueish(attr(el, b"", "hidden")) => {
                let detail = match (attr(el, b"", "min"), attr(el, b"", "max")) {
                    (Some(min), Some(max)) => {
                        let lo = min.parse::<u32>().unwrap_or(0);
                        let hi = max.parse::<u32>().unwrap_or(0);
                        if lo == hi && lo > 0 {
                            format!("hidden column {}", col_letters(lo))
                        } else if lo > 0 {
                            format!("hidden columns {}:{}", col_letters(lo), col_letters(hi))
                        } else {
                            format!("hidden columns {min}:{max}")
                        }
                    }
                    _ => "hidden column".to_string(),
                };
                findings.push(AuditFinding::new(FindingKind::HiddenColumn, format!("{here} <col>"), detail));
            }
            "col" => {}
            _ => {}
        }
        Ok(())
    })
}

/// `xl/comments/*.xml`: legacy comments; `authorId` indexes the `authors` list
/// (ECMA-376 §18.7.3).
fn scan_comments(part: &str, xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    let authors = texts_of(xml, part, NS_X, "author")?;
    for_each_element(xml, part, &mut |el| {
        if el.ns.as_slice() == NS_X
            && el.local == "comment"
            && let Some(id) = attr(el, b"", "authorId")
        {
            let name = id.parse::<usize>().ok().and_then(|i| authors.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty());
            let cell = attr(el, b"", "ref").unwrap_or("?");
            let detail = match name {
                Some(n) => format!("author=\"{n}\" cell=\"{cell}\""),
                None => format!("authorId=\"{id}\" (unresolved) cell=\"{cell}\""),
            };
            findings.push(AuditFinding::new(FindingKind::CommentAuthor, format!("{part} <comment>"), detail));
        }
        Ok(())
    })
}

/// `person id` -> `displayName` from `xl/persons/*.xml`.
fn collect_persons(parts: &BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, String>, AuditError> {
    let mut persons = BTreeMap::new();
    for (part, xml) in parts.iter().filter(|(n, _)| n.starts_with("xl/persons/") && n.ends_with(".xml")) {
        for_each_element(xml, part, &mut |el| {
            if (el.ns.as_slice() == NS_X || el.ns.as_slice() == NS_THREADED)
                && el.local == "person"
                && let (Some(id), Some(name)) = (attr(el, b"", "id"), attr(el, b"", "displayName"))
                && !name.is_empty()
            {
                persons.insert(id.to_string(), name.to_string());
            }
            Ok(())
        })?;
    }
    Ok(persons)
}

/// `xl/threadedComments/*.xml`: modern comments; `personId` resolves through
/// the persons map. Accepted in the main and the 2018 threaded-comments
/// namespaces, since producers vary.
fn scan_threaded_comments(part: &str, xml: &[u8], persons: &BTreeMap<String, String>, findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for_each_element(xml, part, &mut |el| {
        if (el.ns.as_slice() == NS_X || el.ns.as_slice() == NS_THREADED)
            && el.local == "threadedComment"
            && let Some(pid) = attr(el, b"", "personId")
        {
            let cell = attr(el, b"", "ref").unwrap_or("?");
            let detail = match persons.get(pid) {
                Some(n) => format!("author=\"{n}\" cell=\"{cell}\""),
                None => format!("personId=\"{pid}\" (unresolved) cell=\"{cell}\""),
            };
            findings.push(AuditFinding::new(FindingKind::ThreadedCommentAuthor, format!("{part} <threadedComment>"), detail));
        }
        Ok(())
    })
}

/// `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml`.
fn scan_core_properties(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for (ns, local, element) in [(NS_DC, "creator", "dc:creator"), (NS_CP, "lastModifiedBy", "cp:lastModifiedBy")] {
        for value in texts_of(xml, "docProps/core.xml", ns, local)? {
            let value = value.trim();
            if !value.is_empty() {
                findings.push(AuditFinding::new(FindingKind::CorePropertiesAuthor, format!("docProps/core.xml <{element}>"), value.to_string()));
            }
        }
    }
    Ok(())
}

/// `Company` / `Manager` in `docProps/app.xml`: organizational, not personal,
/// but still identifying — reported at `info` severity.
fn scan_extended_properties(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for local in ["Company", "Manager"] {
        for value in texts_of(xml, "docProps/app.xml", NS_EP, local)? {
            let value = value.trim();
            if !value.is_empty() {
                findings.push(AuditFinding::new(FindingKind::ExtendedProperties, format!("docProps/app.xml <{local}>"), value.to_string()));
            }
        }
    }
    Ok(())
}

/// Parts named by convention: pivot caches (source data can be embedded),
/// macro projects and embedded objects. Reported as visibility, never
/// blocked or removed.
fn scan_package_parts(names: &[String], findings: &mut Vec<AuditFinding>) {
    for name in names {
        if name.starts_with("xl/pivotCache/") {
            findings.push(AuditFinding::new(FindingKind::PivotCache, name.clone(), "pivot cache part (may embed source data)".to_string()));
        } else if name == "xl/vbaProject.bin" {
            findings.push(AuditFinding::new(FindingKind::VbaProject, "xl/vbaProject.bin".to_string(), "VBA macro project present".to_string()));
        } else if name.starts_with("xl/embeddings/") {
            findings.push(AuditFinding::new(FindingKind::OleEmbedding, name.clone(), "embedded object present".to_string()));
        }
    }
}

/// ZIP entries no relationship targets: content nothing references.
/// Reachability is computed lexically from every `*.rels` part; external
/// relationship targets (`TargetMode="External"`) are not package parts.
fn scan_orphans(names: &[String], parts: &BTreeMap<String, Vec<u8>>, findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    reachable.insert("[Content_Types].xml".to_string());
    for name in names {
        if name.ends_with(".rels") {
            reachable.insert(name.clone());
        }
    }
    for name in names.iter().filter(|n| n.ends_with(".rels")) {
        let Some(xml) = parts.get(name) else { continue };
        let source_dir = source_dir_of_rels(name);
        for_each_element(xml, name, &mut |el| {
            if el.ns.as_slice() == NS_RELS
                && el.local == "Relationship"
                && let Some(target) = attr(el, b"", "Target")
                && attr(el, b"", "TargetMode") != Some("External")
            {
                reachable.insert(resolve_target(&source_dir, target));
            }
            Ok(())
        })?;
    }
    for name in names {
        if !reachable.contains(name) {
            findings.push(AuditFinding::new(FindingKind::OrphanPart, name.clone(), "no relationship targets this part".to_string()));
        }
    }
    Ok(())
}

/// The directory of the part a `*.rels` file describes:
/// `xl/_rels/workbook.xml.rels` describes `xl/workbook.xml`, whose directory
/// is `xl`; `_rels/.rels` describes the package root.
fn source_dir_of_rels(rels: &str) -> String {
    let without_ext = rels.strip_suffix(".rels").unwrap_or(rels);
    match without_ext.rfind("/_rels/") {
        Some(i) => without_ext[..i].to_string(),
        None => String::new(),
    }
}

/// Resolve a relationship target against the source part's directory, lexically.
/// Absolute targets (`/xl/...`) resolve from the package root.
fn resolve_target(source_dir: &str, target: &str) -> String {
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else if source_dir.is_empty() {
        target.to_string()
    } else {
        format!("{source_dir}/{target}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}
