use super::{
    Column, DataSheet, Report,
    schema::{self, Type},
};
use gridcraft_core::Value;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Catalog {
    pub enums: BTreeMap<String, BTreeMap<String, String>>,
    pub aliases: BTreeMap<String, String>,
    pub versions: BTreeMap<String, i64>,
    pub keys: BTreeMap<String, BTreeSet<String>>,
    pub ids: BTreeSet<String>,
    pub nonscalar_keys: BTreeSet<String>,
}

pub fn catalog(sheets: &[&DataSheet], target: Option<&str>) -> Catalog {
    let mut c = Catalog::default();
    for sh in sheets {
        add_metadata(&mut c, sh);
    }
    for sh in sheets {
        add_keys(&mut c, sh, target);
    }
    c
}

/// First pass: table metadata, independent of file order.
pub fn add_metadata(c: &mut Catalog, sh: &DataSheet) {
    if sh.ignored() {
        return;
    }
    for col in sh.columns.iter().filter(|c| c.included() && c.has("Key")) {
        if Type::parse(&col.typ).is_ok_and(|t| t.list || (t.nullable() && t.base != "NullableTString")) {
            c.nonscalar_keys.insert(sh.table().into());
        }
    }
    if !["EnumList", "Alias", "ResourceVersion"].contains(&sh.table()) {
        return;
    }
    for row in sh.rows.values() {
        match sh.table() {
            "EnumList" => {
                c.enums.entry(sh.row_text(row, "EnumType")).or_default().insert(sh.row_text(row, "EnumName"), sh.row_text(row, "EnumValue"));
            }
            "Alias" => {
                c.aliases.insert(sh.row_text(row, "AliasId"), sh.row_text(row, "Value"));
            }
            "ResourceVersion" => {
                if let Ok(n) = sh.row_text(row, "ResourceVersionNumber").parse() {
                    c.versions.insert(sh.row_text(row, "ResourceVersionName"), n);
                }
            }
            _ => {}
        }
    }
}

/// Second pass: keys and ID definitions after version filtering.
pub fn add_keys(c: &mut Catalog, sh: &DataSheet, target: Option<&str>) {
    if sh.ignored() {
        return;
    }
    let columns = sh.columns.iter().enumerate().filter(|(_, c)| c.included() && (c.has("Key") || c.has("StringIdDefinition"))).collect::<Vec<_>>();
    if columns.is_empty() {
        return;
    }
    for (_, col) in &columns {
        if col.has("Key") {
            c.keys.entry(sh.table().into()).or_default();
        }
    }
    for values in sh.rows.values() {
        if included(sh, values, c, target) == Some(false) {
            continue;
        }
        for (i, col) in &columns {
            let raw = values.get(*i).map(|v| if col.has("Raw") { v.display() } else { resolve(v, c).display() }).unwrap_or_default();
            if !col.has("Raw") && !col.has("Design") {
                c.ids.extend(string_ids(&raw));
            }
            if col.has("Key") {
                let value = canonical(&raw, col, c);
                c.keys.entry(sh.table().into()).or_default().insert(value);
            }
        }
    }
}

pub fn included(sh: &DataSheet, row: &[Value], c: &Catalog, target: Option<&str>) -> Option<bool> {
    let Some(target) = target.filter(|t| !t.is_empty()) else { return Some(true) };
    let limit = c.versions.get(target)?;
    let version = sh.columns.iter().position(|c| c.has("Version")).and_then(|i| row.get(i)).map(Value::display).unwrap_or_default();
    if version.trim().is_empty() { Some(true) } else { c.versions.get(version.trim()).map(|v| v <= limit) }
}

/// Streaming third pass. Only cross-row constraints stay resident, never whole workbooks.
pub struct Validator {
    pub report: Report,
    catalog: Arc<Catalog>,
    target: Option<String>,
    locations: BTreeSet<(String, String)>,
    sheet_names: BTreeSet<String>,
    schemas: BTreeMap<String, Vec<(String, String, Vec<String>)>>,
    unique: BTreeMap<(String, String), BTreeSet<String>>,
    groups: BTreeMap<(String, String), (String, BTreeSet<String>)>,
    compound: BTreeMap<String, BTreeSet<Vec<String>>>,
}
impl Validator {
    pub fn new(catalog: Arc<Catalog>, target: Option<String>, complete: bool) -> Self {
        Self {
            report: Report { complete_context: complete, ..Default::default() },
            catalog,
            target,
            locations: Default::default(),
            sheet_names: Default::default(),
            schemas: Default::default(),
            unique: Default::default(),
            groups: Default::default(),
            compound: Default::default(),
        }
    }
    pub fn push(&mut self, sh: &DataSheet) {
        if sh.ignored() {
            return;
        }
        let first = self.locations.insert((sh.file.clone(), sh.name.clone()));
        let c = &*self.catalog;
        let target = self.target.as_deref();
        let r = &mut self.report;
        let complete = r.complete_context;
        let sheet_names = &mut self.sheet_names;
        let schemas = &mut self.schemas;
        let unique = &mut self.unique;
        let groups = &mut self.groups;
        let compound = &mut self.compound;
        if first {
            if !schema::valid_sheet(&sh.name) {
                r.push(
                    sh,
                    0,
                    0,
                    "error",
                    "sheet.name",
                    "시트 이름은 영문 대문자로 시작하고, 테이블명에는 영문·숫자만 사용합니다. _ 뒤는 구분용 이름입니다.",
                );
            }
            if !sheet_names.insert(sh.name.clone()) {
                r.push(sh, 0, 0, "error", "sheet.duplicate", "동일한 시트 이름이 다른 파일에도 있습니다.");
            }
            let signature = sh
                .columns
                .iter()
                .filter(|col| col.included())
                .map(|col| {
                    let mut f = schema::flags(&col.flags)
                        .into_iter()
                        .map(|s| match s.split_once('.') {
                            Some((a, b)) => format!("{}.{b}", a.to_lowercase()),
                            None => s.to_lowercase(),
                        })
                        .collect::<Vec<_>>();
                    f.sort();
                    (col.name.clone(), Type::parse(&col.typ).map(|t| t.spelling()).unwrap_or_else(|_| col.typ.clone()), f)
                })
                .collect::<Vec<_>>();
            if let Some(previous) = schemas.get(sh.table()) {
                if previous != &signature {
                    r.push(sh, 2, 0, "error", "sheet.merge", format!("{}로 병합되는 시트들의 필드 순서·타입·플래그가 다릅니다.", sh.table()));
                }
            } else {
                schemas.insert(sh.table().to_string(), signature);
            }
            check_schema(sh, c, complete, r);
            for col in sh.columns.iter().filter(|col| col.included()) {
                for flag in schema::flags(&col.flags) {
                    if let Some((prefix, table)) = flag.split_once('.')
                        && prefix.eq_ignore_ascii_case("FKey")
                    {
                        let invalid = c.nonscalar_keys.contains(table);
                        if invalid {
                            r.push(
                                sh,
                                0,
                                col.index,
                                "error",
                                "fkey.scalar",
                                format!("{table}의 Key는 nullable/list가 아닌 scalar 타입이어야 합니다."),
                            );
                        }
                    }
                }
            }
            if let Some(target) = target.filter(|t| !t.is_empty())
                && !c.versions.contains_key(target)
            {
                r.push(
                    sh,
                    0,
                    0,
                    "warning",
                    "version.target",
                    format!("대상 버전 {target}을 ResourceVersion에서 찾을 수 없어 행 포함 여부를 판정할 수 없습니다."),
                );
            }
        }
        let compound_columns = sh.columns.iter().enumerate().filter(|(_, col)| col.has("CompoundKey")).collect::<Vec<_>>();
        for (&row, values) in &sh.rows {
            if included(sh, values, c, target) == Some(false) {
                r.excluded_rows += 1;
                continue;
            }
            r.included_rows += 1;
            if !compound_columns.is_empty() {
                let key = compound_columns
                    .iter()
                    .map(|(i, col)| canonical(&values.get(*i).map(Value::display).unwrap_or_default(), col, c))
                    .collect::<Vec<_>>();
                if !compound.entry(sh.table().into()).or_default().insert(key) {
                    r.push(
                        sh,
                        row,
                        compound_columns.first().map(|(_, col)| col.index).unwrap_or(0),
                        "error",
                        "key.compound",
                        "CompoundKey 값의 조합이 중복됩니다.",
                    );
                }
            }
            for (i, col) in sh.columns.iter().enumerate().filter(|(_, col)| col.included()) {
                let value = values.get(i).cloned().unwrap_or_default();
                let text = value.display();
                let canonical = canonical(&text, col, c);
                if (col.has("NotBlank") || col.has("Key")) && text.trim().is_empty() {
                    r.push(sh, row, col.index, "error", "value.blank", format!("{}: Key/NotBlank 값이 비어 있습니다.", col.name));
                }
                if col.has("Version") && !text.trim().is_empty() && !c.versions.contains_key(text.trim()) {
                    r.push(
                        sh,
                        row,
                        col.index,
                        if complete { "error" } else { "warning" },
                        "version.unknown",
                        format!("ResourceVersion에 없는 버전: {text}"),
                    );
                }
                if (col.has("Key") || col.has("Unique"))
                    && !unique.entry((sh.table().into(), col.name.clone())).or_default().insert(canonical.clone())
                {
                    r.push(sh, row, col.index, "error", "key.duplicate", format!("{}의 값이 중복됩니다: {text}", col.name));
                }
                if col.has("GroupKey") || col.has("SplitBinaryGroupKey") {
                    let (previous, seen) = groups.entry((sh.table().into(), col.name.clone())).or_default();
                    if *previous != canonical || seen.is_empty() {
                        if !seen.insert(canonical.clone()) {
                            r.push(sh, row, col.index, "error", "key.group", "같은 그룹 값은 연속된 행에 있어야 합니다.");
                        }
                        *previous = canonical.clone();
                    }
                }
                if col.has("Design") {
                    continue;
                }
                if !col.has("Raw") {
                    for alias in aliases(&text) {
                        if !c.aliases.contains_key(&alias) {
                            r.push(
                                sh,
                                row,
                                col.index,
                                if complete { "error" } else { "warning" },
                                "alias.missing",
                                format!("정의되지 않은 Alias: {alias}"),
                            );
                        }
                    }
                    if col.has("StringIdReference") {
                        for id in string_ids(&resolve(&value, c).display()) {
                            if !c.ids.contains(&id) {
                                r.push(
                                    sh,
                                    row,
                                    col.index,
                                    if complete { "error" } else { "warning" },
                                    "id.undefined",
                                    format!("현재 데이터에 정의되지 않은 문자열 ID: {id}"),
                                );
                            }
                        }
                    }
                }
                if let Ok(typ) = Type::parse(&col.typ) {
                    let resolved = if col.has("Raw") { value.clone() } else { resolve(&value, c) };
                    if !col.has("Raw") && !aliases(&resolved.display()).is_empty() {
                        r.push(
                            sh,
                            row,
                            col.index,
                            if complete { "error" } else { "warning" },
                            "alias.unresolved",
                            "Alias 치환 후에도 ##...## 표현이 남았습니다. Alias는 재귀 치환하지 않습니다.",
                        );
                    }
                    if (col.has("Raw") || aliases(&resolved.display()).is_empty())
                        && let Err(error) = check_value(&typ, &resolved, c, !col.has("Raw"))
                    {
                        r.push(sh, row, col.index, "error", "value.type", error);
                    }
                    let flags = schema::flags(&col.flags);
                    for table in
                        flags.iter().filter_map(|f| f.split_once('.')).filter(|(p, _)| p.eq_ignore_ascii_case("FKey")).map(|(_, table)| table)
                    {
                        let resolved_text = resolved.display();
                        let fields = if typ.list {
                            schema::split_values(&resolved_text, typ.record())
                        } else if typ.nullable() && (canonical.is_empty() || canonical == "null") {
                            vec![]
                        } else {
                            vec![resolved_text.as_str()]
                        };
                        let mut scalar_col = col.clone();
                        let mut scalar_type = typ.clone();
                        scalar_type.list = false;
                        scalar_type.matrix = false;
                        scalar_col.typ = scalar_type.spelling();
                        match c.keys.get(table) {
                            Some(keys) => {
                                for key in fields {
                                    if !keys.contains(&self::canonical(key, &scalar_col, c)) {
                                        r.push(
                                            sh,
                                            row,
                                            col.index,
                                            if complete { "error" } else { "warning" },
                                            "fkey.missing",
                                            format!("{table}의 Key에 없는 값: {key} (선택한 버전 필터 적용 후)"),
                                        );
                                    }
                                }
                            }
                            None => r.push(
                                sh,
                                0,
                                col.index,
                                if complete { "error" } else { "warning" },
                                "fkey.table",
                                format!("Key가 있는 참조 테이블을 찾을 수 없습니다: {table}"),
                            ),
                        }
                    }
                    if let Some(other) = col.argument("ZipWith") {
                        if let Some(j) = sh.field(&other) {
                            let other_type = sh.columns.get(j).and_then(|col| Type::parse(&col.typ).ok());
                            let other_value = values.get(j).cloned().unwrap_or_default();
                            let length = element_count(&typ, &resolved);
                            if other_type.as_ref().is_some_and(|t| element_count(t, &resolve(&other_value, c)) != length) {
                                r.push(sh, row, col.index, "error", "zip.length", format!("{other}와 원소 수가 다릅니다."));
                            }
                        } else {
                            r.push(sh, 0, col.index, "error", "zip.field", format!("ZipWith 참조 필드가 없습니다: {other}"));
                        }
                    }
                }
            }
        }
    }
}
pub fn analyze(sheets: &[&DataSheet], target: Option<&str>, complete: bool) -> (Report, Catalog) {
    let c = Arc::new(catalog(sheets, target));
    let mut validator = Validator::new(c.clone(), target.map(str::to_string), complete);
    for sh in sheets {
        validator.push(sh);
    }
    let report = std::mem::take(&mut validator.report);
    drop(validator);
    (report, Arc::unwrap_or_clone(c))
}

fn check_schema(sh: &DataSheet, c: &Catalog, complete: bool, r: &mut Report) {
    let mut names = BTreeSet::new();
    for col in sh.columns.iter().filter(|col| col.included()) {
        for error in schema::flag_errors(&col.flags) {
            r.push(sh, 0, col.index, "error", "schema.flags", error);
        }
        if !schema::valid_field(&col.name) {
            r.push(sh, 2, col.index, "error", "schema.name", "필드 이름은 영문자로 시작하고 영문·숫자·밑줄만 사용할 수 있습니다.");
        }
        if !names.insert(&col.name) {
            r.push(sh, 2, col.index, "error", "schema.duplicate", "필드 이름이 중복됩니다.");
        }
        if col.has("Design") {
            continue;
        }
        match Type::parse(&col.typ) {
            Err(error) => r.push(sh, 1, col.index, "error", "schema.type", error),
            Ok(typ) => {
                // The CK exception is specifically the direct TString variant, not NullableTString
                // or List<TString>. Do not silently rewrite authored type headers.
                if typ.base == "TString" && !typ.list {
                    r.push(
                        sh,
                        1,
                        col.index,
                        "error",
                        "ck.tstring",
                        "CK 프로젝트는 일반 TString을 허용하지 않습니다. OptimizedTString 또는 필요한 nullable 타입을 검토하세요.",
                    );
                }
                if typ.base == "Enum" && typ.argument.as_ref().is_some_and(|name| !c.enums.contains_key(name)) {
                    r.push(sh, 1, col.index, if complete { "error" } else { "warning" }, "enum.type", "EnumList에서 Enum 타입을 찾을 수 없습니다.");
                }
                if typ.base.contains("TString") && !typ.list {
                    for segment in typ.argument.as_deref().unwrap_or_default().split('.') {
                        if let Some(field) = segment.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                            && sh.field(field).is_none()
                        {
                            r.push(sh, 1, col.index, "error", "tstring.field", format!("번역 키 패턴의 참조 필드가 없습니다: {field}"));
                        }
                    }
                }
            }
        }
    }
    let count = |flag| sh.columns.iter().filter(|col| col.has(flag)).count();
    for flag in ["Key", "GroupKey", "SplitBinaryGroupKey"] {
        if count(flag) > 1 {
            r.push(sh, 0, 0, "error", "schema.cardinality", format!("{flag} 컬럼은 하나만 사용할 수 있습니다."));
        }
    }
    if count("Key") > 0 && count("CompoundKey") > 0 {
        r.push(sh, 0, 0, "error", "schema.compound", "Key와 CompoundKey는 함께 사용할 수 없습니다.");
    }
    if count("CompoundKey") == 1 {
        r.push(sh, 0, 0, "error", "schema.compound", "CompoundKey는 2개 이상의 컬럼 조합이어야 합니다.");
    }
    let pair = (count("SplitBinaryName"), count("SplitBinaryKey"));
    if pair != (0, 0) && pair != (1, 1) {
        r.push(sh, 0, 0, "error", "schema.split", "SplitBinaryName과 SplitBinaryKey는 각각 하나씩 함께 필요합니다.");
    }
    if count("SplitBinaryGroupKey") > 0 && count("SplitBinaryKey") == 0 {
        r.push(sh, 0, 0, "error", "schema.split", "SplitBinaryGroupKey에는 SplitBinaryKey가 필요합니다.");
    }
}

pub fn value_error(typ: &Type, value: &Value, c: &Catalog) -> Result<(), String> {
    check_value(typ, value, c, true)
}

fn check_value(typ: &Type, value: &Value, c: &Catalog, symbolic_ids: bool) -> Result<(), String> {
    let text = value.display();
    let raw = text.trim();
    if matches!(value, Value::Error(_) | Value::Array(_)) {
        return Err("수식 오류 또는 배열 값을 패처 데이터로 사용할 수 없습니다.".into());
    }
    if typ.list {
        let elements = schema::split_values(raw, typ.record());
        let mut element_type = typ.clone();
        element_type.list = typ.matrix;
        element_type.matrix = false;
        for element in elements {
            check_value(&element_type, &Value::Text(element.into()), c, symbolic_ids)?;
        }
        return Ok(());
    }
    if typ.nullable() && (raw.is_empty() || raw.eq_ignore_ascii_case("null")) {
        return Ok(());
    }
    let base = typ.scalar();
    let numeric = raw.replace('_', "");
    let number = if numeric.is_empty() { "0" } else { &numeric };
    let error = || format!("{} 형식에 맞지 않는 값: {raw}", typ.spelling());
    match base {
        "Int" | "Long" => {
            // Bracketed IDs are resolved by the patcher's ID map. Preserve them; global and FKey
            // checks separately verify definitions. Never round a typed 64-bit integer through f64.
            if symbolic_ids && raw.starts_with('[') && raw.ends_with(']') && string_ids(raw).len() == 1 {
                return Ok(());
            }
            if let Value::Number(v) = value {
                let v = *v;
                let rounded = v.round();
                let bound = if base == "Int" { (i32::MIN as f64, i32::MAX as f64 + 1.0) } else { (i64::MIN as f64, -(i64::MIN as f64)) };
                return if (v - rounded).abs() < 1e-5 && rounded >= bound.0 && rounded < bound.1 { Ok(()) } else { Err(error()) };
            }
            let resolved = enum_number(number, c).unwrap_or(number);
            let integer = resolved.parse::<i64>().map_err(|_| error())?;
            if base == "Int" && i32::try_from(integer).is_err() {
                return Err(error());
            }
        }
        "Float" | "Double" => {
            if !number.parse::<f64>().is_ok_and(f64::is_finite) {
                return Err(error());
            }
            if base == "Float" && !number.parse::<f32>().is_ok_and(f32::is_finite) {
                return Err(error());
            }
        }
        "Bool" => {
            if matches!(value, Value::Bool(_) | Value::Number(_)) {
                return Ok(());
            }
            if !["", "true", "false", "t", "f", "yes", "no", "1", "0", "1.0", "0.0"].iter().any(|s| raw.eq_ignore_ascii_case(s)) {
                return Err(error());
            }
        }
        "Json" => {
            serde_json::from_str::<serde_json::Value>(raw).map_err(|e| format!("JSON 오류: {e}"))?;
        }
        "Enum" => {
            if raw.is_empty() || raw == "None" {
                return Ok(());
            }
            if let Some(values) = typ.argument.as_ref().and_then(|name| c.enums.get(name))
                && !values.contains_key(raw)
            {
                return Err(error());
            }
        }
        "Percentile" | "BasisPoint" => {
            let mut v = if raw.is_empty() { 0.0 } else { raw.parse::<f64>().map_err(|_| error())? };
            if matches!(value, Value::Number(_)) {
                v = (v * 1e12).round() / 1e12;
            }
            let scale = if base == "Percentile" { 100.0 } else { 10_000.0 };
            if !v.is_finite() || (v - (v * scale).round() / scale).abs() > f64::EPSILON || v * scale >= i64::MAX as f64 || v * scale < i64::MIN as f64
            {
                return Err(error());
            }
        }
        b if b.starts_with("Vector") || b.starts_with("Grid") => {
            let length = if b.starts_with("Vector2") { 2 } else { 3 };
            let fields = schema::split_values(raw, false);
            if fields.len() != length {
                return Err(format!("{b}에는 {length}개 좌표가 필요합니다."));
            }
            for field in fields {
                let valid = if b.ends_with("Int") {
                    field.parse::<i32>().is_ok()
                } else if b.ends_with("Double") {
                    field.parse::<f64>().is_ok_and(f64::is_finite)
                } else {
                    field.parse::<f32>().is_ok_and(f32::is_finite)
                };
                if !valid {
                    return Err(error());
                }
            }
        }
        // Date/time and duration accept the patcher's Korean/English natural units and ZoneOffset.
        // Their conversion is deliberately deferred rather than rejecting valid authored formats.
        _ => {}
    }
    Ok(())
}

pub fn element_count(typ: &Type, value: &Value) -> usize {
    if typ.list {
        schema::split_values(&value.display(), typ.record()).len()
    } else if typ.nullable() && (value.display().trim().is_empty() || value.display().trim().eq_ignore_ascii_case("null")) {
        0
    } else {
        1
    }
}

pub fn string_ids(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('[') {
        let Some((id, tail)) = after.split_once(']') else { break };
        if !id.contains('[') {
            result.push(format!("[{}]", id.chars().filter(|c| !c.is_whitespace()).collect::<String>()));
        }
        rest = tail;
    }
    result
}

pub fn aliases(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("##") {
        let Some((name, tail)) = after.split_once("##") else { break };
        result.push(format!("##{name}##"));
        rest = tail;
    }
    result
}

pub fn resolve(value: &Value, c: &Catalog) -> Value {
    let Value::Text(text) = value else { return value.clone() };
    let mut resolved = String::new();
    let mut rest: &str = text;
    while let Some((prefix, after)) = rest.split_once("##") {
        let Some((name, tail)) = after.split_once("##") else { break };
        let token = format!("##{name}##");
        let replacement = c.aliases.get(&token).map(String::as_str).unwrap_or(&token);
        if resolved.len().saturating_add(prefix.len()).saturating_add(replacement.len()).saturating_add(tail.len()) > 1_048_576 {
            return value.clone();
        }
        resolved.push_str(prefix);
        resolved.push_str(replacement);
        rest = tail;
    }
    resolved.push_str(rest);
    Value::Text(resolved.into())
}

fn enum_number<'a>(text: &str, c: &'a Catalog) -> Option<&'a str> {
    let (typ, name) = text.split_once('.')?;
    c.enums.get(typ)?.get(name).map(String::as_str)
}

pub fn canonical(raw: &str, col: &Column, c: &Catalog) -> String {
    let raw = if col.has("Raw") { raw.to_string() } else { resolve(&Value::Text(raw.into()), c).display() };
    let mut output = raw.clone();
    if !col.has("Raw") {
        let mut rest = raw.as_str();
        while let Some((_, after)) = rest.split_once('[') {
            let Some((id, tail)) = after.split_once(']') else { break };
            output = output.replace(&format!("[{id}]"), &format!("[{}]", id.chars().filter(|c| !c.is_whitespace()).collect::<String>()));
            rest = tail;
        }
    }
    if let Ok(typ) = Type::parse(&col.typ) {
        if !typ.list && typ.scalar() == "String" {
            if let Some(inner) = output.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return inner.replace("\"\"", "\"");
            }
            return output;
        }
        output = output.trim().to_string();
        if typ.list {
            let mut element = typ.clone();
            element.list = typ.matrix;
            element.matrix = false;
            let mut element_column = col.clone();
            element_column.typ = element.spelling();
            return schema::split_values(&output, typ.record()).into_iter().map(|s| canonical(s, &element_column, c)).collect::<Vec<_>>().join("\n");
        }
        if typ.scalar() == "Bool" {
            if ["", "false", "f", "no", "0", "0.0"].iter().any(|s| output.eq_ignore_ascii_case(s)) {
                return "false".into();
            }
            if ["true", "t", "yes", "1", "1.0"].iter().any(|s| output.eq_ignore_ascii_case(s)) {
                return "true".into();
            }
        }
        if typ.scalar() == "Enum" {
            if output.is_empty() || output == "None" {
                return "0".into();
            }
            if let Some(value) = typ.argument.as_ref().and_then(|n| c.enums.get(n)).and_then(|e| e.get(&output)) {
                return value.clone();
            }
        }
    }
    if let Ok(typ) = Type::parse(&col.typ)
        && matches!(typ.scalar(), "Int" | "Long" | "Float" | "Double")
        && !typ.list
    {
        if typ.nullable() && (output.is_empty() || output.eq_ignore_ascii_case("null")) {
            return "null".into();
        }
        let number = output.replace('_', "");
        if number.is_empty() {
            return "0".into();
        }
        if let Some(n) = enum_number(&number, c) {
            return n.into();
        }
        if let Ok(n) = number.parse::<i64>() {
            return n.to_string();
        }
        if let Ok(n) = number.parse::<f64>() {
            return n.to_string();
        }
    }
    output
}
