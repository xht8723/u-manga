use serde_json::{Value, json};
#[path = "support/generation.rs"]
mod generation;
use umanga_core::{
    providers,
    thinking::{self, Policy},
    types::*,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[test]
fn metadata_never_calls_low_effort_off() {
    let caps = vec!["completion".into(), "thinking".into()];
    for (values, state, on, off) in [
        (
            json!([false, true]),
            "switchable",
            Some("low"),
            Some("none"),
        ),
        (
            json!(["low", "medium", "high"]),
            "fixed_on",
            Some("low"),
            None,
        ),
        (json!([false]), "fixed_off", None, Some("none")),
        (json!([]), "managed", None, None),
    ] {
        let policy = thinking::local(Some(&json!({"values":values})), &caps);
        assert_eq!(policy.state, state);
        assert_eq!(policy.on.as_ref().and_then(Value::as_str), on);
        assert_eq!(policy.off.as_ref().and_then(Value::as_str), off);
    }
    assert_eq!(thinking::local(None, &caps), Policy::default());
    let unknown = ProviderProfile {
        simple_translation: false,
        model: "custom-model".into(),
        ..Default::default()
    };
    assert_eq!(thinking::hosted(&unknown).state, "managed");
}

#[tokio::test]
async fn thinking_is_mapped_on_actual_protocol_requests() {
    for (protocol, model, request_path, pointer, on, off) in [
        (
            "openai",
            "deepseek-flash",
            "/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "openai",
            "custom-reasoner",
            "/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "openai",
            "gpt-5",
            "/chat/completions",
            "/reasoning_effort",
            json!("minimal"),
            json!("none"),
        ),
        (
            "openai",
            "gpt-4.1",
            "/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "responses",
            "gpt-5.1",
            "/responses",
            "/reasoning/effort",
            json!("low"),
            json!("none"),
        ),
        (
            "responses",
            "gpt-5.2-pro",
            "/responses",
            "/reasoning/effort",
            json!("medium"),
            json!("none"),
        ),
        (
            "responses",
            "custom-reasoner",
            "/responses",
            "/reasoning/effort",
            json!("low"),
            json!("none"),
        ),
        (
            "anthropic",
            "claude-sonnet-4-5",
            "/messages",
            "/thinking/budget_tokens",
            json!(1024),
            Value::Null,
        ),
        (
            "anthropic",
            "custom-reasoner",
            "/messages",
            "/thinking/budget_tokens",
            json!(1024),
            Value::Null,
        ),
        (
            "anthropic",
            "claude-3-haiku",
            "/messages",
            "/thinking/budget_tokens",
            json!(1024),
            Value::Null,
        ),
        (
            "gemini",
            "gemini-2.5-flash-lite",
            "/models/gemini-2.5-flash-lite:generateContent",
            "/generationConfig/thinkingConfig/thinkingBudget",
            json!(512),
            json!(0),
        ),
        (
            "gemini",
            "gemini-2.5-pro",
            "/models/gemini-2.5-pro:generateContent",
            "/generationConfig/thinkingConfig/thinkingBudget",
            json!(128),
            json!(0),
        ),
        (
            "gemini",
            "gemini-3-pro-preview",
            "/models/gemini-3-pro-preview:generateContent",
            "/generationConfig/thinkingConfig",
            json!({"thinkingLevel":"LOW", "includeThoughts":true}),
            json!({"thinkingBudget":0, "includeThoughts":true}),
        ),
        (
            "gemini",
            "custom-reasoner",
            "/models/custom-reasoner:generateContent",
            "/generationConfig/thinkingConfig/thinkingBudget",
            json!(1000),
            json!(0),
        ),
        (
            "gemini",
            "gemini-2.0-flash",
            "/models/gemini-2.0-flash:generateContent",
            "/generationConfig/thinkingConfig/thinkingBudget",
            json!(1000),
            json!(0),
        ),
        (
            "ollama",
            "local-test",
            "/v1/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "ollama",
            "local-no-metadata",
            "/v1/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "ollama",
            "local-required",
            "/v1/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
        (
            "ollama",
            "local-no-thinking",
            "/v1/chat/completions",
            "/reasoning_effort",
            json!("low"),
            json!("none"),
        ),
    ] {
        let server = MockServer::builder().start().await;
        let translation = r#"{"regions":[{"id":"test","source":"こんにちは","target":"你好"}]}"#;
        if protocol == "ollama" {
            Mock::given(path("/api/tags"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"models":[{"name":model,"digest":"thinking-test","size":42}]}),
                ))
                .mount(&server)
                .await;
            let metadata = match model {
                "local-no-metadata" => json!({"capabilities":["completion","thinking"]}),
                "local-required" => {
                    json!({"capabilities":["completion","thinking"],"thinking":{"values":["low","medium","high"]}})
                }
                "local-no-thinking" => json!({"capabilities":["completion"]}),
                _ => {
                    json!({"capabilities":["completion","thinking"],"thinking":{"values":[false,true]}})
                }
            };
            Mock::given(path("/api/show"))
                .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
                .mount(&server)
                .await;
        }
        let response = match protocol {
            "anthropic" => {
                json!({"id":"test","type":"message","role":"assistant","model":model,"stop_reason":"end_turn","content":[{"type":"text","text":translation}],"usage":{"input_tokens":1,"output_tokens":1}})
            }
            "gemini" => {
                json!({"modelVersion":model,"candidates":[{"content":{"parts":[{"text":translation}]},"finishReason":"STOP"}],"usageMetadata":{}})
            }
            "responses" => {
                json!({"id":"test","model":model,"status":"completed","output":[{"id":"msg","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":translation,"annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
            }
            _ => {
                json!({"id":"test","model":model,"choices":[{"index":0,"message":{"role":"assistant","content":translation,"reasoning_content":"not a translation"},"finish_reason":"stop"}]})
            }
        };
        Mock::given(method("POST"))
            .and(path(request_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        let mut p = ProviderProfile {
            simple_translation: false,
            protocol: protocol.into(),
            endpoint: server.uri(),
            model: model.into(),
            instructions: umanga_core::instructions::Overrides {
                text_translation: Some("ADAPTER-CUSTOM-GUIDANCE".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        if protocol == "ollama" {
            // MockServer can reuse a port still present in the metadata cache.
            umanga_core::ollama::details(&p.endpoint, &p.model, true).await;
        }
        for (enabled, expected) in [(false, off), (true, on)] {
            p.thinking = enabled;
            p.thinking_policy = Some(thinking::resolve(&p).await);
            let result = providers::llm(
                &p,
                "test-only",
                &TranslationSettings {
                    glossary_enabled: true,
                    glossary: vec![GlossaryEntry {
                        source: "PROMPT-TERM".into(),
                        target: "词条".into(),
                    }],
                    ..Default::default()
                },
                &[Region {
                    id: "test".into(),
                    source: "こんにちは".into(),
                    ..Default::default()
                }],
                None,
                "Page 1:\nSOURCE-CONTEXT-SENTINEL",
            )
            .await;
            assert!(result.is_ok(), "{protocol}: {result:?}");
            assert_eq!(result.unwrap()[0].target, "你好");
            let requests = server.received_requests().await.unwrap();
            let body: Value = serde_json::from_slice(
                &requests
                    .iter()
                    .rev()
                    .find(|r| r.url.path() == request_path)
                    .unwrap()
                    .body,
            )
            .unwrap();
            assert_eq!(
                body.pointer(pointer).cloned().unwrap_or(Value::Null),
                expected,
                "{protocol} thinking={enabled}"
            );
            generation::assert_service_defaults(&body, protocol);
            let serialized = body.to_string();
            assert_eq!(serialized.matches("ADAPTER-CUSTOM-GUIDANCE").count(), 1);
            assert!(!serialized.contains("Use natural dialogue"));
            assert!(
                serialized.contains("SOURCE-CONTEXT-SENTINEL"),
                "{protocol}: context lost"
            );
            assert!(
                serialized.contains("PROMPT-TERM"),
                "{protocol}: glossary lost"
            );
            assert!(
                !serialized.contains("Copy each supplied source"),
                "{protocol}: Hidden prompt contract restored"
            );
            assert!(serialized.contains("ADAPTER-CUSTOM-GUIDANCE"));
            if protocol == "ollama" {
                assert_eq!(body["response_format"]["type"], "json_schema");
            }
            if protocol == "anthropic" && !enabled {
                assert!(
                    body.get("thinking").is_none(),
                    "Anthropic Off must not enable thinking"
                );
            }
        }
    }
}

#[tokio::test]
async fn new_request_policy_overrides_capabilities_and_changes_cache_identity() {
    for protocol in ["openai", "responses", "anthropic", "gemini"] {
        for model in [
            "gpt-5.2-pro",
            "gpt-4.1",
            "gemini-2.5-pro",
            "custom-reasoner",
        ] {
            let p = ProviderProfile {
                simple_translation: false,
                protocol: protocol.into(),
                model: model.into(),
                ..Default::default()
            };
            let capability = thinking::capability(&p).await;
            let request = thinking::resolve(&p).await;
            assert_eq!(request.state, "switchable");
            assert_eq!(
                request.off,
                Some(if protocol == "gemini" {
                    json!(0)
                } else {
                    json!("none")
                })
            );
            assert!(request.effort(true).is_some());
            assert_ne!(
                request, capability,
                "Captured policies must distinguish the new request behavior"
            );
            assert_eq!(
                thinking::capability(&p).await,
                capability,
                "Capability discovery is still advisory"
            );
        }
    }
}

#[tokio::test]
async fn rejected_thinking_choice_is_not_retried_with_another_effort() {
    for protocol in ["openai", "ollama"] {
        let server = MockServer::builder().start().await;
        let request_path = if protocol == "ollama" {
            "/v1/chat/completions"
        } else {
            "/chat/completions"
        };
        if protocol == "ollama" {
            Mock::given(path("/api/tags"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"models":[{"name":"mandatory","digest":"mandatory","size":42}]}),
                ))
                .mount(&server)
                .await;
            Mock::given(path("/api/show"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion","thinking"],"thinking":{"values":["low","high"]}})))
                .mount(&server).await;
        }
        Mock::given(method("POST"))
            .and(path(request_path))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"reasoning_effort none is not supported; PRIVATE-PAGE-TEXT", "type":"invalid_request_error"}})))
            .expect(1)
            .mount(&server).await;
        let p = ProviderProfile {
            simple_translation: false,
            protocol: protocol.into(),
            endpoint: server.uri(),
            model: "mandatory".into(),
            thinking: false,
            ..Default::default()
        };
        if protocol == "ollama" {
            umanga_core::ollama::details(&p.endpoint, &p.model, true).await;
        }
        let error = providers::llm(
            &p,
            "test-only",
            &TranslationSettings::default(),
            &[Region {
                id: "test".into(),
                source: "hello".into(),
                ..Default::default()
            }],
            None,
            "",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("Thinking setting"), "{error}");
        assert!(!error.contains("PRIVATE-PAGE-TEXT"));
        let requests = server.received_requests().await.unwrap();
        let chats: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == request_path)
            .collect();
        assert_eq!(chats.len(), 1);
        let body: Value = serde_json::from_slice(&chats[0].body).unwrap();
        assert_eq!(body["reasoning_effort"], "none");
    }
}

#[test]
fn persisted_thinking_and_captured_policy_are_independent() {
    let mut p = ProviderProfile::default();
    assert!(!p.thinking);
    p.thinking_policy = Some(thinking::hosted(&p));
    let captured = p.clone();
    p.thinking = true;
    assert!(!captured.thinking);
    let reopened: ProviderProfile =
        serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(reopened.thinking);
    assert_eq!(reopened.thinking_policy, p.thinking_policy);
    let page = Page {
        id: "cache-test".into(),
        number: 0,
        name: "sample".into(),
        source: Source {
            path: "synthetic.png".into(),
            kind: "image".into(),
            entry: None,
            index: 0,
        },
        width: 100,
        height: 100,
        pdf_points: None,
        source_stamp: String::new(),
        fingerprint: "pixels".into(),
        revision: 0,
        regions: vec![],
        rendered: None,
        background: None,
        cleanup: None,
        status: "new".into(),
        error: None,
    };
    let settings = TranslationSettings::default();
    assert_ne!(
        umanga_core::store::cache_key(&page, &settings, &captured, "").unwrap(),
        umanga_core::store::cache_key(&page, &settings, &p, "").unwrap()
    );
    p.thinking = false;
    p.thinking_policy = Some(Policy::default());
    assert_ne!(
        umanga_core::store::cache_key(&page, &settings, &captured, "").unwrap(),
        umanga_core::store::cache_key(&page, &settings, &p, "").unwrap()
    );
}

#[test]
fn sibling_models_do_not_inherit_unsupported_reasoning_controls() {
    for (model, state, effort) in [
        ("gpt-5.2-2025-12-11", "switchable", Some("none")),
        ("gpt-5.2-pro", "fixed_on", Some("medium")),
        ("gpt-5-pro", "fixed_on", Some("high")),
        ("gpt-5", "fixed_on", Some("minimal")),
        ("gpt-5.2-codex", "managed", None),
        ("custom/gpt-5.2", "managed", None),
    ] {
        let p = ProviderProfile {
            simple_translation: false,
            protocol: "responses".into(),
            model: model.into(),
            ..Default::default()
        };
        let policy = thinking::hosted(&p);
        assert_eq!(policy.state, state, "{model}");
        assert_eq!(
            policy.effort(false).and_then(|v| v.as_keyword()),
            effort,
            "{model}"
        );
    }
}
