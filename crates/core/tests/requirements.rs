use std::path::PathBuf;
use umanga_core::{
    models::{ModelLocations, VerificationCache},
    requirements::{self, Action},
    setup,
    types::*,
};
fn status(
    action: Action,
    s: &TranslationSettings,
    p: Option<&ProviderProfile>,
    key: bool,
    cached: bool,
) -> requirements::Availability {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    requirements::check(
        action,
        s,
        p,
        key,
        &ModelLocations {
            bundled: root.join("assets/models"),
            downloaded: root.join("test-output/no-models-here"),
        },
        &mut VerificationCache::default(),
        cached,
    )
    .unwrap()
}

#[test]
fn region_operations_only_require_the_dependencies_they_use() {
    let mut s = TranslationSettings::default();
    let p = ProviderProfile {
        protocol: "openai".into(),
        model: "text-model".into(),
        vision: false,
        ..Default::default()
    };
    let local = status(Action::RegionRead, &s, None, false, false);
    assert!(!requirements::needs_service(Action::RegionRead, &s));
    assert!(local.issues.iter().any(|i| i.code == "pack:manga_ocr_onnx"));
    assert!(local.issues.iter().all(|i| i.section != "services"
        && i.code != "pack:rtdetr_int8"
        && !i.code.contains("inpaint")));
    s.mode = "vision".into();
    assert!(requirements::needs_service(Action::RegionRead, &s));
    assert!(!status(Action::RegionRead, &s, Some(&p), true, false).ready);
    assert!(
        status(Action::RegionTranslate, &s, Some(&p), true, false).ready,
        "Text-only region translation requires neither image support nor OCR/cleanup packs"
    );
    let p = ProviderProfile { vision: true, ..p };
    assert!(status(Action::RegionRead, &s, Some(&p), true, false).ready);
}
#[test]
fn recommended_defaults_and_optional_local_service_do_not_weaken_automatic_requirements() {
    let mut s = TranslationSettings::default();
    assert_eq!(s.mode, "local");
    assert_eq!(s.ocr, "manga");
    assert_eq!(s.cleanup.method, CleanupMethod::MangaLama);
    assert_eq!(s.device, "directml");
    assert_eq!(s.cleanup.device, CleanupDevice::Directml);
    assert!(setup::configuration(&s, None, false).is_empty());
    assert!(
        status(Action::Translate, &s, None, false, false)
            .issues
            .iter()
            .any(|i| i.code == "service")
    );
    let prepare = status(Action::Prepare, &s, None, false, false);
    assert!(prepare.issues.iter().all(|i| i.section != "services"));
    assert!(
        prepare
            .issues
            .iter()
            .any(|i| i.code == "pack:manga_ocr_onnx")
    );
    assert!(
        prepare
            .issues
            .iter()
            .any(|i| i.code == "pack:inpaint_manga_lama")
    );
    s.source_language = "ko".into();
    assert!(
        setup::configuration(&s, None, false)
            .iter()
            .any(|i| i.code == "ocr_language")
    );
    s.mode = "vision".into();
    assert!(
        setup::configuration(&s, None, false)
            .iter()
            .any(|i| i.code == "service")
    );
    assert!(
        status(Action::Prepare, &s, None, false, false)
            .issues
            .iter()
            .any(|i| i.code == "mode")
    );
}
#[test]
fn cleanup_and_saved_background_edits_never_require_ocr_or_services() {
    let mut s = TranslationSettings {
        source_language: "unsupported".into(),
        provider_id: "disconnected".into(),
        ..Default::default()
    };
    s.cleanup.method = CleanupMethod::Solid;
    assert!(status(Action::Cleanup, &s, None, false, false).ready);
    s.cleanup.method = CleanupMethod::MangaAot;
    assert_eq!(
        status(Action::Cleanup, &s, None, false, false).issues.len(),
        1
    );
    assert!(status(Action::Edit, &s, None, false, true).ready);
    assert!(!requirements::service_required(Action::Prepare));
    assert!(!requirements::service_required(Action::Cleanup));
}
#[test]
fn service_test_has_no_inference_model_dependency() {
    let s = TranslationSettings {
        provider_id: "test".into(),
        source_language: "ko".into(),
        ..Default::default()
    };
    let p = ProviderProfile {
        id: "test".into(),
        model: "mock".into(),
        ..Default::default()
    };
    assert!(status(Action::Service, &s, Some(&p), true, false).ready);
    let missing = status(Action::Service, &s, Some(&p), false, false);
    assert!(missing.issues.iter().any(|i| i.code == "credential"));
    assert!(missing.issues.iter().all(|i| !i.code.starts_with("pack:")));
}
