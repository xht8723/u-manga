//! Validate durable identities without resolving credentials, models, or original images.
use crate::{glossary, safety, types::Job};
use anyhow::{Result, ensure};
use std::collections::HashSet;
// Preserve the existing budget for progress/metadata, while admitting every valid
// 4 MiB glossary (JSON control characters can expand to six bytes each).
pub const MAX_OTHER_ROW_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ROW_BYTES: usize =
    MAX_OTHER_ROW_BYTES + 6 * glossary::MAX_FILE_BYTES + 26 * 10_000 + 2;

/// Every durable writer must admit the same encoded row that recovery reads.
pub fn serialize(job: &Job) -> Result<String> {
    let json = serde_json::to_string(job)?;
    serialized_size(&json, job)?;
    Ok(json)
}

pub fn serialized_size(json: &str, job: &Job) -> Result<()> {
    let glossary_bytes = serde_json::to_string(&job.settings.glossary)?.len();
    ensure!(
        json.len() <= MAX_ROW_BYTES
            && json.len().saturating_sub(glossary_bytes) <= MAX_OTHER_ROW_BYTES,
        "Job row exceeds size limit"
    );
    Ok(())
}
pub const STAGES: &[&str] = &[
    "waiting",
    "starting",
    "loading",
    "detecting",
    "ocr",
    "transcribing",
    "translating",
    "glossary",
    "rendering",
    "cleaning",
    "lettering",
    "saving",
    "done",
];

pub fn validate(sql_id: &str, job: &Job, revision: u64, regions: &HashSet<String>) -> Result<()> {
    safety::identity(sql_id)?;
    safety::identity(&job.id)?;
    safety::identity(&job.page_id)?;
    ensure!(
        sql_id == job.id,
        "Stored Job identity does not match its row"
    );
    ensure!(
        STAGES.contains(&job.stage.as_str())
            && job.stage_detail.len() <= 65536
            && job
                .error
                .as_ref()
                .is_none_or(|e| e.len() <= 4 * 1024 * 1024),
        "Invalid Job progress data"
    );
    ensure!(
        job.project.len() <= 32768
            && job.model_directory.len() <= 32768
            && job.device.len() <= 4096
            && job.created.len() <= 128,
        "Job metadata exceeds limits"
    );
    let s = &job.settings;
    ensure!(
        s.context_pages <= 20
            && ["local", "vision"].contains(&s.mode.as_str())
            && ["manga", "pp"].contains(&s.ocr.as_str())
            && ["cpu", "directml"].contains(&s.device.as_str())
            && s.source_language.len() <= 64
            && s.target_language.len() <= 64
            && s.provider_id.len() <= 4096
            && s.deepl_glossary_id.len() <= 4096,
        "Invalid captured Job settings"
    );
    let mut glossary = s.glossary.clone();
    glossary::validate_entries(&mut glossary, true)?;
    let p = &job.provider;
    p.instructions.validate()?;
    ensure!(
        p.rate_limit.is_finite()
            && p.rate_limit > 0.
            && p.rate_limit <= 1000.
            && [
                &p.id,
                &p.name,
                &p.service,
                &p.protocol,
                &p.endpoint,
                &p.model,
                &p.region,
                &p.app_id
            ]
            .iter()
            .all(|v| v.len() <= 8192),
        "Invalid captured service settings"
    );
    ensure!(
        job.page_revision.is_none_or(|r| r < i64::MAX as u64),
        "Invalid Job page revision"
    );
    let mut seen = HashSet::new();
    ensure!(job.steps.len() <= STAGES.len(), "Too many Job steps");
    for step in &job.steps {
        ensure!(
            STAGES.contains(&step.stage.as_str())
                && seen.insert(&step.stage)
                && ["running", "complete", "warning", "skipped", "failed"]
                    .contains(&step.status.as_str())
                && step.detail.len() <= 65536
                && step
                    .error
                    .as_ref()
                    .is_none_or(|e| e.len() <= 4 * 1024 * 1024),
            "Invalid or duplicate Job step"
        );
    }
    if let Some(c) = &job.glossary_checkpoint {
        let eligible = c.eligible.as_deref().unwrap_or_default();
        ensure!(
            c.processed.iter().all(|id| eligible.contains(id)),
            "Invalid processed glossary regions"
        );
        for ids in [&c.processed[..], eligible] {
            let unique: HashSet<_> = ids.iter().collect();
            ensure!(
                ids.len() <= 4096
                    && unique.len() == ids.len()
                    && ids.iter().all(|id| !id.is_empty() && id.len() <= 256),
                "Invalid glossary checkpoint identities"
            );
            // Historical results can outlive region replacement. Matching revisions must
            // refer to their saved page; stale revisions remain protected at dispatch.
            if job.page_revision == Some(revision) {
                ensure!(
                    ids.iter().all(|id| regions.contains(id)),
                    "Glossary checkpoint region is missing"
                );
            }
        }
        ensure!(
            c.added <= 10_000
                && c.skipped <= glossary::MAX_FILE_BYTES.saturating_mul(4096)
                && c.warning.as_ref().is_none_or(|w| w.len() <= 65536),
            "Invalid glossary checkpoint"
        );
    }
    Ok(())
}
