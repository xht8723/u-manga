pub use crate::inpainting_types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub path: String,
    pub kind: String,
    pub entry: Option<String>,
    pub index: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub font: Option<String>,
    pub size: Option<f32>,
    pub color: String,
    pub fill: String,
    pub line_gap: f32,
    pub outline_enabled: bool,
    pub outline_width_percent: f32,
    pub outline_color: String,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: None,
            size: None,
            color: "#202020".into(),
            fill: "#ffffff".into(),
            line_gap: 0.2,
            outline_enabled: false,
            outline_width_percent: 8.,
            outline_color: "#ffffff".into(),
        }
    }
}
impl TextStyle {
    /// Detection-time initialization only. Later geometry/rendering never resets it.
    pub fn detected(kind: &str, has_bubble: bool) -> Self {
        Self {
            outline_enabled: !has_bubble && matches!(kind, "free" | "free_text"),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub id: String,
    pub bbox: [f32; 4],
    pub bubble: Option<[f32; 4]>,
    pub kind: String,
    pub score: f32,
    pub source: String,
    pub target: String,
    pub direction: String,
    pub style: TextStyle,
    pub allow_fill: bool,
    pub overlay_only: bool,
    pub prepared: bool,
    #[serde(with = "crate::ui_message::optional")]
    pub review: Option<String>,
}
impl Default for Region {
    fn default() -> Self {
        Self {
            id: uid(),
            bbox: [0.; 4],
            bubble: None,
            kind: "text".into(),
            score: 1.,
            source: String::new(),
            target: String::new(),
            direction: "auto".into(),
            style: TextStyle::default(),
            allow_fill: false,
            overlay_only: false,
            prepared: false,
            review: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub number: usize,
    pub name: String,
    pub source: Source,
    pub width: u32,
    pub height: u32,
    pub pdf_points: Option<[f32; 2]>,
    pub source_stamp: String,
    pub fingerprint: String,
    pub revision: u64,
    pub regions: Vec<Region>,
    pub rendered: Option<String>,
    pub background: Option<String>,
    pub cleanup: Option<PageCleanup>,
    pub status: String,
    #[serde(with = "crate::ui_message::optional_error")]
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSettings {
    pub source_language: String,
    pub target_language: String,
    pub mode: String,
    pub ocr: String,
    pub device: String,
    pub provider_id: String,
    pub glossary_enabled: bool,
    pub context_pages: usize,
    pub glossary: Vec<GlossaryEntry>,
    pub deepl_glossary_id: String,
    pub auto_glossary: bool,
    pub cleanup: CleanupSettings,
}
impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            source_language: "ja".into(),
            target_language: "zh-Hans".into(),
            mode: "local".into(),
            ocr: "manga".into(),
            device: "directml".into(),
            provider_id: String::new(),
            glossary_enabled: false,
            context_pages: 0,
            glossary: vec![],
            deepl_glossary_id: String::new(),
            auto_glossary: false,
            cleanup: CleanupSettings::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    pub simple_translation: bool,
    pub instructions: crate::instructions::Overrides,
    pub thinking: bool,
    pub thinking_policy: Option<crate::thinking::Policy>,
    pub id: String,
    pub name: String,
    pub service: String,
    pub protocol: String,
    pub endpoint: String,
    pub model: String,
    pub vision: bool,
    pub region: String,
    pub app_id: String,
    pub rate_limit: f32,
}
impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            instructions: Default::default(),
            simple_translation: true,
            thinking: false,
            thinking_policy: None,
            id: uid(),
            name: "DeepSeek".into(),
            service: "llm".into(),
            protocol: "openai".into(),
            endpoint: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            vision: true,
            region: String::new(),
            app_id: String::new(),
            rate_limit: 1.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub id: String,
    pub title: String,
    pub settings: TranslationSettings,
    pub pages: Vec<Page>,
    pub omitted_page_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub project: String,
    pub page_id: String,
    pub status: crate::job_state::JobStatus,
    pub stage: String,
    #[serde(with = "crate::ui_message::text")]
    pub stage_detail: String,
    #[serde(with = "crate::ui_message::optional_error")]
    pub error: Option<String>,
    pub created: String,
    pub elapsed_ms: u64,
    pub device: String,
    pub settings: TranslationSettings,
    pub provider: ProviderProfile,
    pub kind: JobKind,
    pub cleanup_model: Option<CleanupMethod>,
    pub model_directory: String,
    pub steps: Vec<JobStep>,
    pub page_revision: Option<u64>,
    pub fresh: bool,
    pub glossary_checkpoint: Option<crate::glossary::ExtractionCheckpoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStep {
    pub stage: String,
    pub status: String,
    #[serde(with = "crate::ui_message::text")]
    pub detail: String,
    #[serde(with = "crate::ui_message::optional_error")]
    pub error: Option<String>,
}
impl Job {
    pub fn detects_glossary(&self) -> bool {
        self.kind == JobKind::Translation
            && self.provider.service == "llm"
            && self.settings.glossary_enabled
            && self.settings.auto_glossary
    }
    pub fn record_step(&mut self, stage: &str, status: &str, detail: &str, error: Option<String>) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.stage == stage) {
            step.status = status.into();
            step.detail = detail.into();
            step.error = error;
        } else {
            self.steps.push(JobStep {
                stage: stage.into(),
                status: status.into(),
                detail: detail.into(),
                error,
            });
        }
    }
    pub fn begin_step(&mut self, stage: &str, detail: &str) {
        for step in &mut self.steps {
            if step.stage != stage && step.status == "running" {
                step.status = "complete".into();
            }
        }
        self.stage = stage.into();
        self.stage_detail = detail.into();
        self.record_step(stage, "running", detail, None);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub job_id: String,
    pub project: String,
    pub page_id: String,
    pub stage: String,
    pub message: String,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayFilters {
    pub brightness: f32,
    pub contrast: f32,
    pub warmth: f32,
    pub saturation: f32,
    pub grayscale: f32,
    pub inversion: f32,
}
impl Default for DisplayFilters {
    fn default() -> Self {
        Self {
            brightness: 1.,
            contrast: 1.,
            warmth: 0.,
            saturation: 1.,
            grayscale: 0.,
            inversion: 0.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeProfile {
    pub colors: BTreeMap<String, String>,
    pub font: String,
    pub scale: f32,
    pub density: f32,
    pub reader_background: String,
    pub margin: u32,
    pub gap: u32,
    pub filters: DisplayFilters,
}
pub fn theme(night: bool) -> ThemeProfile {
    let vals = if night {
        [
            "#171b1e", "#20262a", "#2a3237", "#e8ebe9", "#99a6ac", "#75c8ad", "#0c211b", "#354148",
            "#334b43", "#89d7bf", "#ec9c91", "#d9b86c", "#83c5a7",
        ]
    } else {
        [
            "#f1f0ec", "#ffffff", "#e9e8e2", "#26332f", "#68756f", "#276c55", "#ffffff", "#d6dcd6",
            "#d8e9de", "#34795e", "#ae4035", "#886321", "#34745c",
        ]
    };
    let keys = [
        "window",
        "panel",
        "raised",
        "text",
        "muted",
        "accent",
        "accentText",
        "border",
        "selection",
        "focus",
        "danger",
        "warning",
        "success",
    ];
    ThemeProfile {
        colors: keys
            .iter()
            .zip(vals)
            .map(|(k, v)| (k.to_string(), v.into()))
            .collect(),
        font: "Segoe UI".into(),
        scale: 1.,
        density: 1.,
        reader_background: if night { "#111416" } else { "#dcded7" }.into(),
        margin: 24,
        gap: 20,
        filters: DisplayFilters::default(),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub active: String,
    pub day: ThemeProfile,
    pub night: ThemeProfile,
}
impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            active: "day".into(),
            day: theme(false),
            night: theme(true),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub concurrent_books: usize,
    pub ui_language: UiLanguage,
    pub format_version: u32,
    pub setup: SetupState,
    pub appearance: AppearanceSettings,
    pub providers: Vec<ProviderProfile>,
    pub library_directory: String,
    pub library_view: String,
    pub library_sort: String,
    pub reader: ReaderSettings,
    pub translation: TranslationSettings,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            format_version: 15,
            concurrent_books: 2,
            ui_language: UiLanguage::System,
            setup: SetupState::default(),
            appearance: AppearanceSettings::default(),
            providers: vec![],
            library_directory: String::new(),
            library_view: "grid".into(),
            library_sort: "title".into(),
            reader: ReaderSettings::default(),
            translation: TranslationSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn model_directory(&self) -> std::path::PathBuf {
        if self.library_directory.trim().is_empty() {
            std::path::PathBuf::new()
        } else {
            std::path::Path::new(&self.library_directory).join("models")
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    pub completed: bool,
    pub step: String,
}
impl Default for SetupState {
    fn default() -> Self {
        Self {
            completed: false,
            step: "library".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSettings {
    pub layout: String,
    pub zoom: usize,
}
impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            layout: "continuous".into(),
            zoom: 70,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationItem {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub direction: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPack {
    pub id: String,
    pub distribution: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub languages: Vec<String>,
    pub license: String,
    pub revision: String,
    pub files: Vec<ModelFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiLanguage {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-Hans")]
    SimplifiedChinese,
}
