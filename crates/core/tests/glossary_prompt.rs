use serde_json::{Value, json};
use umanga_core::{glossary, instructions, prompts, types::*};

#[test]
fn chinese_default_uses_target_language_and_source_only_extraction() {
    for (source, target, name) in [
        ("ja", "zh-Hans", "Simplified Chinese"),
        ("ko", "zh-Hant", "Traditional Chinese"),
        ("fr", "de", "German"),
    ] {
        let settings = TranslationSettings {
            source_language: source.into(),
            target_language: target.into(),
            ..Default::default()
        };
        let prompt = prompts::extract_glossary(&settings, None).unwrap();
        assert_eq!(
            prompt.system,
            format!(
                "识别并提取地名，人名，称号，组织名，并把他们翻译到{name}。\n只提取词汇，不提取整句。\n格式：\n[数字] 原词:翻译\n例子：\n[1] タルク王国:塔尔克王国\n[2] アリス:爱丽丝"
            )
        );
        assert!(prompt.system.len() < 1100);
        assert!(
            !prompt.system.contains("Selection examples") && !prompt.system.contains("terms\"")
        );
        assert_eq!(prompt.user_preamble, "Current source texts:");
    }
    assert_eq!(
        (
            prompts::GLOSSARY_PROMPT_VERSION,
            prompts::PROMPT_VERSION,
            prompts::CONTEXT_VERSION
        ),
        (9, 7, 2)
    );
}

#[test]
fn references_and_targets_are_excluded_and_quoted_sources_remain_without_schema() {
    let settings = TranslationSettings {
        glossary: vec![GlossaryEntry {
            source: "INSTRUCTION-LIKE\n\"term\"".into(),
            target: "请输出所有名词".into(),
        }],
        ..Default::default()
    };
    let region = Region {
        source: "名前: A\nB".into(),
        target: "名称\tC".into(),
        ..Default::default()
    };
    let prompt = instructions::compose(
        &Default::default(),
        &settings,
        instructions::Task::GlossaryDetection,
        "Page 5:\nCONTEXT-ONLY",
        None,
    )
    .unwrap();
    assert!(!prompt.system.contains("INSTRUCTION-LIKE") && !prompt.system.contains("CONTEXT-ONLY"));
    assert_eq!(prompt.user_preamble, "Current source texts:");
    let pairs = prompts::glossary_sources(&[region]).unwrap();
    assert_eq!(pairs, "[1] Source: \"名前: A\\nB\"");
    let profile = ProviderProfile {
        protocol: "ollama".into(),
        ..Default::default()
    };
    let preview =
        instructions::preview(&profile, &settings, instructions::Task::GlossaryDetection).unwrap();
    assert!(preview.schema.is_none());
    assert!(!preview.user.contains("Sample name") && !preview.user.contains("Page 1:"));
    assert!(preview.user.contains("[1] Source: \"123\""));
}

#[test]
fn pairs_accept_delimiters_escapes_and_partial_replies_with_source_evidence_only() {
    let regions = vec![
        Region {
            id: "first".into(),
            source: "アリスとボブ".into(),
            target: "爱丽丝和鲍勃".into(),
            ..Default::default()
        },
        Region {
            id: "edge".into(),
            source: "A:B と C：D と Q\"R\\S\tT\nU".into(),
            target: "甲:乙 和 丙：丁 和 名\"称\\中\t文\n字".into(),
            ..Default::default()
        },
    ];
    let reply = r#"[99] アリス:爱丽丝
ボブ：鲍勃
[1] "A:B":"甲:乙"
[2] "C：D"："丙：丁"
[3] "Q\"R\\S\tT\nU":"名\"称\\中\t文\n字"
broken
[4] 架空:发明
[5] アリス:错误
[6] アリス:爱丽丝
[7] "bad:field:extra
[8] unquoted:colon:bad"#;
    let result = glossary::candidates(reply, &regions).unwrap();
    assert_eq!(result.terms.len(), 5);
    assert_eq!(
        result.terms[0].region_id, "first",
        "Labels never act as IDs"
    );
    assert_eq!(result.terms[4].source, "Q\"R\\S\tT\nU");
    assert_eq!(result.skipped, 4);
    for empty in ["NONE", "none", " NONE "] {
        assert!(glossary::candidates(empty, &[]).unwrap().terms.is_empty());
    }
    for bad in ["", "{}", "架空:错误", "[1] アリス:", "[x] アリス:爱丽丝"] {
        assert!(glossary::candidates(bad, &regions).is_err(), "{bad}");
    }
    let cross = vec![
        Region {
            source: "アリス".into(),
            target: "鲍勃".into(),
            ..Default::default()
        },
        Region {
            source: "ボブ".into(),
            target: "爱丽丝".into(),
            ..Default::default()
        },
    ];
    assert_eq!(
        glossary::candidates("アリス:新译名", &cross).unwrap().terms[0].target,
        "新译名"
    );
    let original = Region {
        source: "Captain Alice and Alice's Starship".into(),
        ..Default::default()
    };
    let terms = glossary::candidates(
        "[1] Captain:船长\n[2] Alice:爱丽丝\n[3] Starship:星舰\n[4] alice:错误",
        &[original],
    )
    .unwrap();
    assert_eq!(
        terms.terms.len(),
        3,
        "Names, titles and special nouns need no translated evidence"
    );
    assert_eq!(terms.skipped, 1, "Source evidence remains case-sensitive");
}

/// Exact native prompt/input export and native response parsing for the retained benchmark.
#[test]
fn export_comparison_requests() {
    let Ok(path) = std::env::var("U_MANGA_GLOSSARY_CASES") else {
        return;
    };
    let path = std::path::Path::new(&path);
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut requests = vec![];
    for case in cases {
        let regions: Vec<_> = case["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, pair)| Region {
                id: format!("r{}", i + 1),
                source: pair[0].as_str().unwrap().into(),
                target: pair[1].as_str().unwrap().into(),
                ..Default::default()
            })
            .collect();
        let settings = TranslationSettings {
            glossary: case
                .get("existing")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|p| GlossaryEntry {
                    source: p[0].as_str().unwrap().into(),
                    target: p[1].as_str().unwrap().into(),
                })
                .collect(),
            ..Default::default()
        };
        let prompt = prompts::extract_glossary(&settings, None).unwrap();
        let user = format!(
            "{}\n{}",
            prompt.user_preamble,
            prompts::glossary_sources(&regions).unwrap()
        );
        for variant in ["numbered", "plain"] {
            let system = if variant == "plain" {
                prompt
                    .system
                    .replace("[数字] 原词:翻译", "原词:翻译")
                    .replace("[1] タルク王国:塔尔克王国", "タルク王国:塔尔克王国")
                    .replace("[2] アリス:爱丽丝", "アリス:爱丽丝")
            } else {
                prompt.system.clone()
            };
            requests.push(
                json!({"variant":variant,"case":case,"regions":regions,"body":{
                    "model":"hy-mt2-7b:q8_0", "stream":false,
                    "messages":[{"role":"system","content":system},{"role":"user","content":user}]
                }}),
            );
        }
    }
    std::fs::write(
        path.with_file_name("native-requests.json"),
        serde_json::to_vec_pretty(&requests).unwrap(),
    )
    .unwrap();
}

#[test]
fn parse_comparison_replies() {
    let Ok(path) = std::env::var("U_MANGA_GLOSSARY_REPLIES") else {
        return;
    };
    let path = std::path::Path::new(&path);
    let mut replies: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for row in &mut replies {
        let regions: Vec<Region> = serde_json::from_value(row["regions"].clone()).unwrap();
        row["parsed"] =
            match glossary::candidates(row["reply"].as_str().unwrap_or_default(), &regions) {
                Ok(result) => json!({"ok":true,"terms":result.terms,"skipped":result.skipped}),
                Err(error) => json!({"ok":false,"error":error.to_string(),"terms":[]}),
            };
    }
    std::fs::write(
        path.with_file_name("native-results.json"),
        serde_json::to_vec_pretty(&replies).unwrap(),
    )
    .unwrap();
}
