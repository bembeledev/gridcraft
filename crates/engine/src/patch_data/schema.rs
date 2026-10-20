//! CK patch-data header grammar, implemented from the patcher's observable format contract.
//! This module never changes a workbook or guesses a type for an unrecognized header.

use serde::{Deserialize, Serialize};

pub const FLAGS: &[&str] = &[
    "All",
    "Server",
    "Design",
    "Version",
    "Key",
    "CompoundKey",
    "GroupKey",
    "Unique",
    "NotBlank",
    "Raw",
    "StringIdDefinition",
    "StringIdReference",
    "SplitBinaryName",
    "SplitBinaryKey",
    "SplitBinaryGroupKey",
    "OmitTStringOriginalTextForProd",
];
pub const TYPES: &[&str] = &[
    "Int",
    "Long",
    "Float",
    "Double",
    "Bool",
    "String",
    "DateTime",
    "Duration",
    "Json",
    "Percentile",
    "BasisPoint",
    "Vector2",
    "Vector2Int",
    "Vector3",
    "Vector3Int",
    "Grid3",
    "Grid3Double",
    "Grid3Int",
    "NullableInt",
    "NullableLong",
    "NullableFloat",
    "NullableDouble",
    "NullableDateTime",
    "NullableDuration",
    "NullableString",
];
const INNER_LISTS: &[&str] = &[
    "IntList",
    "LongList",
    "NullableIntList",
    "NullableLongList",
    "NullableFloatList",
    "NullableDoubleList",
    "StringList",
    "NullableStringList",
    "DateTimeList",
    "NullableDateTimeList",
    "DurationList",
    "NullableDurationList",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Type {
    pub base: String,
    pub argument: Option<String>,
    pub list: bool,
    pub matrix: bool,
}

impl Type {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() > 4096 {
            return Err("타입 이름이 너무 깁니다.".into());
        }
        let raw = raw.trim();
        let (inner, list) = match generic(raw, "List") {
            Some(inner) => (inner.trim(), true),
            None => (raw, false),
        };
        if generic(inner, "List").is_some() {
            return Err("List<List<T>>는 사용할 수 없습니다. 2차원 목록은 List<IntList> 같은 타입을 사용하세요.".into());
        }
        if let Some(base) = TYPES.iter().find(|t| t.eq_ignore_ascii_case(inner)) {
            return Ok(Self { base: (*base).into(), argument: None, list, matrix: false });
        }
        if let Some(base) = INNER_LISTS.iter().find(|t| t.eq_ignore_ascii_case(inner)) {
            if !list {
                return Err("IntList 등의 타입은 List<IntList>처럼 List 안에서만 사용합니다.".into());
            }
            return Ok(Self { base: base.trim_end_matches("List").into(), argument: None, list, matrix: true });
        }
        for base in ["Enum", "TString", "OptimizedTString", "NullableTString"] {
            if let Some(argument) = generic(inner, base) {
                if argument.is_empty() {
                    return Err(format!("{base}의 <> 안에 이름 또는 번역 키 패턴이 필요합니다."));
                }
                return Ok(Self { base: base.into(), argument: Some(argument.into()), list, matrix: false });
            }
        }
        Err(format!("지원하지 않는 패처 타입: {raw}"))
    }

    pub fn spelling(&self) -> String {
        let mut inner = match &self.argument {
            Some(a) => format!("{}<{a}>", self.base),
            None => self.base.clone(),
        };
        if self.matrix {
            inner.push_str("List");
        }
        if self.list { format!("List<{inner}>") } else { inner }
    }

    pub fn nullable(&self) -> bool {
        self.base.starts_with("Nullable")
    }
    pub fn scalar(&self) -> &str {
        self.base.strip_prefix("Nullable").unwrap_or(&self.base)
    }
    pub fn record(&self) -> bool {
        self.matrix || self.base.starts_with("Vector") || self.base.starts_with("Grid")
    }
    pub fn hint(&self) -> &'static str {
        if self.matrix {
            return "줄바꿈으로 행을, 쉼표로 각 행의 원소를 구분합니다. 빈 줄은 빈 목록입니다.";
        }
        if self.list && self.record() {
            return "줄바꿈으로 벡터/격자를 구분하고, 각 레코드 안에서는 쉼표로 좌표를 구분합니다.";
        }
        if self.list {
            return "쉼표 또는 줄바꿈으로 구분합니다. 따옴표 안의 구분자는 값의 일부입니다.";
        }
        match self.scalar() {
            "Int" => "32비트 정수. 빈 값은 0. 숫자 구분자 _, [문자열ID], EnumType.Name을 사용할 수 있습니다.",
            "Long" => "64비트 정수. 큰 정수는 정밀도를 보존하도록 텍스트로 입력합니다.",
            "Float" | "Double" => "실수. 빈 값은 0. 숫자 구분자 _를 사용할 수 있습니다.",
            "Bool" => "TRUE/FALSE, t/f, yes/no, 1/0. 빈 값은 false. 숫자 셀은 0이 아니면 true입니다.",
            "DateTime" => "날짜와 시간을 입력합니다. 날짜만 입력하면 자정이며 ZoneOffset 설정을 따릅니다.",
            "Duration" => "예: 3w 4d 5h 6m 7s, 1.2일. 빈 값은 0초입니다.",
            "Json" => "유효한 JSON 값. 패처가 변환할 때 minify합니다.",
            "Percentile" => "소수점 둘째 자리까지의 값. 패처에서 100배한 정수로 저장합니다.",
            "BasisPoint" => "소수점 넷째 자리까지의 값. 패처에서 10,000배한 정수로 저장합니다.",
            "Enum" => "EnumList의 후보를 사용합니다. 빈 값 또는 None은 0입니다.",
            "TString" | "OptimizedTString" => "번역 키 패턴의 [필드명]은 같은 행의 값으로 치환됩니다. OptimizedTString은 빈 원문을 생략합니다.",
            b if b.starts_with("Vector") || b.starts_with("Grid") => "좌표를 쉼표 또는 줄바꿈으로 구분합니다. 타입에 맞는 개수가 필요합니다.",
            _ => "Raw가 아니면 [문자열ID]와 ##Alias##를 패처가 변환합니다. 원본 표현은 그대로 보존합니다.",
        }
    }
}

fn generic<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let (prefix, rest) = raw.split_once('<')?;
    if !prefix.trim().eq_ignore_ascii_case(name) {
        return None;
    }
    rest.strip_suffix('>')
}

pub fn flags(raw: &str) -> Vec<String> {
    raw.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}
pub fn has(flags: &[String], name: &str) -> bool {
    flags.iter().any(|f| f.eq_ignore_ascii_case(name))
}
pub fn argument<'a>(flags: &'a [String], name: &str) -> Option<&'a str> {
    flags.iter().filter_map(|f| f.split_once('.')).find(|(p, _)| p.eq_ignore_ascii_case(name)).map(|(_, a)| a)
}

pub fn flag_errors(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return vec![];
    }
    let f = flags(raw);
    let mut errors = Vec::new();
    for flag in &f {
        let known = FLAGS.iter().any(|name| name.eq_ignore_ascii_case(flag))
            || flag.split_once('.').is_some_and(|(p, _)| ["FKey", "ZipWith", "Client"].iter().any(|name| name.eq_ignore_ascii_case(p)));
        if !known {
            errors.push(format!("지원하지 않는 플래그: {flag}"));
        }
    }
    let targets = ["Version", "All", "Server", "Design"].iter().filter(|name| has(&f, name)).count();
    if !has(&f, "SplitBinaryName") && targets != 1 {
        errors.push("All, Server, Design, Version 중 정확히 하나가 필요합니다.".into());
    }
    if has(&f, "Version") && f.len() != 1 {
        errors.push("Version은 단독으로 사용해야 합니다.".into());
    }
    let defines = has(&f, "StringIdDefinition");
    let references = has(&f, "StringIdReference");
    if (defines || references) && (has(&f, "Design") || has(&f, "Raw")) {
        errors.push("문자열 ID 역할에는 Design 또는 Raw를 함께 사용할 수 없습니다.".into());
    }
    if defines && (references || argument(&f, "FKey").is_some()) {
        errors.push("StringIdDefinition은 StringIdReference 또는 FKey와 함께 사용할 수 없습니다.".into());
    }
    errors
}

pub fn valid_field(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
pub fn valid_sheet(name: &str) -> bool {
    let table = name.split('_').next().unwrap_or_default();
    table.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && table.chars().all(|c| c.is_ascii_alphanumeric())
}
pub fn eligible_file(name: &str) -> bool {
    !name.starts_with(['_', '~']) && name.ends_with("Data.xlsx")
}

/// The patcher keeps quotes in field text and splits only outside them. Newlines split record
/// lists; commas additionally split ordinary lists. Empty fields are kept (empty numeric = 0).
pub fn split_values(text: &str, records: bool) -> Vec<&str> {
    let mut values = Vec::new();
    let mut suffix_odd = text.bytes().filter(|b| *b == b'"').count() % 2 != 0;
    let mut start = 0;
    let mut skip = 0;
    for (index, ch) in text.char_indices() {
        if index < skip {
            continue;
        }
        if ch == '"' {
            suffix_odd = !suffix_odd;
        }
        if !suffix_odd && (ch == '\n' || (!records && ch == ',')) {
            values.push(text.get(start..index).unwrap_or_default().trim());
            start = index.saturating_add(1);
            if ch == ',' {
                let tail = text.get(start..).unwrap_or_default();
                let trimmed = tail.trim_start_matches([' ', '\t']);
                start = start.saturating_add(tail.len() - trimmed.len());
                if trimmed.starts_with('\n') {
                    start = start.saturating_add(1);
                }
            }
            skip = start;
        }
    }
    values.push(text.get(start..).unwrap_or_default().trim());
    if values.len() == 1 && values.first().is_some_and(|s| s.is_empty()) {
        values.clear();
    }
    values
}
