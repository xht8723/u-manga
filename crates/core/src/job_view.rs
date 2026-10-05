//! IPC projection: clients need progress and workflow, never the captured glossary/credentials config.
use crate::types::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSettings {
    pub source_language: String,
    pub target_language: String,
    pub mode: String,
    pub auto_glossary: bool,
    pub glossary_enabled: bool,
    pub cleanup: CleanupSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewProvider {
    pub name: String,
    pub model: String,
    pub service: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlossaryProgress {
    pub added: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
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
    pub kind: JobKind,
    pub cleanup_model: Option<CleanupMethod>,
    pub steps: Vec<JobStep>,
    pub page_revision: Option<u64>,
    pub settings: ViewSettings,
    pub provider: ViewProvider,
    pub glossary_checkpoint: Option<GlossaryProgress>,
    pub requirements_key: String,
    pub fresh: bool,
}
impl JobView {
    pub fn from_job(j: &Job) -> Self {
        Self::with_requirements_key(j, Self::requirements_key(j))
    }
    pub(crate) fn requirements_key(j: &Job) -> String {
        let s = &j.settings;
        let p = &j.provider;
        // Requirements do not depend on glossary contents, stage details, or timer ticks.
        let requirements = serde_json::to_vec(&(
            j.kind,
            (
                &s.source_language,
                &s.target_language,
                &s.mode,
                &s.ocr,
                &s.device,
                &s.provider_id,
                &s.cleanup,
            ),
            (
                &p.id,
                &p.service,
                &p.protocol,
                &p.endpoint,
                &p.model,
                p.vision,
                &p.app_id,
                &p.region,
            ),
            &j.model_directory,
            j.fresh,
        ))
        .expect("finite captured settings");
        crate::store::digest(&requirements)
    }
    pub(crate) fn with_requirements_key(j: &Job, requirements_key: String) -> Self {
        let s = &j.settings;
        let p = &j.provider;
        Self {
            id: j.id.clone(),
            project: j.project.clone(),
            page_id: j.page_id.clone(),
            status: j.status,
            stage: j.stage.clone(),
            stage_detail: j.stage_detail.clone(),
            error: j.error.clone(),
            created: j.created.clone(),
            elapsed_ms: j.elapsed_ms,
            device: j.device.clone(),
            kind: j.kind,
            cleanup_model: j.cleanup_model,
            steps: j.steps.clone(),
            page_revision: j.page_revision,
            settings: ViewSettings {
                source_language: s.source_language.clone(),
                target_language: s.target_language.clone(),
                mode: s.mode.clone(),
                auto_glossary: s.auto_glossary,
                glossary_enabled: s.glossary_enabled,
                cleanup: s.cleanup.clone(),
            },
            provider: ViewProvider {
                name: p.name.clone(),
                model: p.model.clone(),
                service: p.service.clone(),
            },
            glossary_checkpoint: j
                .glossary_checkpoint
                .as_ref()
                .map(|c| GlossaryProgress { added: c.added }),
            requirements_key,
            fresh: j.fresh,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeView {
    #[serde(flatten)]
    pub job: JobView,
    pub sequence: u64,
    pub stopping: bool,
    pub timer_running: bool,
}
