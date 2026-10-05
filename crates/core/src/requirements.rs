//! Requirements belong to an operation, not to every configured pipeline stage.
use crate::{
    models::{self, ModelLocations, VerificationCache},
    setup::{self, Issue},
    types::*,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Translate,
    Prepare,
    Cleanup,
    Edit,
    Service,
    RegionRead,
    RegionTranslate,
}
impl From<JobKind> for Action {
    fn from(kind: JobKind) -> Self {
        match kind {
            JobKind::Translation => Self::Translate,
            JobKind::Preparation => Self::Prepare,
            JobKind::Cleanup => Self::Cleanup,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Availability {
    pub ready: bool,
    pub issues: Vec<Issue>,
}
impl Availability {
    pub fn reason(&self) -> String {
        self.issues
            .iter()
            .map(|i| i.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn block(&mut self, code: &str, section: &str, message: &str) {
        self.ready = false;
        self.issues.push(Issue {
            code: code.into(),
            section: section.into(),
            message: message.into(),
        });
    }
}
pub fn service_required(action: Action) -> bool {
    matches!(
        action,
        Action::Translate | Action::Service | Action::RegionTranslate
    )
}
pub fn needs_service(action: Action, settings: &TranslationSettings) -> bool {
    service_required(action) || (action == Action::RegionRead && settings.mode == "vision")
}
pub fn check(
    action: Action,
    s: &TranslationSettings,
    provider: Option<&ProviderProfile>,
    credential: bool,
    locations: &ModelLocations,
    cache: &mut VerificationCache,
    cached_cleanup: bool,
) -> anyhow::Result<Availability> {
    let catalog = models::catalog()?;
    let mut local = s.clone();
    if action == Action::RegionTranslate {
        local.mode = "local".into();
    }
    let service = needs_service(action, s);
    if !service {
        local.provider_id.clear();
    }
    let mut issues = if matches!(action, Action::Cleanup | Action::Edit) {
        vec![]
    } else {
        setup::configuration(&local, if service { provider } else { None }, credential)
    };
    if matches!(action, Action::Service | Action::RegionTranslate)
        || (action == Action::RegionRead && s.mode == "vision")
    {
        issues.retain(|i| i.section == "services" || i.code == "languages" || i.code == "mode");
    }
    if !service {
        issues.retain(|i| i.section != "services");
    }
    let mut result = Availability {
        ready: true,
        issues,
    };
    if service && provider.is_none() && !result.issues.iter().any(|i| i.code == "service") {
        result.block(
            "service",
            "services",
            "Select and configure a translation service in Settings → Translation → Services.",
        );
    }
    if action == Action::Prepare && s.mode != "local" {
        result.block(
            "mode",
            "pipeline",
            "Choose Local OCR to prepare a page for manual translation.",
        );
    }
    let ids = match action {
        Action::Service | Action::RegionTranslate => vec![],
        Action::RegionRead => setup::required_packs(s, &catalog)
            .into_iter()
            .filter(|id| id != "rtdetr_int8" && s.cleanup.method.pack() != Some(id.as_str()))
            .collect(),
        Action::Edit if cached_cleanup => vec![],
        Action::Cleanup | Action::Edit => s
            .cleanup
            .method
            .pack()
            .map(|p| vec![p.into()])
            .unwrap_or_default(),
        _ => setup::required_packs(s, &catalog),
    };
    if matches!(
        action,
        Action::Translate | Action::Prepare | Action::RegionRead
    ) && s.mode == "local"
        && s.ocr == "pp"
        && !catalog
            .iter()
            .any(|p| p.kind == "recognizer" && p.languages.contains(&s.source_language))
    {
        result.block(
            "ocr_pack",
            "pipeline",
            "No PP-OCRv5 pack supports the selected source language.",
        );
    }
    for id in ids {
        let pack = catalog
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown model pack {id}"))?;
        if !cache
            .check(locations.root(pack), pack, false)
            .unwrap_or(false)
        {
            result.block(
                &format!("pack:{id}"),
                "models",
                &if pack.distribution == "bundled" {
                    "Restart U-Manga to repair its included dialogue detector.".into()
                } else {
                    format!(
                        "Download or verify {} in Settings → Translation → Local models.",
                        pack.name
                    )
                },
            );
        }
    }
    result.ready = result.issues.is_empty();
    Ok(result)
}

/// Whether edits can render without a neural cleanup dependency. An imported
/// background or manual-only cleanup works directly; otherwise a saved background
/// requires unchanged cleanup inputs and lettering eligibility.
pub fn cached_edit(
    page: &Page,
    base: &Page,
    renderer: &crate::render::Renderer,
    language: &str,
) -> bool {
    if page.source.path != base.source.path {
        return false;
    }
    if let Some(path) = &page.background {
        return image::image_dimensions(path).ok() == Some((page.width, page.height));
    }
    if page.regions.iter().all(|r| {
        r.overlay_only || r.allow_fill || (!crate::safety::has_text(&r.target) && !r.prepared)
    }) {
        return true;
    }
    if page.background != base.background || page.regions.len() != base.regions.len() {
        return false;
    }
    let cache = page.cleanup.as_ref().is_some_and(|c| {
        c.reviews.is_empty()
            && image::image_dimensions(&c.path).ok() == Some((page.width, page.height))
    });
    if !cache {
        return false;
    }
    page.regions.iter().all(|r| {
        base.regions.iter().find(|b| b.id == r.id).is_some_and(|b| {
            let identity = |x: &Region| {
                serde_json::to_vec(&(
                    x.bbox,
                    &x.bubble,
                    x.allow_fill,
                    x.overlay_only,
                    &x.style.fill,
                    x.prepared,
                ))
                .ok()
            };
            let eligible = |x: &Region| {
                x.prepared
                    || (crate::safety::has_text(&x.target)
                        && renderer.lettering(x, language).ok().flatten().is_some())
            };
            identity(r) == identity(b) && eligible(r) == eligible(b)
        })
    })
}
