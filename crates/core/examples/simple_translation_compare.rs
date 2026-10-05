//! Explicit, loopback-only synthetic comparison; never reads manga or saved settings.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
use umanga_core::{
    instructions::{self, Task},
    providers,
    types::*,
};

#[tokio::main]
async fn main() -> Result<()> {
    ensure!(
        std::env::args().any(|a| a == "--run"),
        "Pass --run to send synthetic requests to installed local Ollama"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root
        .join("test-output/simple-translation")
        .join(chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string());
    std::fs::create_dir_all(&out)?;
    let mut p = ProviderProfile {
        protocol: "ollama".into(),
        endpoint: "http://127.0.0.1:11434".into(),
        model: "hy-mt2-7b:q8_0".into(),
        simple_translation: true,
        rate_limit: 20.,
        ..Default::default()
    };
    let mut s = TranslationSettings::default();
    let runtime: Value = reqwest::get(format!("{}/api/version", p.endpoint))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tags: Value = reqwest::get(format!("{}/api/tags", p.endpoint))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let model = tags["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == p.model)
        .ok_or_else(|| {
            anyhow::anyhow!("Required local model is not installed; no download attempted")
        })?
        .clone();
    p.thinking_policy = Some(umanga_core::thinking::resolve(&p).await);
    let warmup = Instant::now();
    let warmup_result = providers::llm(
        &p,
        "",
        &s,
        &[Region {
            source: "こんにちは".into(),
            ..Default::default()
        }],
        None,
        "",
    )
    .await;
    let warmup_ms = warmup.elapsed().as_millis();
    let mut records = Vec::new();
    let fixtures: Vec<Value> = serde_json::from_str(include_str!(
        "../../../benchmarks/raw-translation/cases.json"
    ))?;
    let mut cases = fixtures
        .into_iter()
        .filter(|c| ["r01", "r09", "r14", "r20", "r21", "r23"].contains(&c["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    cases.extend([
        json!({"id":"multiline","source":"待って！\n\nまだ話は終わってない。","expected":"Wait; the conversation has not ended. Preserve both sentences in one region."}),
        json!({"id":"context","source":"アリスはもう着いたかな？","expected":"Wonder whether Alice has arrived; use glossary 爱丽丝 and preserve uncertainty."}),
        json!({"id":"redaction","source":"■■■■","expected":"Simple mode skips this without a translation request; retain original."}),
    ]);
    for round in 0..2 {
        for case in &cases {
            for simple in if round == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                p.simple_translation = simple;
                s.glossary_enabled = case["id"] == "context";
                s.glossary = if s.glossary_enabled {
                    vec![GlossaryEntry {
                        source: "アリス".into(),
                        target: "爱丽丝".into(),
                    }]
                } else {
                    vec![]
                };
                let context = if s.glossary_enabled {
                    "Page 12:\nアリスは先に城へ向かった。"
                } else {
                    ""
                };
                let region = Region {
                    id: case["id"].as_str().unwrap().into(),
                    source: case["source"].as_str().unwrap().into(),
                    ..Default::default()
                };
                let schema =
                    (!simple).then(|| providers::translation_schema(std::slice::from_ref(&region)));
                let prompt = if simple {
                    instructions::simple_prompt(
                        &p.instructions,
                        &s,
                        context,
                        std::slice::from_ref(&region),
                    )?
                } else {
                    instructions::compose(
                        &p.instructions,
                        &s,
                        Task::TextTranslation,
                        context,
                        schema.as_ref(),
                    )?
                };
                let user = if simple {
                    prompt.user_preamble.clone()
                } else {
                    format!(
                        "{}\n\n{}",
                        prompt.user_preamble,
                        umanga_core::prompts::region_label(&region)?
                    )
                };
                let start = Instant::now();
                let result =
                    providers::llm(&p, "", &s, std::slice::from_ref(&region), None, context).await;
                records.push(json!({"round":round+1,"case":case,"mode":if simple {"simple"} else {"structured"},"system":prompt.system,"user":user,"schema":schema,"elapsedMs":start.elapsed().as_millis(),"result":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{e:#}")),"skippedLocally":simple&&!instructions::simple_source_readable(&region.source)}));
                let report = json!({"model":model,"runtime":runtime,"thinking":p.thinking,"thinkingPolicy":p.thinking_policy,"generationOverrides":false,"warmupMs":warmup_ms,"warmupSucceeded":warmup_result.is_ok(),"records":records});
                std::fs::write(
                    out.join("results.json"),
                    serde_json::to_vec_pretty(&report)?,
                )?;
                println!(
                    "round={} case={} simple={} ok={}",
                    round + 1,
                    case["id"],
                    simple,
                    result.is_ok()
                );
            }
        }
    }
    println!("Evidence: {}", out.display());
    Ok(())
}
