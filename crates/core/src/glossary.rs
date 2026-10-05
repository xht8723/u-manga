//! Book-owned terminology and strict, lossless two-column interchange.
use crate::types::GlossaryEntry;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookGlossary {
    pub enabled: bool,
    pub auto_detect: bool,
    pub automatic_sources: Vec<String>,
    pub entries: Vec<GlossaryEntry>,
    pub deepl_glossary_id: String,
    pub revision: u64,
}
/// Raw, trimmed source/target bytes; retained for current-format data compatibility.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;
// Worst-case doubled quotes + two quoted columns, delimiter and CRLF per row;
// the UTF-8 BOM and source/target header add 18 bytes.
pub const MAX_ENCODED_FILE_BYTES: usize = 2 * MAX_FILE_BYTES + 7 * MAX_ENTRIES + 18;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionCheckpoint {
    pub completed: bool,
    pub processed: Vec<String>,
    /// None until the source-reading stage captures its unfinished regions.
    pub eligible: Option<Vec<String>>,
    pub added: usize,
    pub skipped: usize,
    #[serde(with = "crate::ui_message::optional_error")]
    pub warning: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub region_id: String,
    pub source: String,
    pub target: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Extraction {
    pub terms: Vec<Candidate>,
    pub skipped: usize,
}
/// Bad optional candidates cannot discard useful candidates or translation work.
pub fn candidates(text: &str, regions: &[crate::types::Region]) -> Result<Extraction> {
    ensure!(
        text.len() <= MAX_FILE_BYTES,
        "Glossary response exceeds the size limit"
    );
    let text = text.trim();
    if text.eq_ignore_ascii_case("NONE") {
        return Ok(Extraction::default());
    }
    let mut result = Extraction::default();
    let mut seen = HashSet::new();
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if line.starts_with("```") {
            continue;
        }
        let candidate = pair(line).and_then(|(source, target)| {
            regions
                .iter()
                .find(|r| r.source.contains(&source))
                .map(|r| Candidate {
                    region_id: r.id.clone(),
                    source,
                    target,
                })
        });
        let Some(candidate) = candidate.filter(|c| valid_candidate(c, regions)) else {
            result.skipped += 1;
            continue;
        };
        if seen.insert(candidate.source.clone()) && result.terms.len() < 16 {
            result.terms.push(candidate);
        }
    }
    ensure!(
        !result.terms.is_empty(),
        "No usable glossary pairs in response"
    );
    Ok(result)
}
/// Labels only delimit rows. Colon separators must be outside JSON-quoted fields.
fn pair(line: &str) -> Option<(String, String)> {
    let line = if let Some(label) = line.strip_prefix('[') {
        let end = label.find(']')?;
        let number = &label[..end];
        if number.is_empty() || !number.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        label[end + 1..].trim_start()
    } else {
        line
    };
    let (mut quoted, mut escaped, mut separator) = (false, false, None);
    for (index, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted && matches!(c, ':' | '：') {
            if separator.is_some() {
                return None;
            }
            separator = Some((index, c.len_utf8()));
        }
    }
    if quoted || escaped {
        return None;
    }
    let (index, bytes) = separator?;
    Some((field(&line[..index])?, field(&line[index + bytes..])?))
}
fn field(text: &str) -> Option<String> {
    let text = text.trim();
    let value: String = if text.starts_with('"') {
        serde_json::from_str(text).ok()?
    } else {
        if text.contains('"') || text.chars().any(char::is_control) {
            return None;
        }
        text.to_owned()
    };
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}
fn valid_candidate(e: &Candidate, regions: &[crate::types::Region]) -> bool {
    !e.source.is_empty()
        && !e.target.is_empty()
        && e.source.chars().count() <= 256
        && e.target.chars().count() <= 512
        && !e.source.contains('\0')
        && !e.target.contains('\0')
        && regions
            .iter()
            .any(|r| r.id == e.region_id && r.source.contains(&e.source))
}
/// The same evidence/limits apply to transport, cache, and durable checkpoint publication.
pub fn validate_candidates(
    rows: impl IntoIterator<Item = Candidate>,
    regions: &[crate::types::Region],
) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    rows.into_iter()
        .take(16)
        .filter_map(|mut e| {
            e.source = e.source.trim().into();
            e.target = e.target.trim().into();
            if !valid_candidate(&e, regions) || !seen.insert(e.source.clone()) {
                return None;
            }
            Some(e)
        })
        .collect()
}
/// Invalid rows remain on disk but must not authorize extraction or suppress a new request.
pub fn cached_candidates(text: &str, regions: &[crate::types::Region]) -> Option<Extraction> {
    if text.len() > MAX_FILE_BYTES {
        return None;
    }
    let rows: Extraction = serde_json::from_str(text).ok()?;
    if rows.terms.len() > 16 || rows.skipped > MAX_FILE_BYTES {
        return None;
    }
    let valid = validate_candidates(rows.terms.clone(), regions);
    (valid == rows.terms).then_some(rows)
}
pub fn validate_entries(entries: &mut [GlossaryEntry], unique: bool) -> Result<()> {
    ensure!(
        entries.len() <= MAX_ENTRIES,
        "Glossary is limited to 10,000 entries."
    );
    let mut sources = HashSet::new();
    let mut bytes = 0;
    for e in entries {
        e.source = e.source.trim().into();
        e.target = e.target.trim().into();
        ensure!(
            !e.source.is_empty() && !e.target.is_empty(),
            "Enter both a source term and a translation."
        );
        ensure!(
            !e.source.contains('\0') && !e.target.contains('\0'),
            "Glossary terms cannot contain null characters."
        );
        bytes += e.source.len() + e.target.len();
        ensure!(bytes <= MAX_FILE_BYTES, "Glossary exceeds the 4 MiB limit.");
        ensure!(
            !unique || sources.insert(e.source.clone()),
            "Each source term must appear only once."
        );
    }
    Ok(())
}
pub fn validate(value: &mut BookGlossary) -> Result<()> {
    validate_entries(&mut value.entries, true)?;
    let sources: HashSet<_> = value.entries.iter().map(|e| e.source.as_str()).collect();
    value
        .automatic_sources
        .retain(|s| sources.contains(s.as_str()));
    value.automatic_sources.sort();
    value.automatic_sources.dedup();
    value.deepl_glossary_id = value.deepl_glossary_id.trim().into();
    ensure!(
        value.deepl_glossary_id.len() <= 1024
            && !value.deepl_glossary_id.chars().any(char::is_control),
        "Invalid hosted glossary ID."
    );
    Ok(())
}
fn delimiter(format: &str) -> Result<char> {
    match format {
        "csv" => Ok(','),
        "tsv" => Ok('\t'),
        _ => anyhow::bail!("Choose a CSV or TSV file."),
    }
}
/// Quoting follows CSV conventions for both comma and tab delimiters.
pub fn parse(text: &str, format: &str) -> Result<Vec<GlossaryEntry>> {
    ensure!(
        text.len() <= MAX_ENCODED_FILE_BYTES,
        "Glossary file exceeds the encoded size limit."
    );
    let delimiter = delimiter(format)?;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let (mut entries, mut row, mut field) = (vec![], vec![], String::new());
    let (mut quoted, mut closed, mut first_row) = (false, false, true);
    let mut bytes = 0;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        if c == delimiter {
            ensure!(
                row.is_empty(),
                "Glossary files must contain exactly two columns: source and target."
            );
            row.push(std::mem::take(&mut field));
            closed = false;
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            closed = false;
            if row.len() != 1 || !row[0].trim().is_empty() {
                parsed_row(
                    std::mem::take(&mut row),
                    &mut entries,
                    &mut first_row,
                    &mut bytes,
                )?;
            } else {
                row.clear();
            }
        } else if c == '"' && field.is_empty() && !closed {
            quoted = true;
        } else {
            ensure!(
                !closed && c != '"',
                "Malformed glossary file: invalid quoting."
            );
            field.push(c);
        }
    }
    ensure!(!quoted, "Malformed glossary file: unclosed quoted field.");
    if !field.is_empty() || !row.is_empty() || closed {
        row.push(field);
        parsed_row(row, &mut entries, &mut first_row, &mut bytes)?;
    }
    Ok(entries)
}

/// Validate each completed row before retaining another, including duplicate import terms.
fn parsed_row(
    row: Vec<String>,
    entries: &mut Vec<GlossaryEntry>,
    first: &mut bool,
    bytes: &mut usize,
) -> Result<()> {
    ensure!(
        row.len() == 2,
        "Glossary files must contain exactly two columns: source and target."
    );
    let header = *first
        && row[0].trim().eq_ignore_ascii_case("source")
        && row[1].trim().eq_ignore_ascii_case("target");
    *first = false;
    if header {
        return Ok(());
    }
    ensure!(
        entries.len() < MAX_ENTRIES,
        "Glossary is limited to 10,000 entries."
    );
    let mut fields = row.into_iter();
    let mut entry = GlossaryEntry {
        source: fields.next().unwrap(),
        target: fields.next().unwrap(),
    };
    validate_entries(std::slice::from_mut(&mut entry), false)?;
    *bytes += entry.source.len() + entry.target.len();
    ensure!(
        *bytes <= MAX_FILE_BYTES,
        "Glossary exceeds the 4 MiB limit."
    );
    entries.push(entry);
    Ok(())
}
pub fn encode(entries: &[GlossaryEntry], format: &str) -> Result<String> {
    let delimiter = delimiter(format)?;
    let mut entries = entries.to_vec();
    validate_entries(&mut entries, true)?;
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut text = format!("\u{feff}source{delimiter}target\r\n");
    for e in entries {
        text.push_str(&format!(
            "{}{delimiter}{}\r\n",
            quote(&e.source),
            quote(&e.target)
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_quotes_and_multiline_round_trip() {
        let rows = vec![GlossaryEntry {
            source: "魔王,\"A\"\n城\t門".into(),
            target: "魔王、城门".into(),
        }];
        for format in ["csv", "tsv"] {
            assert_eq!(
                parse(&encode(&rows, format).unwrap(), format).unwrap(),
                rows
            );
        }
        assert_eq!(
            parse(" source , target \r\n 勇者 , 勇者 ", "csv").unwrap()[0].source,
            "勇者"
        );
        assert_eq!(parse("猫\t貓", "tsv").unwrap().len(), 1);
    }

    #[test]
    fn accepted_content_round_trips_with_encoded_quote_and_row_overhead() {
        let plain = (0..MAX_ENTRIES)
            .map(|i| GlossaryEntry {
                source: format!("k{i:05}{}", "s".repeat(203)),
                target: "t".repeat(209),
            })
            .collect::<Vec<_>>();
        let quotes = vec![GlossaryEntry {
            source: "A".into(),
            target: "\"".repeat(MAX_FILE_BYTES - 1),
        }];
        let mixed = vec![GlossaryEntry {
            source: "a,\"日\"\r\n\tb".into(),
            target: "  x,\"語\"\r\n\tz  ".into(),
        }];
        for mut rows in [plain, quotes, mixed] {
            validate_entries(&mut rows, true).unwrap();
            for format in ["csv", "tsv"] {
                let encoded = encode(&rows, format).unwrap();
                assert!(encoded.len() <= MAX_ENCODED_FILE_BYTES);
                assert_eq!(parse(&encoded, format).unwrap(), rows);
            }
        }
    }

    #[test]
    fn streaming_import_keeps_raw_and_row_limits_and_duplicate_semantics() {
        let text = "\u{feff} source , target \r\n a , b \r\n a , c \r\n";
        let rows = parse(text, "csv").unwrap();
        assert_eq!(
            rows,
            vec![
                GlossaryEntry {
                    source: "a".into(),
                    target: "b".into()
                },
                GlossaryEntry {
                    source: "a".into(),
                    target: "c".into()
                },
            ]
        );
        let many = "a,b\n".repeat(MAX_ENTRIES + 1);
        assert!(
            parse(&many, "csv")
                .unwrap_err()
                .to_string()
                .contains("10,000")
        );
        let excess = format!("a,{}", "b".repeat(MAX_FILE_BYTES));
        assert!(
            parse(&excess, "csv")
                .unwrap_err()
                .to_string()
                .contains("4 MiB")
        );
        let repeated = format!(
            "a,{}\na,{}\n",
            "b".repeat(MAX_FILE_BYTES / 2),
            "c".repeat(MAX_FILE_BYTES / 2)
        );
        assert!(
            parse(&repeated, "csv")
                .unwrap_err()
                .to_string()
                .contains("4 MiB")
        );
        assert!(
            parse(&" ".repeat(MAX_ENCODED_FILE_BYTES + 1), "csv")
                .unwrap_err()
                .to_string()
                .contains("encoded")
        );
        assert!(
            parse(&",".repeat(MAX_ENCODED_FILE_BYTES), "csv")
                .unwrap_err()
                .to_string()
                .contains("two columns")
        );
    }

    #[test]
    fn invalid_files_and_duplicate_terms() {
        for text in ["a,b,c", "\"a,b", "\"a\"x,b", "a,", "a\"x,b"] {
            assert!(parse(text, "csv").is_err(), "{text}");
        }
        let mut rows = parse("a,b\na,c", "csv").unwrap();
        assert!(validate_entries(&mut rows, true).is_err());
        assert!(encode(&rows, "csv").is_err());
        assert!(parse("a,b", "exe").is_err());
        assert!(parse(&"a".repeat(MAX_FILE_BYTES + 1), "csv").is_err());
    }
}
