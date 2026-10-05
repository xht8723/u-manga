use serde_json::{Value, json};
use umanga_core::{
    instructions::{self, Task},
    providers, store,
    types::*,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
#[path = "support/generation.rs"]
mod generation;

#[tokio::test]
async fn glossary_pairs_use_no_response_schema_on_every_adapter() {
    for (protocol, route) in [
        ("openai", "/chat/completions"),
        ("responses", "/responses"),
        ("anthropic", "/messages"),
        ("gemini", "/models/simple:1b:generateContent"),
        ("ollama", "/v1/chat/completions"),
    ] {
        for content in ["[1] アリス:爱丽丝\nbad row", "NONE", "unusable"] {
            for template in [None, Some("GLOSSARY-CUSTOM"), Some("")] {
                let (server, mut p) = profile(protocol).await;
                p.instructions.glossary_detection = template.map(str::to_owned);
                Mock::given(method("POST"))
                    .and(path(route))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(response(protocol, content, "stop")),
                    )
                    .mount(&server)
                    .await;
                let region = Region {
                    source: "アリスは城へ".into(),
                    target: "爱丽丝去了城堡".into(),
                    ..Default::default()
                };
                let result = providers::extract_glossary(
                    &p,
                    "mock",
                    &TranslationSettings {
                        glossary_enabled: true,
                        glossary: vec![GlossaryEntry {
                            source: "EXISTING-ONLY-SENTINEL".into(),
                            target: "DO-NOT-SEND".into(),
                        }],
                        ..Default::default()
                    },
                    &[region],
                )
                .await;
                if content == "unusable" {
                    assert!(result.is_err());
                } else {
                    let result = result.unwrap();
                    assert_eq!(result.terms.len(), usize::from(content != "NONE"));
                    assert_eq!(result.skipped, usize::from(content != "NONE"));
                }
                let requests = server.received_requests().await.unwrap();
                let calls: Vec<_> = requests.iter().filter(|r| r.url.path() == route).collect();
                assert_eq!(calls.len(), 1, "No repair call");
                let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
                assert_eq!(
                    body.to_string().contains("GLOSSARY-CUSTOM"),
                    template == Some("GLOSSARY-CUSTOM")
                );
                assert_eq!(
                    body.to_string().contains("识别并提取地名"),
                    template.is_none()
                );
                generation::assert_service_defaults(&body, protocol);
                for field in [
                    "response_format",
                    "format",
                    "responseJsonSchema",
                    "responseSchema",
                ] {
                    assert!(
                        body.get(field).is_none() && body["generationConfig"].get(field).is_none(),
                        "{protocol}: {field}"
                    );
                }
                assert!(
                    body.to_string().contains("Source:")
                        && !body.to_string().contains("Translation:")
                        && !body.to_string().contains("爱丽丝去了城堡")
                );
                assert!(!body.to_string().contains("Region ID:"));
                assert!(!body.to_string().contains("EXISTING-ONLY-SENTINEL"));
                assert!(
                    !body.to_string().contains("Existing terms:")
                        && !body.to_string().contains("previous dialogue")
                );
            }
        }
    }
}

fn source(text: &str) -> Region {
    Region {
        id: "private-region-id".into(),
        source: text.into(),
        bbox: [0., 0., 40., 40.],
        ..Default::default()
    }
}
async fn profile(protocol: &str) -> (MockServer, ProviderProfile) {
    let server = MockServer::start().await;
    if protocol == "ollama" {
        Mock::given(path("/api/tags"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"models":[{"name":"simple:1b","digest":"simple","size":1}]}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(path("/api/show")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion","vision","thinking"],"thinking":{"values":[false,true]}}))).mount(&server).await;
        umanga_core::ollama::details(&server.uri(), "simple:1b", true).await;
    }
    let p = ProviderProfile {
        simple_translation: true,
        protocol: protocol.into(),
        endpoint: server.uri(),
        model: "simple:1b".into(),
        vision: true,
        rate_limit: 100.,
        ..Default::default()
    };
    (server, p)
}
fn response(protocol: &str, content: &str, finish: &str) -> Value {
    match protocol {
        "anthropic" => {
            json!({"id":"test","type":"message","role":"assistant","model":"simple:1b","stop_reason":"end_turn","content":[{"type":"text","text":content}],"usage":{"input_tokens":1,"output_tokens":1}})
        }
        "gemini" => {
            json!({"modelVersion":"simple:1b","candidates":[{"content":{"parts":[{"text":content}]},"finishReason":"STOP"}],"usageMetadata":{}})
        }
        "responses" => {
            json!({"id":"test","model":"simple:1b","status":"completed","output":[{"id":"msg","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":content,"annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
        }
        _ => {
            json!({"id":"test","model":"simple:1b","choices":[{"index":0,"message":{"role":"assistant","content":content,"reasoning_content":"NEVER TRANSLATE THIS REASONING"},"finish_reason":finish}]})
        }
    }
}

#[tokio::test]
async fn all_adapters_separate_system_and_user_without_schema_and_keep_thinking() {
    for (protocol, route, thinking_pointer, off, on) in [
        (
            "openai",
            "/chat/completions",
            "/reasoning_effort",
            json!("none"),
            json!("low"),
        ),
        (
            "responses",
            "/responses",
            "/reasoning/effort",
            json!("none"),
            json!("low"),
        ),
        (
            "anthropic",
            "/messages",
            "/thinking/type",
            Value::Null,
            json!("enabled"),
        ),
        (
            "gemini",
            "/models/simple:1b:generateContent",
            "/generationConfig/thinkingConfig/thinkingBudget",
            json!(0),
            json!(1000),
        ),
        (
            "ollama",
            "/v1/chat/completions",
            "/reasoning_effort",
            json!("none"),
            json!("low"),
        ),
    ] {
        let (server, mut p) = profile(protocol).await;
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(response(
                protocol,
                "[1] 第一行\n\n    第二行♥\n[2] 456\n[3] 第三条",
                "stop",
            )))
            .expect(6)
            .mount(&server)
            .await;
        let s = TranslationSettings {
            mode: "vision".into(),
            glossary_enabled: true,
            glossary: vec![GlossaryEntry {
                source: "Sample name".into(),
                target: "Preferred name".into(),
            }],
            ..Default::default()
        };
        p.instructions.text_translation = Some("STRUCTURED-ONLY".into());
        let regions: Vec<_> = ["123", "456", "789"]
            .into_iter()
            .enumerate()
            .map(|(i, text)| Region {
                id: format!("private-region-id-{i}"),
                ..source(text)
            })
            .collect();
        for template in [None, Some("CUSTOM {{target_language}}."), Some("")] {
            p.instructions.simple_text_translation = template.map(str::to_owned);
            for (enabled, expected) in [(false, off.clone()), (true, on.clone())] {
                p.thinking = enabled;
                p.thinking_policy = Some(umanga_core::thinking::resolve(&p).await);
                let items = providers::llm(&p, "mock", &s, &regions, None, "Page 1:\n456")
                    .await
                    .unwrap();
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].id, regions[0].id);
                assert_eq!(items[0].source, "123");
                assert_eq!(items[0].target, "第一行\n\n第二行♥");
                assert_eq!(items[1].target, "456");
                assert_eq!(items[2].target, "第三条");
                let requests = server.received_requests().await.unwrap();
                let body: Value = serde_json::from_slice(
                    &requests
                        .iter()
                        .rev()
                        .find(|r| r.url.path() == route)
                        .unwrap()
                        .body,
                )
                .unwrap();
                assert_eq!(
                    body.pointer(thinking_pointer)
                        .cloned()
                        .unwrap_or(Value::Null),
                    expected,
                    "{protocol}"
                );
                generation::assert_service_defaults(&body, protocol);
                let serialized = body.to_string();
                for absent in [
                    "response_format",
                    "responseJsonSchema",
                    "responseSchema",
                    "STRUCTURED-ONLY",
                    "private-region-id",
                    "image_url",
                    "inlineData",
                ] {
                    assert!(!serialized.contains(absent), "{protocol}: {serialized}");
                }
                let preview = instructions::preview(&p, &s, Task::SimpleTextTranslation).unwrap();
                let sent = if protocol == "responses" {
                    &body["input"]
                } else if protocol == "gemini" {
                    &body["contents"]
                } else {
                    &body["messages"]
                };
                let separated_system = match protocol {
                    "anthropic" => body.get("system").and_then(Value::as_str),
                    "gemini" => body
                        .pointer("/systemInstruction/parts/0/text")
                        .and_then(Value::as_str),
                    _ => sent
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|m| m["role"] == "system")
                        .and_then(|m| m["content"].as_str()),
                };
                assert_eq!(
                    separated_system.unwrap_or(""),
                    preview.system,
                    "{protocol}: {body}"
                );
                let user = sent
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["role"] == "user")
                    .unwrap();
                let expected_count =
                    if !preview.system.is_empty() && !matches!(protocol, "anthropic" | "gemini") {
                        2
                    } else {
                        1
                    };
                assert_eq!(
                    sent.as_array().unwrap().len(),
                    expected_count,
                    "{protocol}: {body}"
                );
                let parts = if protocol == "gemini" {
                    &user["parts"]
                } else {
                    &user["content"]
                };
                let actual = parts
                    .as_str()
                    .or_else(|| parts[0]["text"].as_str())
                    .unwrap();
                assert_eq!(actual, preview.user, "{protocol}");
                if !preview.system.is_empty() {
                    assert!(!actual.contains(&preview.system));
                }
            }
        }
    }
}

#[test]
fn prompt_defaults_empty_custom_capture_and_sources() {
    let s = TranslationSettings::default();
    let mut p = ProviderProfile::default();
    assert!(p.simple_translation);
    p.instructions.simple_text_translation = Some("Custom {{target_language}}.".into());
    let captured = instructions::capture(&p, &s).unwrap();
    p.simple_translation = false;
    p.instructions.simple_text_translation = Some("LATER".into());
    let restored: ProviderProfile =
        serde_json::from_value(serde_json::to_value(&captured).unwrap()).unwrap();
    assert!(restored.simple_translation);
    let saved_off: ProviderProfile =
        serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert!(!saved_off.simple_translation);
    assert_eq!(
        instructions::translation_task(&saved_off, &s),
        Task::TextTranslation
    );
    assert_eq!(
        instructions::translation_task(&restored, &s),
        Task::SimpleTextTranslation
    );
    assert_eq!(
        instructions::translation_task(
            &restored,
            &TranslationSettings {
                mode: "vision".into(),
                ..s.clone()
            }
        ),
        Task::VisionTranslation
    );
    let prompt = instructions::compose(
        &restored.instructions,
        &s,
        Task::SimpleTextTranslation,
        "",
        None,
    )
    .unwrap();
    assert_eq!(prompt.system, "Custom Simplified Chinese.");
    assert_eq!(prompt.user_preamble, "Current text:");
    assert!(!prompt.user_preamble.contains("Glossary:"));
    assert!(!prompt.user_preamble.contains("Previous dialogue:"));
    p.instructions.simple_text_translation = Some(String::new());
    let prompt =
        instructions::compose(&p.instructions, &s, Task::SimpleTextTranslation, "", None).unwrap();
    assert!(prompt.system.is_empty());
    assert!(prompt.user_preamble.starts_with("Current text:"));
    assert!(!prompt.user_preamble.contains("natural manga"));
    for readable in ["…", "♥", "!?", "■", "hello ■■", "\n日本語\n", "3.14"] {
        assert!(instructions::simple_source_readable(readable));
    }
    for unreadable in ["", " \n　", "■■■■", "█ ■\n"] {
        assert!(!instructions::simple_source_readable(unreadable));
    }
}

#[tokio::test]
async fn structured_adapters_accept_a_whole_page_and_preserve_partial_validation() {
    for (protocol, route) in [
        ("openai", "/chat/completions"),
        ("responses", "/responses"),
        ("anthropic", "/messages"),
        ("gemini", "/models/simple:1b:generateContent"),
        ("ollama", "/v1/chat/completions"),
    ] {
        let (server, mut p) = profile(protocol).await;
        p.simple_translation = false;
        let regions: Vec<_> = (0..48)
            .map(|i| Region {
                id: format!("region-{i}"),
                ..source(&format!("原文{i}"))
            })
            .collect();
        Mock::given(path(route)).respond_with(ResponseTemplate::new(200).set_body_json(response(protocol,
            &json!({"regions":[{"id":"region-0","source":"rewritten","target":"有效译文"}]}).to_string(),"stop")))
            .expect(1).mount(&server).await;
        let items = providers::llm(
            &p,
            "mock",
            &TranslationSettings::default(),
            &regions,
            None,
            "Page 12:\n前文",
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "原文0");
        assert_eq!(items[0].target, "有效译文");
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(
            &requests
                .iter()
                .find(|r| r.url.path() == route)
                .unwrap()
                .body,
        )
        .unwrap();
        generation::assert_service_defaults(&body, protocol);
        let serialized = body.to_string();
        assert_eq!(serialized.matches("Region ID:").count(), 48, "{protocol}");
        assert!(serialized.contains("原文47"));
        assert!(serialized.contains("前文"));
        assert!(!serialized.contains("image_url"));
        if protocol == "ollama" {
            let schema = &body["response_format"]["json_schema"]["schema"]["properties"]["regions"];
            assert_eq!(schema["minItems"], 48);
            assert_eq!(schema["maxItems"], 48);
            assert_eq!(
                schema["items"]["properties"]["id"]["enum"]
                    .as_array()
                    .unwrap()
                    .len(),
                48
            );
        }
    }
}

#[tokio::test]
async fn numbered_reply_failures_are_atomic_and_never_trigger_a_repair_request() {
    for (text, finish) in [
        ("[1] 第一条", "stop"),
        ("[1] 第一条\n[2]", "stop"),
        ("[1] 第一条\n[2] 第二条", "length"),
        ("[2] 第二条\n[1] 第一条", "stop"),
    ] {
        let (server, p) = profile("ollama").await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(response("ollama", text, finish)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let regions = [
            source("原文"),
            Region {
                id: "second".into(),
                ..source("次の文")
            },
        ];
        assert!(
            providers::llm(&p, "", &TranslationSettings::default(), &regions, None, "")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn enabled_simple_mode_does_not_change_direct_vision_transcription_or_extraction() {
    let (server, p) = profile("ollama").await;
    Mock::given(path("/v1/chat/completions")).respond_with(|r: &wiremock::Request| {
        let body:Value=serde_json::from_slice(&r.body).unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(),2);
        let system=body["messages"][0]["content"].as_str().unwrap();
        let content=if system.starts_with("识别并提取") {
            assert!(body.get("response_format").is_none()); "NONE".to_owned()
        } else {
            assert_eq!(body["response_format"]["type"],"json_schema");
            json!({"regions":[{"id":"private-region-id","source":"画像文字","target":if system.starts_with("Transcribe"){ "" } else {"图片文字"}}]}).to_string()
        };
        ResponseTemplate::new(200).set_body_json(response("ollama",&content,"stop"))
    }).expect(3).mount(&server).await;
    let r = source("画像文字");
    let image = image::DynamicImage::new_rgb8(40, 40);
    let s = TranslationSettings {
        mode: "vision".into(),
        ..Default::default()
    };
    assert_eq!(
        providers::llm(&p, "", &s, std::slice::from_ref(&r), Some(&image), "")
            .await
            .unwrap()[0]
            .target,
        "图片文字"
    );
    assert_eq!(
        providers::transcribe(&p, "", &s, &r, &image).await.unwrap(),
        "画像文字"
    );
    assert!(
        providers::extract_glossary(&p, "", &s, &[r])
            .await
            .unwrap()
            .terms
            .is_empty()
    );
}

#[tokio::test]
async fn rejects_empty_oversized_and_truncated_final_but_does_not_parse_or_rewrite_text() {
    for (content, finish, succeeds) in [
        ("", "stop", false),
        ("  \n", "stop", false),
        ("incomplete", "length", false),
        (&"字".repeat(4097), "stop", false),
        ("{\"literal\":\"quoted source\"}", "stop", true),
        ("魔素", "stop", true),
        ("1. 中文\n2. 第二行", "stop", true),
    ] {
        let (server, p) = profile("ollama").await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(response("ollama", content, finish)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = providers::llm(
            &p,
            "",
            &TranslationSettings::default(),
            &[source("魔素")],
            None,
            "",
        )
        .await;
        assert_eq!(result.is_ok(), succeeds, "{finish}: {result:?}");
        if succeeds {
            assert_eq!(result.unwrap()[0].target, content);
        }
    }
    let p = ProviderProfile {
        simple_translation: true,
        endpoint: "http://127.0.0.1:1".into(),
        ..Default::default()
    };
    for text in [" ", "■■■■"] {
        assert!(
            providers::llm(
                &p,
                "",
                &TranslationSettings::default(),
                &[source(text)],
                None,
                ""
            )
            .await
            .unwrap()
            .is_empty()
        );
    }
    assert!(
        providers::llm(
            &p,
            "",
            &TranslationSettings::default(),
            &vec![source("a"); 49],
            None,
            ""
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("page batching limits")
    );
}

#[test]
fn cache_identity_uses_actual_task_and_keeps_extraction_independent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("page.png");
    image::RgbImage::new(50, 50).save(&file).unwrap();
    let page = umanga_core::documents::import(&[file.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let mut s = TranslationSettings::default();
    let mut p = ProviderProfile {
        simple_translation: false,
        ..Default::default()
    };
    let structured = store::cache_key(&page, &s, &p, "").unwrap();
    let extraction = instructions::extraction_identity(&p, &s).unwrap();
    p.simple_translation = true;
    let simple = store::cache_key(&page, &s, &p, "").unwrap();
    assert!(simple.starts_with("simple-text-v4:"));
    let book = dir.path().join("book.umanga");
    store::create(&book, "Cache versions", std::slice::from_ref(&page)).unwrap();
    let old_key = simple.replacen("simple-text-v4:", "simple-text-v3:", 1);
    store::cache_put(
        &book,
        &old_key,
        &[TranslationItem {
            id: page.id.clone(),
            source: "old".into(),
            target: "historical".into(),
            direction: String::new(),
        }],
    )
    .unwrap();
    assert!(store::cache_get(&book, &simple).unwrap().is_none());
    assert_eq!(
        store::cache_get(&book, &old_key).unwrap().unwrap()[0].target,
        "historical"
    );
    assert!(structured.starts_with("llm-text-batch-v1:"));
    assert_ne!(structured, simple);
    assert_eq!(
        extraction,
        instructions::extraction_identity(&p, &s).unwrap()
    );
    p.instructions.simple_text_translation = Some(instructions::default_template(
        Task::SimpleTextTranslation,
        "ja",
    ));
    assert_eq!(simple, store::cache_key(&page, &s, &p, "").unwrap());
    p.instructions.text_translation = Some("structured changed".into());
    assert_eq!(simple, store::cache_key(&page, &s, &p, "").unwrap());
    p.instructions.simple_text_translation = Some("simple changed".into());
    assert_ne!(simple, store::cache_key(&page, &s, &p, "").unwrap());
    assert_eq!(
        extraction,
        instructions::extraction_identity(&p, &s).unwrap()
    );
    s.mode = "vision".into();
    let vision = store::cache_key(&page, &s, &p, "").unwrap();
    p.simple_translation = false;
    p.instructions.simple_text_translation = Some("another".into());
    assert_eq!(vision, store::cache_key(&page, &s, &p, "").unwrap());
    p.simple_translation = true;
    s.auto_glossary = true;
    s.glossary_enabled = true;
    assert_eq!(
        instructions::translation_task(&p, &s),
        Task::VisionTranslation
    );
}
