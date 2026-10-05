//! Whole-page text requests with bounded framing and application-owned identities.
use crate::{prompts, safety, types::*};
use anyhow::{Result, ensure};

pub const MAX_REGIONS: usize = 48;
pub const MAX_CURRENT_BYTES: usize = 6 * 1024;
pub const STRUCTURED_CACHE_VERSION: u32 = 1;
const INVALID_REPLY: &str = "Simple translation returned an incomplete or malformed numbered batch. Earlier progress was kept; retry the page.";

/// Only the current-text section is budgeted. References and guidance are never truncated.
pub fn current_text(regions: &[Region], simple: bool) -> Result<String> {
    if simple && regions.len() == 1 {
        return Ok(regions[0].source.clone());
    }
    regions
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if simple {
                // Normalize wire line endings so source labels cannot escape indentation.
                // The stored source and returned TranslationItem source remain untouched.
                let source = r.source.replace("\r\n", "\n").replace('\r', "\n");
                Ok(format!("[{}] {}", i + 1, source.replace('\n', "\n    ")))
            } else {
                prompts::region_label(r)
            }
        })
        .collect::<Result<Vec<_>>>()
        .map(|items| items.join(if simple { "\n" } else { "\n\n" }))
}

/// A single oversized region stays whole and uses the existing single-region path.
pub fn plan(regions: &[Region], simple: bool) -> Result<Vec<&[Region]>> {
    let mut batches = Vec::new();
    let mut start = 0;
    while start < regions.len() {
        let mut end = start + 1;
        while end < regions.len()
            && end - start < MAX_REGIONS
            && current_text(&regions[start..=end], simple)?.len() <= MAX_CURRENT_BYTES
        {
            end += 1;
        }
        batches.push(&regions[start..end]);
        start = end;
    }
    Ok(batches)
}

pub fn validate_request(regions: &[Region], simple: bool) -> Result<()> {
    ensure!(
        regions.len() <= MAX_REGIONS
            && (regions.len() <= 1 || current_text(regions, simple)?.len() <= MAX_CURRENT_BYTES),
        "Text translation request exceeds the page batching limits"
    );
    for region in regions {
        safety::response_text(&region.source, "")?;
    }
    Ok(())
}

/// A labelled reply is all-or-nothing. Indented list labels are target content.
pub fn simple_results(text: &str, regions: &[Region]) -> Result<Vec<TranslationItem>> {
    ensure!(!regions.is_empty(), "{INVALID_REPLY}");
    let values = if regions.len() == 1 {
        vec![text.trim().to_owned()]
    } else {
        let mut values: Vec<String> = Vec::new();
        for line in text.trim().lines() {
            ensure!(
                !line.trim_start().starts_with("```") && !line.trim_start().starts_with("~~~"),
                "{INVALID_REPLY}"
            );
            if let Some(label) = line
                .strip_prefix('[')
                .filter(|s| s.chars().next().is_some_and(|c| c.is_numeric()))
            {
                let (number, target) = label
                    .split_once(']')
                    .ok_or_else(|| anyhow::anyhow!(INVALID_REPLY))?;
                ensure!(
                    number == (values.len() + 1).to_string() && values.len() < regions.len(),
                    "{INVALID_REPLY}"
                );
                values.push(target.trim_start().to_owned());
            } else {
                let previous = values
                    .last_mut()
                    .ok_or_else(|| anyhow::anyhow!(INVALID_REPLY))?;
                previous.push('\n');
                let indent = line.bytes().take(4).take_while(|b| *b == b' ').count();
                previous.push_str(&line[indent..]);
            }
        }
        ensure!(values.len() == regions.len(), "{INVALID_REPLY}");
        values
    };
    regions
        .iter()
        .zip(values)
        .map(|(region, target)| {
            let target = target.trim();
            ensure!(!target.is_empty(), "{INVALID_REPLY}");
            safety::response_text(&region.source, target)?;
            Ok(TranslationItem {
                id: region.id.clone(),
                source: region.source.clone(),
                target: target.into(),
                direction: String::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn regions(n: usize) -> Vec<Region> {
        (0..n)
            .map(|i| Region {
                id: format!("region-{i}"),
                source: "原文".into(),
                ..Default::default()
            })
            .collect()
    }
    #[test]
    fn whole_page_and_count_guards_keep_order() {
        for simple in [false, true] {
            for (n, expected) in [
                (0, vec![]),
                (1, vec![1]),
                (2, vec![2]),
                (48, vec![48]),
                (49, vec![48, 1]),
                (97, vec![48, 48, 1]),
            ] {
                let input = regions(n);
                let batches = plan(&input, simple).unwrap();
                assert_eq!(
                    batches.iter().map(|b| b.len()).collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(
                    batches
                        .iter()
                        .flat_map(|b| b.iter().map(|r| &r.id))
                        .collect::<Vec<_>>(),
                    input.iter().map(|r| &r.id).collect::<Vec<_>>()
                );
                for batch in batches {
                    validate_request(batch, simple).unwrap();
                }
            }
        }
    }
    #[test]
    fn byte_guards_use_actual_framing_and_keep_oversized_regions_whole() {
        for simple in [false, true] {
            let mut input = regions(2);
            let overhead = current_text(&input, simple).unwrap().len();
            input[1]
                .source
                .push_str(&"x".repeat(MAX_CURRENT_BYTES - overhead));
            assert_eq!(
                current_text(&input, simple).unwrap().len(),
                MAX_CURRENT_BYTES
            );
            assert_eq!(plan(&input, simple).unwrap().len(), 1);
            input[1].source.push('字');
            assert_eq!(plan(&input, simple).unwrap().len(), 2);
            assert!(validate_request(&input, simple).is_err());
            input[0].source = "字".repeat(MAX_CURRENT_BYTES);
            let batches = plan(&input, simple).unwrap();
            assert_eq!(batches.len(), 2);
            assert_eq!(batches[0][0].source, input[0].source);
            validate_request(batches[0], simple).unwrap();
        }
        let mut input = regions(2);
        input[0].source = "甲\r\n[2] 内部\r乙".into();
        assert!(
            current_text(&input, true)
                .unwrap()
                .starts_with("[1] 甲\n    [2] 内部\n    乙\n[2]")
        );
        assert_eq!(input[0].source, "甲\r\n[2] 内部\r乙");
    }
    #[test]
    fn labelled_results_preserve_paragraphs_nested_lists_and_identity() {
        let input = regions(2);
        let reply = "\n[1] 甲\r\n    [1] 内部条目\r\n\r\n    第二段\r\n[2] ♥\n";
        let items = simple_results(reply, &input).unwrap();
        assert_eq!(items[0].target, "甲\n[1] 内部条目\n\n第二段");
        assert_eq!(items[1].target, "♥");
        assert_eq!(items[0].id, input[0].id);
        assert_eq!(items[0].source, input[0].source);
        assert_eq!(
            simple_results("1. 原样\n2. 保留", &input[..1]).unwrap()[0].target,
            "1. 原样\n2. 保留"
        );
    }
    #[test]
    fn bad_batches_never_return_partial_results() {
        let input = regions(2);
        for text in [
            "",
            "[1] 甲",
            "[1] 甲\n[2] ",
            "[1] 甲\n[1] 乙",
            "[2] 乙\n[1] 甲",
            "[1] 甲\n[3] 乙",
            "译文：\n[1] 甲\n[2] 乙",
            "```\n[1] 甲\n[2] 乙\n```",
            "[01] 甲\n[2] 乙",
            "[1] 甲\n[２] 乙",
            "[1] 甲\n[2x] 乙",
            "[1] 甲\n[2] 乙\n[3] 多余",
            "[1] 甲\n[2] 乙\n~~~",
        ] {
            assert!(simple_results(text, &input).is_err(), "{text}");
        }
        assert!(
            simple_results(
                &format!("[1] 甲\n[2] {}", "字".repeat(safety::MAX_TARGET_CHARS + 1)),
                &input
            )
            .is_err()
        );
    }
}
