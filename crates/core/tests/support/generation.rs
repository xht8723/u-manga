use serde_json::Value;

/// Inspect actual transport fields, not arbitrary dialogue or schema properties.
pub fn assert_service_defaults(body: &Value, protocol: &str) {
    for key in [
        "temperature",
        "top_p",
        "top_k",
        "seed",
        "presence_penalty",
        "frequency_penalty",
        "repeat_penalty",
        "min_p",
        "typical_p",
        "tfs_z",
        "mirostat",
        "options",
        "num_ctx",
        "num_predict",
        "stop",
        "stop_sequences",
        "max_completion_tokens",
        "max_output_tokens",
    ] {
        assert!(body.get(key).is_none(), "{protocol} overrides {key}");
    }
    if protocol == "anthropic" {
        // Anthropic requires this field; the pinned adapter supplies its default.
        assert!(body["max_tokens"].as_u64().is_some_and(|n| n > 0));
    } else {
        assert!(
            body.get("max_tokens").is_none(),
            "{protocol} overrides max_tokens"
        );
    }
    for key in [
        "temperature",
        "topP",
        "topK",
        "maxOutputTokens",
        "stopSequences",
    ] {
        assert!(
            body["generationConfig"].get(key).is_none(),
            "{protocol} overrides {key}"
        );
    }
}
