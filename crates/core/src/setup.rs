//! Shared readiness for setup, settings and Jobs submission.
use crate::{
    models::{self, ModelLocations, VerificationCache},
    providers,
    types::*,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SOURCES: &[&str] = &[
    "ja", "zh-Hans", "zh-Hant", "en", "ko", "fr", "de", "es", "ar", "hi", "ru", "th", "vi", "ta",
];
pub const TARGETS: &[&str] = &[
    "zh-Hans", "zh-Hant", "en", "ja", "ko", "fr", "de", "es", "ar", "hi",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub code: String,
    pub section: String,
    #[serde(with = "crate::ui_message::text")]
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackStatus {
    pub id: String,
    pub name: String,
    pub distribution: String,
    pub bytes: u64,
    pub status: String,
    pub path: String,
    pub required: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub ready: bool,
    pub issues: Vec<Issue>,
    pub packs: Vec<PackStatus>,
    pub credential_stored: bool,
    pub credential_required: bool,
    pub ollama: Option<crate::ollama::ModelStatus>,
    pub destination: String,
    pub advancement: std::collections::BTreeMap<String, crate::requirements::Availability>,
}
fn issue(issues: &mut Vec<Issue>, code: &str, section: &str, message: impl Into<String>) {
    issues.push(Issue {
        code: code.into(),
        section: section.into(),
        message: message.into(),
    });
}
pub fn required_packs(s: &TranslationSettings, catalog: &[ModelPack]) -> Vec<String> {
    let mut ids = vec!["rtdetr_int8".into()];
    if s.mode == "local" {
        if s.ocr == "manga" {
            ids.push("manga_ocr_onnx".into());
        }
        if s.ocr == "pp" {
            ids.push("pp_det".into());
            if let Some(pack) = catalog
                .iter()
                .find(|p| p.kind == "recognizer" && p.languages.contains(&s.source_language))
            {
                ids.push(pack.id.clone());
            }
        }
    }
    if let Some(pack) = s.cleanup.method.pack() {
        ids.push(pack.into());
    }
    ids
}
pub fn configuration(
    s: &TranslationSettings,
    p: Option<&ProviderProfile>,
    credential: bool,
) -> Vec<Issue> {
    let mut issues = vec![];
    if !SOURCES.contains(&s.source_language.as_str())
        || !TARGETS.contains(&s.target_language.as_str())
    {
        issue(
            &mut issues,
            "languages",
            "pipeline",
            "Select supported source and target languages.",
        );
    }
    if !["vision", "local"].contains(&s.mode.as_str()) {
        issue(
            &mut issues,
            "mode",
            "pipeline",
            "Choose Vision or Local OCR.",
        );
    }
    if s.mode == "local" {
        if s.ocr == "manga" && s.source_language != "ja" {
            issue(
                &mut issues,
                "ocr_language",
                "pipeline",
                "Manga OCR reads Japanese only. Choose PP-OCRv5 for this source language.",
            );
        } else if !["manga", "pp"].contains(&s.ocr.as_str()) {
            issue(
                &mut issues,
                "ocr",
                "pipeline",
                "Choose Manga OCR or PP-OCRv5.",
            );
        }
        if !["cpu", "directml"].contains(&s.device.as_str()) {
            issue(
                &mut issues,
                "device",
                "pipeline",
                "Choose CPU or DirectML for local recognition.",
            );
        }
    }
    if s.context_pages > 20 {
        issue(&mut issues, "limits", "pipeline", "Use 0–20 context pages.");
    }
    let Some(p) = p else {
        if s.mode == "local" && s.provider_id.is_empty() {
            return issues;
        }
        issue(
            &mut issues,
            "service",
            "services",
            "Select and configure a translation service.",
        );
        return issues;
    };
    if !["llm", "google", "microsoft", "deepl", "baidu"].contains(&p.service.as_str()) {
        issue(
            &mut issues,
            "service_type",
            "services",
            "Select a supported translation service.",
        );
    }
    if s.mode == "vision" && (p.service != "llm" || (!providers::is_ollama(p) && !p.vision)) {
        issue(
            &mut issues,
            "vision",
            "services",
            "Vision requires an image-capable model. Select one or choose Local OCR.",
        );
    }
    if let Err(e) = p.instructions.validate() {
        issue(&mut issues, "instructions", "services", e.to_string());
    }
    if p.service == "llm" {
        if p.model.trim().is_empty() {
            issue(
                &mut issues,
                "model",
                "services",
                "Select or enter a model ID.",
            );
        }
        if !["openai", "responses", "anthropic", "gemini", "ollama"].contains(&p.protocol.as_str())
        {
            issue(
                &mut issues,
                "protocol",
                "services",
                "Choose a supported API protocol.",
            );
        }
    }
    if let Err(e) = providers::credential_scope(p) {
        issue(
            &mut issues,
            "endpoint",
            "services",
            format!("Check the service endpoint: {e}"),
        );
    }
    if p.service == "baidu" && p.app_id.trim().is_empty() {
        issue(
            &mut issues,
            "app_id",
            "services",
            "Enter the Baidu application ID.",
        );
    }
    if !p.rate_limit.is_finite() || !(0.1..=20.).contains(&p.rate_limit) {
        issue(
            &mut issues,
            "rate",
            "services",
            "Set a request rate between 0.1 and 20 per second.",
        );
    }
    if providers::credential_required(p) && !credential {
        issue(
            &mut issues,
            "credential",
            "services",
            "Enter an API key for the selected service and endpoint.",
        );
    }
    issues
}
pub fn destination(p: &ProviderProfile) -> String {
    if providers::is_ollama(p) {
        return crate::ollama::destination(&p.endpoint);
    }
    match p.service.as_str() {
        "google" => "https://translation.googleapis.com/".into(),
        "microsoft" => "https://api.cognitive.microsofttranslator.com/".into(),
        "deepl" => "https://api.deepl.com/ (api-free.deepl.com for Free keys)".into(),
        "baidu" => "https://fanyi-api.baidu.com/".into(),
        _ => providers::endpoint(p),
    }
}
pub fn readiness(
    settings: &AppSettings,
    locations: &ModelLocations,
    cache: &mut VerificationCache,
    credential: bool,
    force: bool,
) -> Result<Readiness> {
    let s = &settings.translation;
    let provider = settings.providers.iter().find(|p| p.id == s.provider_id);
    let mut issues = configuration(s, provider, credential);
    if !(1..=4).contains(&settings.concurrent_books) {
        issue(
            &mut issues,
            "limits",
            "services",
            "Use 1–4 concurrent books.",
        );
    }
    let root = Path::new(&settings.library_directory);
    if settings.library_directory.is_empty()
        || !root.is_dir()
        || std::fs::read_dir(root).is_err()
        || std::fs::metadata(root)
            .map(|m| !cfg!(windows) && m.permissions().readonly())
            .unwrap_or(true)
    {
        issue(
            &mut issues,
            "library",
            "library",
            "Choose an accessible, writable library folder.",
        );
    } else if root.join("library.sqlite").exists() {
        let valid = rusqlite::Connection::open_with_flags(
            root.join("library.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .and_then(|c| c.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)));
        if valid.ok() != Some(crate::store::FORMAT_VERSION) {
            issue(
                &mut issues,
                "library_format",
                "library",
                "Choose an empty folder or a library made with this book format.",
            );
        }
    }
    let catalog = models::catalog()?;
    let required = required_packs(s, &catalog);
    if s.mode == "local"
        && s.ocr == "pp"
        && !catalog
            .iter()
            .any(|p| p.kind == "recognizer" && p.languages.contains(&s.source_language))
    {
        issue(
            &mut issues,
            "ocr_pack",
            "pipeline",
            "No PP-OCRv5 recognition pack is available for this source language.",
        );
    }
    let packs = catalog.iter().map(|p| {
        let root = locations.root(p);
        let needed = required.contains(&p.id);
        let exists = !root.as_os_str().is_empty() && p.files.iter().all(|f| root.join(&p.id).join(&f.name).is_file());
        let valid = if needed || force { Some(cache.check(root, p, force).unwrap_or(false)) } else { cache.cached(root, p) };
        let status = if valid == Some(true) { "verified" } else if !exists { "missing" } else if valid == Some(false) { "corrupt" } else { "downloaded" };
        if needed && status != "verified" {
            let message = if p.distribution == "bundled" { "The included detector needs repair. Restart U-Manga to restore it from the executable.".into() }
            else if root.as_os_str().is_empty() { "Choose a library folder before downloading model packs.".into() }
            else { format!("Download or verify {} in Local models.", p.name) };
            issue(&mut issues, &format!("pack:{}", p.id), "models", message);
        }
        PackStatus { id:p.id.clone(), name:p.name.clone(), distribution:p.distribution.clone(), bytes:p.files.iter().map(|f|f.bytes).sum(), status:status.into(), path:root.to_string_lossy().into(), required:needed }
    }).collect();
    let mut report = Readiness {
        ready: issues.is_empty(),
        issues,
        packs,
        credential_stored: credential,
        credential_required: provider.is_some_and(providers::credential_required),
        ollama: None,
        destination: provider.map(destination).unwrap_or_default(),
        advancement: Default::default(),
    };
    update_advancement(&mut report, &s.mode);
    Ok(report)
}
/// Network readiness is asynchronous and separate from filesystem/model verification.
pub async fn check_service(report: &mut Readiness, settings: &AppSettings, force: bool) {
    report.issues.retain(|issue| issue.code != "ollama");
    let Some(p) = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .filter(|p| providers::is_ollama(p))
    else {
        return;
    };
    let status = crate::ollama::details(&p.endpoint, &p.model, force).await;
    let problem = status.message.clone().or_else(|| {
        status.model.as_ref().and_then(|m| {
            m.validate(settings.translation.mode == "vision")
                .err()
                .map(|e| e.to_string())
        })
    });
    if let Some(message) = problem {
        issue(&mut report.issues, "ollama", "services", message);
    }
    report.ollama = Some(status);
    report.ready = report.issues.is_empty();
    update_advancement(report, &settings.translation.mode);
}
fn setup_stages(report: &Readiness, mode: &str) -> Vec<&'static str> {
    let mut stages = vec!["library", "pipeline", "services"];
    if mode == "local"
        || report
            .packs
            .iter()
            .any(|p| p.required && p.distribution == "download")
    {
        stages.push("models");
    }
    stages.push("review");
    stages
}
fn advancement_issues(report: &Readiness, destination: &str) -> Vec<Issue> {
    report
        .issues
        .iter()
        .filter(|i| match destination {
            "library" => false,
            "pipeline" => i.section == "library" || i.code == "languages",
            "services" => {
                i.section == "library" || i.section == "pipeline" || i.code == "pack:rtdetr_int8"
            }
            "models" => i.section != "models" || i.code == "pack:rtdetr_int8",
            _ => true,
        })
        .cloned()
        .collect()
}
/// The UI and advancement command consume exactly the same prerequisites.
pub fn update_advancement(report: &mut Readiness, mode: &str) {
    let stages = setup_stages(report, mode);
    report.advancement = stages
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let next = stages.get(index + 1).copied().unwrap_or("review");
            let issues = advancement_issues(report, next);
            (
                (*step).into(),
                crate::requirements::Availability {
                    ready: issues.is_empty(),
                    issues,
                },
            )
        })
        .collect();
}
/// Validate the completed prerequisites, not the fields on the destination step.
pub fn validate_step(report: &Readiness, step: &str, complete: bool, mode: &str) -> Result<()> {
    let stages = setup_stages(report, mode);
    if !stages.contains(&step) {
        bail!("Unknown setup step");
    }
    if complete && step != "review" {
        bail!("Review setup before finishing");
    }
    let mut errors: Vec<_> = advancement_issues(report, step)
        .into_iter()
        .map(|i| i.message)
        .collect();
    errors.dedup();
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTest {
    pub elapsed_ms: u64,
    pub source: String,
    pub target: String,
}
pub async fn test_service(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
) -> Result<ServiceTest> {
    let problems: Vec<_> = configuration(s, Some(p), !key.trim().is_empty())
        .into_iter()
        .filter(|i| i.section == "services" || ["languages", "mode"].contains(&i.code.as_str()))
        .collect();
    if !problems.is_empty() {
        bail!(
            "{}",
            problems
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    // A synthetic 123 bitmap, never a user's manga page. No inference, cache or queue access.
    let mut sample = image::RgbImage::from_pixel(160, 80, image::Rgb([255, 255, 255]));
    for (n, glyph) in [
        ["010", "110", "010", "010", "111"],
        ["111", "001", "111", "100", "111"],
        ["111", "001", "111", "001", "111"],
    ]
    .iter()
    .enumerate()
    {
        for (y, row) in glyph.iter().enumerate() {
            for (x, bit) in row.bytes().enumerate() {
                if bit == b'1' {
                    for dy in 0..8 {
                        for dx in 0..8 {
                            sample.put_pixel(
                                24 + n as u32 * 40 + x as u32 * 8 + dx,
                                20 + y as u32 * 8 + dy,
                                image::Rgb([0, 0, 0]),
                            );
                        }
                    }
                }
            }
        }
    }
    let image = image::DynamicImage::ImageRgb8(sample);
    let mut regions = vec![Region {
        id: "setup-test".into(),
        bbox: [0., 0., 160., 80.],
        source: if s.mode == "vision" {
            String::new()
        } else {
            "123".into()
        },
        ..Default::default()
    }];
    let simple_text = s.mode != "vision" && crate::instructions::simple_translation(p, false);
    if simple_text {
        regions.push(Region {
            id: "setup-test-2".into(),
            source: "456".into(),
            ..Default::default()
        });
    }
    let mut settings = s.clone();
    settings.glossary_enabled = false;
    settings.context_pages = 0;
    let start = std::time::Instant::now();
    let items = if p.service == "llm" {
        providers::llm(
            p,
            key,
            &settings,
            &regions,
            if s.mode == "vision" {
                Some(&image)
            } else {
                None
            },
            "",
        )
        .await?
    } else {
        providers::conventional(p, key, &settings, &regions, "").await?
    };
    if simple_text {
        anyhow::ensure!(
            regions.iter().all(|r| items
                .iter()
                .any(|i| i.id == r.id && !i.target.trim().is_empty())),
            "The service returned no translation for the test sample."
        );
        return Ok(ServiceTest {
            elapsed_ms: start.elapsed().as_millis() as u64,
            source: regions
                .iter()
                .map(|r| r.source.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            target: items
                .iter()
                .map(|i| i.target.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }
    let Some(item) = items
        .iter()
        .find(|i| i.id == "setup-test" && !i.target.trim().is_empty())
    else {
        bail!("The service returned no translation for the test sample.");
    };
    Ok(ServiceTest {
        elapsed_ms: start.elapsed().as_millis() as u64,
        source: "123".into(),
        target: item.target.clone(),
    })
}
