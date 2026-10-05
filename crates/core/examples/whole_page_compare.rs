//! Explicit synthetic experiment using production prompts, transport and result validation.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
use umanga_core::{
    instructions::{self, Task},
    providers, text_batches,
    types::*,
};

#[tokio::main]
async fn main() -> Result<()> {
    ensure!(
        std::env::args().any(|a| a == "--run"),
        "Pass --run for local synthetic requests"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let endpoint =
        std::env::var("U_MANGA_BENCH_ENDPOINT").unwrap_or("http://127.0.0.1:11434".into());
    ensure!(
        url::Url::parse(&endpoint)?.host_str() == Some("127.0.0.1"),
        "Only loopback is allowed"
    );
    let run = std::env::var("U_MANGA_BENCH_RUN")
        .unwrap_or_else(|_| chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string());
    ensure!(
        !run.is_empty() && run.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "Use a simple benchmark run name"
    );
    let out = root.join("test-output/whole-page-production").join(run);
    std::fs::create_dir_all(&out)?;
    let mut provider = ProviderProfile {
        protocol: "ollama".into(),
        endpoint,
        model: "hy-mt2-7b:q8_0".into(),
        simple_translation: true,
        rate_limit: 100.,
        ..Default::default()
    };
    let runtime: Value = reqwest::get(format!("{}/api/version", provider.endpoint))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let models: Value = reqwest::get(format!("{}/api/tags", provider.endpoint))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let model = models["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == provider.model)
        .ok_or_else(|| {
            anyhow::anyhow!("Required model is not installed; downloads are not allowed")
        })?
        .clone();
    provider.thinking_policy = Some(umanga_core::thinking::resolve(&provider).await);
    let warm = Region {
        source: "こんにちは。".into(),
        ..Default::default()
    };
    let started = Instant::now();
    providers::llm(
        &provider,
        "",
        &TranslationSettings::default(),
        &[warm],
        None,
        "",
    )
    .await?;
    let warmup_ms = started.elapsed().as_millis();
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../benchmarks/simple-batch-retest/cases.json"
    ))?;
    let mut records = Vec::new();
    for case in cases.iter().filter(|c| {
        ["hospital", "glasses", "glossary", "multiline", "literal"]
            .contains(&c["id"].as_str().unwrap())
    }) {
        let settings = TranslationSettings {
            glossary_enabled: case.get("glossary").is_some(),
            glossary: case
                .get("glossary")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default(),
            ..Default::default()
        };
        let context = case
            .get("context")
            .map(|v| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        format!(
                            "Page {}:\n{}",
                            p["page"],
                            p["sources"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|s| s.as_str().unwrap())
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default();
        let regions: Vec<_> = case["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| Region {
                id: r["id"].as_str().unwrap().into(),
                source: r["source"].as_str().unwrap().into(),
                ..Default::default()
            })
            .collect();
        for mode in ["single", "simple-page", "structured-page"] {
            provider.simple_translation = mode != "structured-page";
            let batches = if mode == "single" {
                regions.chunks(1).collect()
            } else {
                text_batches::plan(&regions, provider.simple_translation)?
            };
            for (index, batch) in batches.into_iter().enumerate() {
                let (system, user, schema) = if provider.simple_translation {
                    let prompt = instructions::simple_prompt(
                        &provider.instructions,
                        &settings,
                        &context,
                        batch,
                    )?;
                    (prompt.system, prompt.user_preamble, None)
                } else {
                    let schema = providers::translation_schema(batch);
                    let prompt = instructions::compose(
                        &provider.instructions,
                        &settings,
                        Task::TextTranslation,
                        &context,
                        Some(&schema),
                    )?;
                    (
                        prompt.system,
                        format!(
                            "{}\n{}",
                            prompt.user_preamble,
                            text_batches::current_text(batch, false)?
                        ),
                        Some(schema),
                    )
                };
                let started = Instant::now();
                let result = providers::llm(&provider, "", &settings, batch, None, &context).await;
                let elapsed = started.elapsed().as_millis();
                records.push(json!({"case":case["id"],"mode":mode,"batch":index+1,"regionCount":batch.len(),"regions":batch,"system":system,"user":user,"schema":schema,"elapsedMs":elapsed,"result":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))}));
                std::fs::write(
                    out.join("results.json"),
                    serde_json::to_vec_pretty(
                        &json!({"model":model,"runtime":runtime,"warmupMs":warmup_ms,"thinkingPolicy":provider.thinking_policy,"generationOverrides":false,"cases":cases,"records":records}),
                    )?,
                )?;
                println!(
                    "case={} mode={mode} batch={} regions={} ok={} elapsed={elapsed}ms",
                    case["id"],
                    index + 1,
                    batch.len(),
                    result.is_ok()
                );
            }
        }
    }
    Ok(())
}
