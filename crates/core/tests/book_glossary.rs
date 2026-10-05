use std::path::Path;
use umanga_core::{
    glossary::BookGlossary,
    library::{self, ImportPreview, Metadata},
    store,
    types::*,
};
fn make(root: &Path, title: &str) -> library::Book {
    library::create(
        root,
        Metadata {
            title: title.into(),
            ..Default::default()
        },
        ImportPreview::default(),
        None,
    )
    .unwrap()
}
#[test]
fn isolated_glossary_saves_preserve_other_book_changes_and_detect_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let a = make(dir.path(), "甲");
    let b = make(dir.path(), "乙");
    let path = Path::new(&a.path);
    let mut glossary = a.glossary.clone();
    glossary.entries.push(GlossaryEntry {
        source: " 魔王 ".into(),
        target: "魔王".into(),
    });
    let mut metadata = a.metadata.clone();
    metadata.title = "Renamed".into();
    library::update(dir.path(), path, metadata, a.overrides.clone(), a.revision).unwrap();
    let saved = library::save_glossary(dir.path(), path, 0, glossary.clone()).unwrap();
    assert_eq!(saved.metadata.title, "Renamed");
    assert_eq!(saved.glossary.entries[0].source, "魔王");
    assert_eq!(saved.glossary.revision, 1);
    assert_eq!(
        library::open(Path::new(&b.path)).unwrap().glossary,
        BookGlossary::default()
    );
    assert!(library::save_glossary(dir.path(), path, 0, glossary).is_err());
    assert_eq!(library::open(path).unwrap().glossary, saved.glossary);
}
#[test]
fn snapshots_and_cache_identity_follow_book_glossary_not_global_or_override_terms() {
    let dir = tempfile::tempdir().unwrap();
    let mut book = make(dir.path(), "Book");
    book.glossary.enabled = true;
    let mut defaults = AppSettings::default();
    defaults.translation.glossary = vec![GlossaryEntry {
        source: "global".into(),
        target: "ignored".into(),
    }];
    book.overrides.translation = Some(defaults.translation.clone());
    book.glossary.entries = vec![GlossaryEntry {
        source: "勇者".into(),
        target: "勇者".into(),
    }];
    book.glossary.deepl_glossary_id = "hosted-for-this-book".into();
    let captured = library::effective(&book, &defaults);
    assert_eq!(captured.glossary, book.glossary.entries);
    assert_eq!(captured.deepl_glossary_id, "hosted-for-this-book");
    book.overrides.translation = None;
    assert_eq!(
        library::effective(&book, &defaults).glossary,
        captured.glossary
    );
    let page = Page {
        id: uid(),
        number: 0,
        name: "page".into(),
        source: Source {
            path: "source.png".into(),
            kind: "image".into(),
            entry: None,
            index: 0,
        },
        width: 10,
        height: 10,
        pdf_points: None,
        source_stamp: String::new(),
        fingerprint: "original".into(),
        revision: 0,
        regions: vec![],
        rendered: None,
        background: None,
        cleanup: None,
        status: "new".into(),
        error: None,
    };
    let profile = ProviderProfile::default();
    let old_key = store::cache_key(&page, &captured, &profile, "").unwrap();
    book.glossary.entries[0].target = "英雄".into();
    let next = library::effective(&book, &defaults);
    assert_ne!(
        old_key,
        store::cache_key(&page, &next, &profile, "").unwrap()
    );
    assert_eq!(captured.glossary[0].target, "勇者");
    book.glossary.enabled = false;
    let disabled = library::effective(&book, &defaults);
    assert!(disabled.glossary.is_empty());
    assert_eq!(book.glossary.entries.len(), 1);
    let mut old_disabled = disabled.clone();
    old_disabled.glossary = book.glossary.entries.clone();
    assert_eq!(
        store::cache_key(&page, &disabled, &profile, "").unwrap(),
        store::cache_key(&page, &old_disabled, &profile, "").unwrap(),
        "Unused terms must not alter cache reuse"
    );
    let prompt = umanga_core::prompts::build(&disabled, false, "", None).unwrap();
    assert_eq!(prompt.user_preamble, "Current regions to translate:");
}
#[test]
fn fresh_defaults_are_opt_in_and_saved_selections_survive_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let book = make(dir.path(), "New book");
    assert!(!book.glossary.enabled);
    assert!(!book.glossary.auto_detect);
    let mut preferences = AppSettings::default();
    assert!(
        serde_json::to_value(&preferences)
            .unwrap()
            .get("newBookAutoGlossary")
            .is_none()
    );
    assert!(!preferences.translation.glossary_enabled);
    assert!(!preferences.translation.auto_glossary);
    assert_eq!(
        preferences.translation.cleanup.method,
        CleanupMethod::MangaLama
    );
    preferences.translation.cleanup.method = CleanupMethod::MangaAot;
    let saved: AppSettings =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert!(
        serde_json::to_value(&saved)
            .unwrap()
            .get("newBookAutoGlossary")
            .is_none()
    );
    assert_eq!(saved.translation.cleanup.method, CleanupMethod::MangaAot);
    let mut glossary = book.glossary;
    glossary.enabled = true;
    glossary.auto_detect = true;
    glossary.entries.push(GlossaryEntry {
        source: "アリス".into(),
        target: "爱丽丝".into(),
    });
    let saved = library::save_glossary(
        dir.path(),
        Path::new(&book.path),
        glossary.revision,
        glossary,
    )
    .unwrap();
    assert_eq!(
        library::open(Path::new(&book.path)).unwrap().glossary,
        saved.glossary
    );
    assert!(saved.glossary.enabled && saved.glossary.auto_detect);
}
#[test]
fn export_protection_covers_other_books_and_owned_assets() {
    let dir = tempfile::tempdir().unwrap();
    let a = make(dir.path(), "A");
    let b = make(dir.path(), "B");
    let protected = library::export_protection(Path::new(&a.path)).unwrap();
    for p in [
        Path::new(&b.path).to_owned(),
        store::assets(Path::new(&b.path)).join("glossary.csv"),
        dir.path().join("models/model.csv"),
        dir.path().join("library.sqlite"),
    ] {
        assert!(protected.check(&p).is_err());
    }
    assert!(protected.check(&dir.path().join("glossary.csv")).is_ok());
}
