use std::{
    fs,
    path::{Path, PathBuf},
};
use umanga_core::{documents, library, providers, safety, store, types::*};
fn fixture() -> (tempfile::TempDir, Project) {
    let t = tempfile::tempdir().unwrap();
    let image = t.path().join("original.png");
    image::RgbImage::new(40, 60).save(&image).unwrap();
    let pages = documents::import(&[image.to_string_lossy().into()]).unwrap();
    let p = store::create(&t.path().join("book.umanga"), "Audit", &pages).unwrap();
    (t, p)
}
#[test]
fn invalid_cached_text_is_bypassed_valid_history_reused_and_sql_errors_propagate() {
    let (_t, p) = fixture();
    let path = Path::new(&p.path);
    let region = Region {
        source: "原文".into(),
        ..Default::default()
    };
    let valid = TranslationItem {
        id: region.id.clone(),
        source: "原文".into(),
        target: "译文".into(),
        direction: String::new(),
    };
    store::cache_put(path, "good", std::slice::from_ref(&valid)).unwrap();
    assert_eq!(
        store::cache_validated(path, "good", std::slice::from_ref(&region), true)
            .unwrap()
            .unwrap()[0]
            .target,
        "译文"
    );
    let mut invalid = valid.clone();
    invalid.target = "字".repeat(4097);
    assert!(store::cache_put(path, "bad-write", std::slice::from_ref(&invalid)).is_err());
    let c = store::connection(path).unwrap();
    c.execute(
        "INSERT INTO cache VALUES('bad',?1)",
        [serde_json::to_string(&vec![invalid]).unwrap()],
    )
    .unwrap();
    assert!(
        store::cache_validated(path, "bad", std::slice::from_ref(&region), true)
            .unwrap()
            .is_none()
    );
    store::cache_put_validated(path, "bad", &[valid]).unwrap();
    assert!(
        store::cache_validated(path, "bad", &[region], true)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM cache WHERE key='bad'", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        1
    );
    c.execute("DROP TABLE cache", []).unwrap();
    assert!(store::cache_get(path, "absent").is_err());
    assert!(store::cache_validated(path, "absent", &[], true).is_err());
}
#[test]
fn oversized_provider_results_rejected_before_checkpoint() {
    let r = Region::default();
    for (source, target) in [
        ("字".repeat(16385), "好".into()),
        ("源".into(), "字".repeat(4097)),
    ] {
        let text = serde_json::json!({"regions":[{"id":r.id,"source":source,"target":target}]})
            .to_string();
        assert!(providers::validate(&text, std::slice::from_ref(&r)).is_err());
    }
}
#[tokio::test]
async fn unreadable_text_never_calls_provider() {
    let p = ProviderProfile {
        endpoint: "http://127.0.0.1:1".into(),
        ..Default::default()
    };
    let s = TranslationSettings {
        mode: "local".into(),
        ..Default::default()
    };
    let r = Region {
        source: " \n\t".into(),
        ..Default::default()
    };
    assert!(
        providers::llm(&p, "", &s, std::slice::from_ref(&r), None, "")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        providers::conventional(&p, "", &s, &[r], "")
            .await
            .unwrap()
            .is_empty()
    );
}
#[test]
fn inconsistent_import_never_copies_or_changes_source() {
    for column in ["number", "revision"] {
        let (t, p) = fixture();
        let path = Path::new(&p.path);
        store::connection(path)
            .unwrap()
            .execute(&format!("UPDATE pages SET {column}={column}+1"), [])
            .unwrap();
        let before = store::file_hash(path).unwrap();
        let root = t.path().join("destination");
        assert!(library::import_book(&root, path).is_err());
        assert_eq!(before, store::file_hash(path).unwrap());
        assert!(
            fs::read_dir(root).unwrap().all(|e| e
                .unwrap()
                .path()
                .extension()
                .is_none_or(|x| x != "umanga"))
        );
    }
}
#[test]
fn failed_atomic_replace_removes_only_its_temporary_file() {
    let t = tempfile::tempdir().unwrap();
    let destination = t.path().join("occupied");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(store::atomic_write(&destination, b"replacement").is_err());
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}
#[test]
fn aggregate_render_limit_does_not_allocate_images() {
    let (_t, p) = fixture();
    let mut page = p.pages[0].clone();
    assert!(safety::render_budget(&page).is_ok());
    page.width = 5000;
    page.height = 5000;
    page.regions = (0..100)
        .map(|_| Region {
            bbox: [0., 0., 4000., 4000.],
            ..Default::default()
        })
        .collect();
    assert!(safety::render_budget(&page).is_err());
}
#[test]
fn committed_glossary_reports_summary_failure_without_losing_save() {
    let (t, p) = fixture();
    let path = Path::new(&p.path);
    let b = library::open(path).unwrap();
    let mut glossary = b.glossary.clone();
    glossary.enabled = false;
    let broken = t.path().join("not-a-directory");
    fs::write(&broken, b"sentinel").unwrap();
    let saved = library::save_glossary(&broken, path, glossary.revision, glossary).unwrap();
    assert!(!saved.glossary.enabled);
    assert_eq!(saved.warnings.len(), 1);
    assert!(!library::open(path).unwrap().glossary.enabled);
    assert!(library::open(path).unwrap().warnings.is_empty());
}
#[cfg(windows)]
#[test]
fn windows_canonical_aliases_compare_equal() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("models");
    fs::create_dir(&path).unwrap();
    let alias = PathBuf::from(path.to_string_lossy().to_uppercase());
    assert!(safety::same_path(&alias, &path.canonicalize().unwrap()).unwrap());
}
#[cfg(windows)]
#[tokio::test]
async fn model_directory_junction_never_changes_external_sentinel() {
    use std::os::windows::process::CommandExt;
    let t = tempfile::tempdir().unwrap();
    let models = t.path().join("models");
    let external = t.path().join("external");
    fs::create_dir(&models).unwrap();
    fs::create_dir(&external).unwrap();
    let mut pack = umanga_core::models::catalog().unwrap().remove(0);
    pack.files.truncate(1);
    let sentinel = external.join(&pack.files[0].name);
    fs::write(&sentinel, b"keep").unwrap();
    let junction = models.join(&pack.id);
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(output.status.success());
    let result = umanga_core::models::download(
        &models,
        &pack,
        &tokio_util::sync::CancellationToken::new(),
        |_, _| {},
    )
    .await;
    let contents = fs::read(&sentinel).unwrap();
    // Remove the junction itself before TempDir recursively cleans its own directory.
    fs::remove_dir(&junction).unwrap();
    assert!(result.is_err());
    assert_eq!(contents, b"keep");
    assert_eq!(fs::read_dir(external).unwrap().count(), 1);
}
#[tokio::test]
async fn service_waits_are_shared_and_cancellation_does_not_reserve_future_slots() {
    use std::time::{Duration, Instant};
    let p = ProviderProfile {
        id: uid(),
        rate_limit: 10.,
        ..Default::default()
    };
    umanga_core::service_requests::wait(&p).await;
    let started = Instant::now();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(15),
            umanga_core::service_requests::wait(&p)
        )
        .await
        .is_err()
    );
    umanga_core::service_requests::wait(&p).await;
    assert!(started.elapsed() >= Duration::from_millis(90));
    assert!(started.elapsed() < Duration::from_millis(500));
    let mut other = p.clone();
    other.id = uid();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            umanga_core::service_requests::wait(&other)
        )
        .await
        .is_ok()
    );
}
