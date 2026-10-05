//! Explicit local-only synthetic smoke; never reads credentials or manga files.
use anyhow::{Result, ensure};
use serde_json::json;
use umanga_core::{setup, thinking, types::*};

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join("test-output/lettering-thinking-navigation");
    std::fs::create_dir_all(&output)?;
    let mut p = ProviderProfile {
        protocol: "ollama".into(),
        endpoint: "http://localhost:11434".into(),
        model: "huihui_ai/qwen3.5-abliterated:4b".into(),
        ..Default::default()
    };
    let policy = thinking::resolve(&p).await;
    ensure!(
        policy.state == "switchable",
        "Resolved Thinking preference must have an on/off request policy"
    );
    let mut results = vec![];
    for mode in ["local", "vision"] {
        for enabled in [false, true] {
            p.thinking = enabled;
            p.thinking_policy = Some(policy.clone());
            let settings = TranslationSettings {
                mode: mode.into(),
                ..Default::default()
            };
            let result = setup::test_service(&p, "", &settings).await;
            match result {
                Ok(result) => {
                    println!(
                        "{mode} thinking={enabled}: {:.2}s",
                        result.elapsed_ms as f64 / 1000.
                    );
                    results.push(json!({"mode":mode,"thinking":enabled,"ok":true,"result":result}));
                }
                Err(error) => {
                    println!("{mode} thinking={enabled}: failed: {error}");
                    results.push(json!({"mode":mode,"thinking":enabled,"ok":false,"error":error.to_string()}));
                }
            }
            std::fs::write(
                output.join("ollama-smoke.json"),
                serde_json::to_vec_pretty(
                    &json!({"model":p.model,"policy":policy,"syntheticOnly":true,"results":results}),
                )?,
            )?;
        }
    }
    ensure!(
        results.iter().all(|v| v["ok"] == true),
        "One or more synthetic cases failed; see ollama-smoke.json"
    );
    Ok(())
}
