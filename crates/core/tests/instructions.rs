use serde_json::{Value, json};
use umanga_core::{
    instructions::{self, Overrides, Task},
    prompts, providers, store,
    types::*,
};

#[test]
fn references_are_optional_quoted_data_and_have_short_guidance() {
    let mut settings = TranslationSettings {
        glossary_enabled: true,
        glossary: vec![GlossaryEntry {
            source: " a: \"name\"\nnext ".into(),
            target: " 值:\r\n\"quoted\" ".into(),
        }],
        ..Default::default()
    };
    for task in [
        Task::TextTranslation,
        Task::VisionTranslation,
        Task::SimpleTextTranslation,
    ] {
        let prompt = instructions::compose(
            &Overrides::default(),
            &settings,
            task,
            "Page 2:\nDialogue",
            None,
        )
        .unwrap();
        assert!(
            prompt
                .user_preamble
                .contains(r#"" a: \"name\"\nnext ": " 值:\r\n\"quoted\" ""#)
        );
        assert!(
            prompt
                .user_preamble
                .contains("Use previous dialogue as context; translate only current text.")
        );
        assert!(!prompt.system.contains("Dialogue"));
        assert!(!prompt.user_preamble.contains("(none)"));
    }
    settings.glossary_enabled = false;
    for task in [Task::TextTranslation, Task::VisionTranslation] {
        assert_eq!(
            instructions::compose(&Overrides::default(), &settings, task, "", None)
                .unwrap()
                .user_preamble,
            "Current regions to translate:"
        );
    }
    assert_eq!(
        instructions::compose(
            &Overrides::default(),
            &settings,
            Task::GlossaryDetection,
            "",
            None
        )
        .unwrap()
        .user_preamble,
        "Current source texts:"
    );
    settings.glossary.clear();
    assert_eq!(
        instructions::compose(
            &Overrides::default(),
            &settings,
            Task::GlossaryDetection,
            "",
            None
        )
        .unwrap()
        .user_preamble,
        "Current source texts:"
    );
}

#[test]
fn complete_defaults_keep_formats_and_protocol_schema_separate() {
    let settings = TranslationSettings::default();
    let profile = ProviderProfile {
        protocol: "ollama".into(),
        ..Default::default()
    };
    for task in [
        Task::TextTranslation,
        Task::VisionTranslation,
        Task::GlossaryDetection,
    ] {
        let preview = instructions::preview(&profile, &settings, task).unwrap();
        assert_eq!(preview.prompt, preview.system);
        assert!(!preview.system.contains("Response JSON Schema"));
        assert_eq!(preview.schema.is_none(), task == Task::GlossaryDetection);
        assert!(preview.system.contains(if task == Task::GlossaryDetection {
            "[数字] 原词:翻译"
        } else {
            r#""regions""#
        }));
        if task != Task::GlossaryDetection {
            assert_eq!(preview.system.matches("Japanese (ja)").count(), 1);
            assert_eq!(
                preview
                    .system
                    .matches("Simplified Chinese (zh-Hans)")
                    .count(),
                1
            );
            assert!(!preview.system.contains("glossary"));
            assert!(preview.system.len() < 600);
        }
    }
    let simple = instructions::default_template(Task::SimpleTextTranslation, "ja");
    assert!(simple.contains("numbered input"));
    assert!(simple.contains("unnumbered input"));
    assert!(!simple.contains("glossary"));
}

#[test]
fn custom_and_empty_templates_replace_every_prompt_instruction() {
    let settings = TranslationSettings::default();
    for value in ["CUSTOM {{target_language}}", ""] {
        let custom = Overrides {
            text_translation: Some(value.into()),
            vision_translation: Some(value.into()),
            glossary_detection: Some(value.into()),
            simple_text_translation: Some(value.into()),
        };
        for task in [
            Task::SimpleTextTranslation,
            Task::TextTranslation,
            Task::VisionTranslation,
            Task::GlossaryDetection,
        ] {
            let prompt = instructions::compose(
                &custom,
                &settings,
                task,
                "DATA-ONLY",
                Some(&json!({"schema":"in protocol"})),
            )
            .unwrap();
            assert_eq!(
                prompt.system,
                value.replace("{{target_language}}", "Simplified Chinese")
            );
            assert_eq!(
                prompt.user_preamble.contains("DATA-ONLY"),
                task != Task::GlossaryDetection
            );
            assert!(!prompt.system.contains("in protocol"));
        }
        let r = Region {
            source: "hello".into(),
            ..Default::default()
        };
        let prompt = instructions::simple_prompt(&custom, &settings, "", &[r]).unwrap();
        assert_eq!(
            prompt.system,
            value.replace("{{target_language}}", "Simplified Chinese")
        );
        assert_eq!(prompt.user_preamble, "Current text:\nhello");
    }
}

#[test]
fn templates_are_single_pass_bounded_and_keep_json_unicode() {
    let mut settings = TranslationSettings::default();
    let template = "{{source_language}} {{source_language_code}} → {{target_language}} {{target_language_code}}\n{\"a\":{\"b\":\"中文\"}}";
    instructions::validate_template(template).unwrap();
    assert_eq!(
        instructions::expand(template, &settings).unwrap(),
        "Japanese ja → Simplified Chinese zh-Hans\n{\"a\":{\"b\":\"中文\"}}"
    );
    settings.source_language = "{{target_language}}".into();
    assert_eq!(
        instructions::expand("{{source_language_code}}", &settings).unwrap(),
        "{{target_language}}"
    );
    for invalid in [
        "{{unknown}}".to_owned(),
        "汉".repeat(6000),
        "a".repeat(16385),
    ] {
        assert!(instructions::validate_template(&invalid).is_err());
    }
    instructions::validate_template(&"a".repeat(16384)).unwrap();
    instructions::validate_template("").unwrap();
}

#[test]
fn capture_freezes_defaults_and_roundtrips_empty_overrides() {
    let settings = TranslationSettings::default();
    let mut provider = ProviderProfile::default();
    provider.instructions.vision_translation = Some(String::new());
    let captured = instructions::capture(&provider, &settings).unwrap();
    provider.instructions.text_translation = Some("LATER".into());
    let restored: ProviderProfile =
        serde_json::from_value(serde_json::to_value(&captured).unwrap()).unwrap();
    assert_eq!(restored.instructions, captured.instructions);
    assert!(
        restored
            .instructions
            .text_translation
            .as_ref()
            .unwrap()
            .contains("natural dialogue")
    );
    assert_eq!(
        restored.instructions.vision_translation.as_deref(),
        Some("")
    );
    assert!(
        restored
            .instructions
            .glossary_detection
            .as_ref()
            .unwrap()
            .contains("[数字] 原词:翻译")
    );
    assert!(
        instructions::default_template(Task::GlossaryDetection, "en").contains("[数字] 原词:翻译")
    );
}

#[test]
fn cache_identity_is_scoped_to_effective_request_guidance() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("source.png");
    image::RgbImage::new(24, 32).save(&image).unwrap();
    let page = umanga_core::documents::import(&[image.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let mut settings = TranslationSettings::default();
    let mut p = ProviderProfile {
        simple_translation: false,
        ..Default::default()
    };
    let base = store::cache_key(&page, &settings, &p, "").unwrap();
    let extraction = instructions::extraction_identity(&p, &settings).unwrap();
    p.instructions.text_translation =
        Some(instructions::default_template(Task::TextTranslation, "ja"));
    assert_eq!(base, store::cache_key(&page, &settings, &p, "").unwrap());
    p.instructions.text_translation = Some("NEW TEXT".into());
    assert_ne!(base, store::cache_key(&page, &settings, &p, "").unwrap());
    assert_eq!(
        extraction,
        instructions::extraction_identity(&p, &settings).unwrap()
    );
    let translated = store::cache_key(&page, &settings, &p, "").unwrap();
    p.instructions.glossary_detection = Some("NEW GLOSSARY".into());
    p.instructions.vision_translation = Some("NEW VISION".into());
    assert_eq!(
        translated,
        store::cache_key(&page, &settings, &p, "").unwrap()
    );
    assert_ne!(
        extraction,
        instructions::extraction_identity(&p, &settings).unwrap()
    );
    settings.mode = "vision".into();
    assert_eq!(
        instructions::translation_task(&p, &settings),
        Task::VisionTranslation
    );
    settings.auto_glossary = true;
    settings.glossary_enabled = true;
    assert_eq!(
        instructions::translation_task(&p, &settings),
        Task::VisionTranslation
    );
}

#[test]
fn native_previews_match_reviewed_browser_fixtures() {
    let mut fixtures = Vec::new();
    for source in ["ja", "en"] {
        for protocol in ["openai", "ollama"] {
            for task in [
                Task::SimpleTextTranslation,
                Task::TextTranslation,
                Task::VisionTranslation,
                Task::GlossaryDetection,
            ] {
                let settings = TranslationSettings {
                    source_language: source.into(),
                    ..Default::default()
                };
                let provider = ProviderProfile {
                    protocol: protocol.into(),
                    ..Default::default()
                };
                let preview = instructions::preview(&provider, &settings, task).unwrap();
                let context = "Page 1:\n456";
                let schema = preview.schema.as_ref();
                let composed =
                    instructions::compose(&provider.instructions, &settings, task, context, schema)
                        .unwrap();
                assert_eq!(preview.system, composed.system);
                fixtures.push(
                    json!({"source":source,"protocol":protocol,"task":task,"preview":preview}),
                );
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::create_dir_all(root.join("test-output/instructions")).unwrap();
    std::fs::write(
        root.join("test-output/instructions/previews.json"),
        serde_json::to_vec_pretty(&fixtures).unwrap(),
    )
    .unwrap();
    let saved: Value =
        serde_json::from_str(include_str!("../../../resources/instruction-previews.json")).unwrap();
    assert!(
        json!(fixtures) == saved,
        "Review test-output/instructions/previews.json before updating the shared fixture"
    );
}

#[test]
fn queued_jobs_keep_instruction_snapshots_and_reject_invalid_templates() {
    use std::{path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("original.png");
    image::RgbImage::new(32, 48).save(&image).unwrap();
    let pages = umanga_core::documents::import(&[image.to_string_lossy().into()]).unwrap();
    let book = store::create(&temp.path().join("book.umanga"), "Instructions", &pages).unwrap();
    let engine = umanga_core::pipeline::Engine::new(
        temp.path().join("models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("No service calls")),
    )
    .unwrap();
    let settings = TranslationSettings::default();
    let mut profile = ProviderProfile::default();
    profile.instructions.text_translation = Some("Captured guidance".into());
    engine
        .enqueue_configured(&book.path, &[pages[0].id.clone()], &profile, &settings)
        .unwrap();
    profile.instructions.text_translation = Some("Later guidance".into());
    let jobs = store::jobs(Path::new(&book.path)).unwrap();
    assert_eq!(
        jobs[0].provider.instructions.text_translation.as_deref(),
        Some("Captured guidance")
    );
    assert!(jobs[0].provider.instructions.glossary_detection.is_some());
    let mut invalid = jobs[0].clone();
    invalid.provider.instructions.text_translation = Some("{{invalid}}".into());
    assert!(
        umanga_core::job_validation::validate(&invalid.id, &invalid, 0, &Default::default())
            .is_err()
    );
    profile.instructions.text_translation = Some("{{invalid}}".into());
    assert!(
        engine
            .enqueue_configured(&book.path, &[pages[0].id.clone()], &profile, &settings)
            .is_err()
    );
    assert_eq!(store::jobs(Path::new(&book.path)).unwrap().len(), 1);
}

#[tokio::test]
async fn actual_requests_use_custom_guidance_and_keep_transcription_independent() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    let p = ProviderProfile {
        simple_translation: false,
        endpoint: server.uri(),
        model: "test".into(),
        rate_limit: 20.,
        instructions: Overrides {
            text_translation: Some("TEXT-CUSTOM".into()),
            vision_translation: Some("VISION-CUSTOM".into()),
            glossary_detection: Some("GLOSSARY-CUSTOM".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    Mock::given(method("POST")).respond_with(|r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        let result = if system.starts_with("GLOSSARY-CUSTOM") { "NONE".to_owned() } else { json!({"regions":[{"id":"r","source":"hello","target":"你好"}]}).to_string() };
        ResponseTemplate::new(200).set_body_json(json!({"id":"mock","choices":[{"index":0,"message":{"role":"assistant","content":result},"finish_reason":"stop"}]}))
    }).mount(&server).await;
    let region = Region {
        id: "r".into(),
        source: "hello".into(),
        bbox: [0., 0., 32., 32.],
        ..Default::default()
    };
    let settings = TranslationSettings::default();
    providers::llm(
        &p,
        "mock",
        &settings,
        std::slice::from_ref(&region),
        None,
        "",
    )
    .await
    .unwrap();
    providers::llm(
        &p,
        "mock",
        &settings,
        std::slice::from_ref(&region),
        Some(&image::DynamicImage::new_rgb8(32, 32)),
        "",
    )
    .await
    .unwrap();
    providers::extract_glossary(&p, "mock", &settings, std::slice::from_ref(&region))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    for (request, expected) in
        requests
            .iter()
            .zip(["TEXT-CUSTOM", "VISION-CUSTOM", "GLOSSARY-CUSTOM"])
    {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .eq(expected)
        );
        assert!(body.get("temperature").is_none());
    }
    assert!(
        !prompts::transcription(&settings, None)
            .unwrap()
            .system
            .contains("CUSTOM")
    );
}
