use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use umanga_core::{
    documents,
    library::{self, Chapter, ImportPreview, Metadata},
    pipeline::Engine,
    store,
    types::*,
};
fn image(p: &Path) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(64, 96, image::Rgb([250, 240, 230]))
        .save(p)
        .unwrap();
}
fn meta() -> Metadata {
    Metadata {
        title: "書籍".into(),
        ..Default::default()
    }
}
#[test]
fn models_follow_library_and_relocation_preserves_partial_downloads_and_captured_jobs() {
    let t = tempfile::tempdir().unwrap();
    let old = t.path().join("漫画 library");
    let target = t.path().join("新しい library");
    let mut settings = AppSettings::default();
    assert!(settings.model_directory().as_os_str().is_empty());
    settings.library_directory = old.to_string_lossy().into();
    assert_eq!(settings.model_directory(), old.join("models"));
    assert!(
        serde_json::to_value(&settings)
            .unwrap()
            .get("modelDirectory")
            .is_none()
    );
    let pack = settings.model_directory().join("inpaint_migan");
    fs::create_dir_all(&pack).unwrap();
    fs::write(pack.join("model.onnx"), b"verified test model").unwrap();
    fs::write(pack.join("second.onnx.part"), b"partial download").unwrap();
    let original = t.path().join("source.png");
    image(&original);
    let book = library::create(&old, meta(), scan(&original), None).unwrap();
    let job = Job {
        id: uid(),
        project: book.path.clone(),
        page_id: book.chapters[0].page_ids[0].clone(),
        kind: JobKind::Cleanup,
        cleanup_model: Some(CleanupMethod::Migan),
        steps: vec![],
        origin: umanga_core::job_state::JobOrigin::Explicit,
        settings_captured: true,
        attempt: 0,
        failure_kind: None,
        page_revision: None,
        fresh: false,
        glossary_checkpoint: None,
        model_directory: settings.model_directory().to_string_lossy().into(),
        status: "paused".into(),
        stage: "cleaning".into(),
        stage_detail: "Tile 1".into(),
        error: None,
        created: now(),
        elapsed_ms: 0,
        device: "cpu".into(),
        settings: settings.translation.clone(),
        provider: Default::default(),
    };
    store::save_job(Path::new(&book.path), &job).unwrap();
    library::copy_model_storage(&old, &target).unwrap();
    let copied = library::import_book(&target, Path::new(&book.path)).unwrap();
    library::relocate_job_storage(Path::new(&copied.path), &old, &target).unwrap();
    let moved = &store::jobs(Path::new(&copied.path)).unwrap()[0];
    assert_eq!(moved.id, job.id);
    assert_eq!(moved.project, copied.path);
    assert_eq!(Path::new(&moved.model_directory), target.join("models"));
    assert_eq!(moved.status, "paused");
    assert_eq!(
        fs::read(target.join("models/inpaint_migan/second.onnx.part")).unwrap(),
        b"partial download"
    );
    assert_eq!(
        fs::read(pack.join("model.onnx")).unwrap(),
        b"verified test model"
    );
    assert_eq!(
        store::jobs(Path::new(&book.path)).unwrap()[0].model_directory,
        job.model_directory
    );
    assert!(library::copy_model_storage(&old, &pack.join("nested")).is_err());
    settings.library_directory = target.to_string_lossy().into();
    let restored: AppSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(restored.model_directory(), target.join("models"));
}
fn scan(p: &Path) -> ImportPreview {
    library::scan(
        &[p.to_string_lossy().into()],
        true,
        &CancellationToken::new(),
    )
    .unwrap()
}
#[test]
fn idempotent_import_rejects_incomplete_existing_copy() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("source.png");
    image(&original);
    let old = t.path().join("old");
    let new = t.path().join("new");
    let book = library::create(&old, meta(), scan(&original), None).unwrap();
    let copy = library::import_book(&new, Path::new(&book.path)).unwrap();
    store::connection(Path::new(&copy.path))
        .unwrap()
        .execute("DELETE FROM pages", [])
        .unwrap();
    assert!(library::import_book(&new, Path::new(&book.path)).is_err());
    assert_eq!(store::open(Path::new(&book.path)).unwrap().pages.len(), 1);
    assert!(original.exists());
}
#[test]
fn relocation_retries_pruning_and_index_failures_after_database_removal() {
    for failure in ["prune", "index"] {
        let t = tempfile::tempdir().unwrap();
        let original = t.path().join("source.png");
        image(&original);
        let root = t.path().join("library");
        let book = library::create(&root, meta(), scan(&original), None).unwrap();
        let path = Path::new(&book.path);
        let assets = store::assets(path);
        let stash = root.join("stashed-assets");
        let linked = assets.join("linked.png");
        let unused = assets.join("unused.png");
        image(&linked);
        image(&unused);
        let protected = std::collections::HashSet::from([linked.canonicalize().unwrap()]);
        let original_hash = store::file_hash(&original).unwrap();
        let linked_hash = store::file_hash(&linked).unwrap();
        if failure == "prune" {
            fs::rename(&assets, &stash).unwrap();
            fs::write(&assets, b"failure injection").unwrap();
        } else {
            rusqlite::Connection::open(root.join("library.sqlite")).unwrap().execute_batch(
                "CREATE TRIGGER deny_retirement BEFORE DELETE ON books BEGIN SELECT RAISE(FAIL,'injected retirement index failure'); END;"
            ).unwrap();
        }
        for _ in 0..2 {
            assert!(library::retire_copy(&root, path, &book.id, &protected).is_err());
            assert!(!path.exists());
            assert_eq!(library::list(&root).unwrap().len(), 1);
        }
        if failure == "prune" {
            fs::remove_file(&assets).unwrap();
            fs::rename(&stash, &assets).unwrap();
        } else {
            rusqlite::Connection::open(root.join("library.sqlite"))
                .unwrap()
                .execute_batch("DROP TRIGGER deny_retirement")
                .unwrap();
        }
        library::retire_copy(&root, path, &book.id, &protected).unwrap();
        assert!(library::list(&root).unwrap().is_empty());
        assert!(!unused.exists());
        assert_eq!(store::file_hash(&original).unwrap(), original_hash);
        assert_eq!(store::file_hash(&linked).unwrap(), linked_hash);
        library::retire_copy(&root, path, &book.id, &protected).unwrap();
    }
}
#[cfg(windows)]
#[test]
fn model_relocation_rejects_destination_junctions_before_writing() {
    use std::os::windows::process::CommandExt;
    for nested in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join("old");
        let target = t.path().join("new");
        let external = t.path().join("external");
        fs::create_dir_all(old.join("models/pack")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&external).unwrap();
        fs::write(old.join("models/pack/model.bin"), b"new model").unwrap();
        let junction = if nested {
            fs::create_dir(target.join("models")).unwrap();
            target.join("models/pack")
        } else {
            target.join("models")
        };
        let sentinel = if nested {
            external.join("model.bin")
        } else {
            fs::create_dir(external.join("pack")).unwrap();
            external.join("pack/model.bin")
        };
        fs::write(&sentinel, b"protected original").unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(junction.to_string_lossy().replace('/', "\\"))
            .arg(external.to_string_lossy().replace('/', "\\"))
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Junction fixture failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result = library::copy_model_storage(&old, &target);
        let contents = fs::read(&sentinel).unwrap();
        // Remove only this test's junction, before TempDir visits the tree.
        fs::remove_dir(&junction).unwrap();
        assert!(result.is_err(), "destination links must be rejected");
        assert_eq!(contents, b"protected original");
    }
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[test]
fn empty_books_and_failed_creation_leave_no_partial_records() {
    let t = tempfile::tempdir().unwrap();
    let lib = t.path().join("library");
    let b = library::create(&lib, meta(), ImportPreview::default(), None).unwrap();
    assert!(b.chapters.is_empty());
    assert_eq!(library::list(&lib).unwrap()[0].pages, 0);
    assert!(
        library::create(
            &lib,
            meta(),
            ImportPreview::default(),
            Some(t.path().join("missing.png").to_string_lossy().into())
        )
        .is_err()
    );
    assert_eq!(library::list(&lib).unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(&lib)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|s| s == "umanga"))
            .count(),
        1
    );
}
#[test]
fn thousand_book_index_does_not_open_book_files() {
    let t = tempfile::tempdir().unwrap();
    let b = library::create(t.path(), meta(), ImportPreview::default(), None).unwrap();
    let template = library::list(t.path()).unwrap().remove(0);
    let mut c = rusqlite::Connection::open(t.path().join("library.sqlite")).unwrap();
    let tx = c.transaction().unwrap();
    for i in 1..1000 {
        let mut s = template.clone();
        s.id = format!("fixture-{i}");
        s.path = t
            .path()
            .join(format!("missing-{i}.umanga"))
            .to_string_lossy()
            .into();
        s.metadata.title = format!("Book {i}");
        tx.execute(
            "INSERT INTO books VALUES(?1,?2)",
            rusqlite::params![s.id, serde_json::to_string(&s).unwrap()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let now = std::time::Instant::now();
    let books = library::list(t.path()).unwrap();
    println!("1,000 summaries: {} ms", now.elapsed().as_millis());
    assert_eq!(books.len(), 1000);
    assert_eq!(
        fs::read_dir(t.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|s| s == "umanga"))
            .count(),
        1
    );
    assert!(books.iter().any(|s| s.id == b.id));
}
#[test]
fn deletion_preserves_originals_linked_from_owned_asset_directories() {
    let t = tempfile::tempdir().unwrap();
    let lib = t.path().join("library");
    let b = library::create(&lib, meta(), ImportPreview::default(), None).unwrap();
    let linked = store::assets(Path::new(&b.path)).join("selected-original.png");
    image(&linked);
    let other = library::create(&lib, meta(), scan(&linked), None).unwrap();
    library::delete(&lib, Path::new(&b.path)).unwrap();
    assert!(linked.is_file());
    assert!(
        documents::load(
            &library::chapter_pages(Path::new(&other.path), &other.chapters[0].id).unwrap()[0]
                .source
        )
        .is_ok()
    );
}
#[test]
fn filesystem_chapters_natural_order_duplicates_warnings_manual_cancel() {
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("原稿");
    image(&raw.join("Chapter 10/2.png"));
    image(&raw.join("Chapter 2/deeper/10.png"));
    image(&raw.join("Chapter 2/deeper/2.png"));
    image(&raw.join("loose.png"));
    fs::write(raw.join("bad.pdf"), b"not pdf").unwrap();
    let preview = scan(&raw);
    assert_eq!(
        preview
            .chapters
            .iter()
            .map(|c| c.title.as_str())
            .collect::<Vec<_>>(),
        ["bad", "Chapter 2", "Chapter 10", "原稿"]
    );
    assert_eq!(preview.pages[0].name, "2.png");
    assert_eq!(preview.pages[1].name, "10.png");
    assert_eq!(preview.chapters[1].page_ids.len(), 2);
    assert!(!preview.warnings.is_empty());
    let doubled = library::scan(
        &[raw.to_string_lossy().into(), raw.to_string_lossy().into()],
        true,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(doubled.pages.len(), 4);
    assert!(doubled.warnings.iter().any(|s| s.contains("Duplicate")));
    let manual = library::scan(
        &[raw.to_string_lossy().into()],
        false,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(manual.chapters.len(), 1);
    assert_eq!(manual.pages.len(), 4);
    let token = CancellationToken::new();
    token.cancel();
    assert!(library::scan(&[raw.to_string_lossy().into()], true, &token).is_err());
    let flat = t.path().join("flat");
    image(&flat.join("10.png"));
    image(&flat.join("2.png"));
    let preview = scan(&flat);
    assert_eq!(preview.chapters[0].title, "flat");
    assert_eq!(preview.pages[0].name, "2.png");
    let off = library::scan(
        &[
            flat.join("2.png").to_string_lossy().into(),
            flat.join("10.png").to_string_lossy().into(),
        ],
        false,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(off.chapters.len(), 1);
    let selected = library::scan(
        &[
            flat.join("10.png").to_string_lossy().into(),
            flat.join("2.png").to_string_lossy().into(),
        ],
        true,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(selected.chapters.len(), 1);
    assert_eq!(selected.pages[0].name, "2.png");
}
#[test]
fn archive_and_pdf_sources_and_ordered_exports() {
    documents::init_pdfium(&root().join("assets/runtime")).unwrap();
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("入力");
    image(&raw.join("p.png"));
    let bytes = fs::read(raw.join("p.png")).unwrap();
    let archive = raw.join("Chapter 2.cbz");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for n in ["deep/10.png", "deep/2.png"] {
        zip.start_file(n, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let pages = documents::import(&[raw.join("p.png").to_string_lossy().into()]).unwrap();
    let document = store::create(&t.path().join("source.umanga"), "pdf", &pages).unwrap();
    documents::export(&document, &raw.join("Chapter 10.pdf"), "pdf").unwrap();
    let p = scan(&raw);
    assert_eq!(p.chapters[0].title, "Chapter 2");
    assert_eq!(p.chapters[0].page_ids.len(), 2);
    assert_eq!(p.chapters[1].title, "Chapter 10");
    assert_eq!(p.pages[0].name, "deep/2.png");
    let lib = t.path().join("library");
    let b = library::create(&lib, meta(), p, None).unwrap();
    let stored = store::open(Path::new(&b.path)).unwrap();
    let names = stored
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), format!("001 Chapter/{}.png", i + 1)))
        .collect();
    let out = t.path().join("export.cbz");
    documents::export_named(&stored, &out, "cbz", &names).unwrap();
    let mut z = zip::ZipArchive::new(fs::File::open(out).unwrap()).unwrap();
    assert_eq!(z.by_index(0).unwrap().name(), "001 Chapter/1.png");
}
#[test]
fn organization_is_atomic_preserves_edits_cache_order_and_completion() {
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("raw");
    for i in 0..4 {
        image(&raw.join(format!("{i}.png")));
    }
    let lib = t.path().join("library");
    let mut b = library::create(&lib, meta(), scan(&raw), None).unwrap();
    let path = PathBuf::from(&b.path);
    let mut p = store::page(&path, &b.chapters[0].page_ids[0]).unwrap();
    p.regions.push(Region {
        target: "保留".into(),
        ..Default::default()
    });
    let revision = p.revision;
    assert!(store::save_page(&path, &mut p, revision).unwrap());
    store::cache_put(
        &path,
        "cached",
        &[TranslationItem {
            id: p.regions[0].id.clone(),
            source: "a".into(),
            target: "b".into(),
            direction: "vertical".into(),
        }],
    )
    .unwrap();
    b = library::complete(&lib, &path, &b.chapters[0].id, true).unwrap();
    let ids = b.chapters[0].page_ids.clone();
    let mut chapters = b.chapters.clone();
    let split = chapters[0].page_ids.split_off(2);
    chapters.push(Chapter {
        id: uid(),
        title: "Part 2".into(),
        page_ids: split,
        read: true,
    });
    b = library::organize(&lib, &path, b.revision, chapters, vec![], vec![]).unwrap();
    assert!(b.chapters.iter().all(|c| c.read));
    let old = b.clone();
    let mut reordered = b.chapters.clone();
    reordered.reverse();
    b = library::organize(&lib, &path, b.revision, reordered, vec![], vec![]).unwrap();
    assert!(library::organize(&lib, &path, old.revision, old.chapters, vec![], vec![]).is_err());
    let stored = store::open(&path).unwrap();
    assert_eq!(stored.pages[0].id, ids[2]);
    for (i, p) in stored.pages.iter().enumerate() {
        assert_eq!(i, p.number);
    }
    assert_eq!(
        store::page(&path, &ids[0]).unwrap().regions[0].target,
        "保留"
    );
    assert!(store::cache_get(&path, "cached").unwrap().is_some());
    assert!(!store::save_page(&path, &mut p, revision + 1).unwrap());
    let mut moved = b.chapters.clone();
    let id = moved[1].page_ids.remove(0);
    moved[0].page_ids.push(id);
    b = library::organize(&lib, &path, b.revision, moved, vec![], vec![]).unwrap();
    assert!(!b.chapters[0].read);
    assert!(b.chapters[1].read);
    let mut merged = b.chapters.clone();
    let next = merged.remove(1);
    merged[0].page_ids.extend(next.page_ids);
    merged[0].read = false;
    b = library::organize(&lib, &path, b.revision, merged, vec![], vec![]).unwrap();
    assert!(!b.chapters[0].read);
    let mut invalid = b.chapters.clone();
    let duplicate = invalid[0].page_ids[0].clone();
    invalid[0].page_ids.push(duplicate);
    assert!(library::organize(&lib, &path, b.revision, invalid, vec![], vec![]).is_err());
    assert_eq!(library::open(&path).unwrap().revision, b.revision);
    library::delete(&lib, &path).unwrap();
    assert!(raw.join("0.png").exists());
    assert!(store::save_page(&path, &mut p, 0).is_err());
    assert!(!path.exists());
}
#[test]
fn current_format_copy_preserves_owned_assets_settings_jobs_and_external_source() {
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("original.png");
    image(&raw);
    let pages = documents::import(&[raw.to_string_lossy().into()]).unwrap();
    let source = t.path().join("current.umanga");
    let mut old = store::create(&source, "Current book", &pages).unwrap();
    old.settings.context_pages = 3;
    store::settings(&source, &old.settings).unwrap();
    let details = library::open(&source).unwrap();
    library::update(
        t.path(),
        &source,
        details.metadata,
        library::Overrides {
            translation: Some(old.settings.clone()),
            reader: None,
        },
        details.revision,
    )
    .unwrap();
    let asset = store::assets(&source).join("rendered.png");
    fs::copy(&raw, &asset).unwrap();
    let p = &mut old.pages[0];
    p.rendered = Some(asset.to_string_lossy().into());
    store::save_page(&source, p, 0).unwrap();
    store::cache_put(&source, "keep", &[]).unwrap();
    let job = Job {
        kind: JobKind::Translation,
        cleanup_model: None,
        steps: vec![],
        origin: umanga_core::job_state::JobOrigin::Explicit,
        settings_captured: true,
        attempt: 0,
        failure_kind: None,
        page_revision: None,
        fresh: false,
        glossary_checkpoint: None,
        model_directory: String::new(),
        id: uid(),
        project: source.to_string_lossy().into(),
        page_id: p.id.clone(),
        status: "paused".into(),
        stage: "translating".into(),
        stage_detail: "Batch 1 / 2".into(),
        error: None,
        created: now(),
        elapsed_ms: 0,
        device: "cpu".into(),
        settings: old.settings.clone(),
        provider: Default::default(),
    };
    store::save_job(&source, &job).unwrap();
    let original_version: String = store::connection(&source)
        .unwrap()
        .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let lib = t.path().join("managed");
    let b = library::import_book(&lib, &source).unwrap();
    assert_eq!(b.id, old.id);
    assert_eq!(b.chapters[0].page_ids[0], p.id);
    assert_eq!(b.overrides.translation.as_ref().unwrap().context_pages, 3);
    let new = Path::new(&b.path);
    let copied = store::page(new, &p.id).unwrap();
    assert_ne!(copied.rendered, p.rendered);
    assert!(Path::new(copied.rendered.as_ref().unwrap()).is_file());
    assert_eq!(copied.revision, p.revision);
    assert_eq!(store::jobs(new).unwrap()[0].id, job.id);
    assert!(store::cache_get(new, "keep").unwrap().is_some());
    assert_eq!(
        store::connection(&source)
            .unwrap()
            .query_row::<String, _, _>("SELECT value FROM meta WHERE key='version'", [], |r| r
                .get(0))
            .unwrap(),
        original_version
    );
    assert_eq!(store::page(&source, &p.id).unwrap().rendered, p.rendered);
    let cover = library::set_cover(&lib, new, Some(raw.to_str().unwrap())).unwrap();
    assert!(Path::new(cover.cover.as_ref().unwrap()).is_file());
    assert_eq!(library::open(new).unwrap().cover, cover.cover);
    assert!(library::set_cover(&lib, new, None).unwrap().cover.is_none());
    assert_eq!(library::import_book(&lib, &source).unwrap().id, b.id);
    assert_eq!(library::list(&lib).unwrap().len(), 1);
    let relocated = t.path().join("moved");
    fs::rename(&lib, &relocated).unwrap();
    let summary = library::list(&relocated).unwrap().remove(0);
    assert!(Path::new(&summary.path).is_file());
    assert!(library::open(Path::new(&summary.path)).is_ok());
}
#[tokio::test]
async fn paused_jobs_are_removed_before_deletion_and_late_responses_cannot_recreate() {
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("page.png");
    image(&raw);
    let lib = t.path().join("library");
    let b = library::create(&lib, meta(), scan(&raw), None).unwrap();
    let e = Engine::new(
        root().join("assets/models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("No live calls")),
    )
    .unwrap();
    let jobs = e
        .enqueue(
            &b.path,
            &b.chapters[0].page_ids,
            &ProviderProfile::default(),
        )
        .unwrap();
    e.quiesce(&b.path).await.unwrap();
    assert_eq!(e.list()[0].status, "paused");
    assert!(e.control(&jobs[0].id, "resume").is_err());
    assert!(
        e.enqueue(
            &b.path,
            &b.chapters[0].page_ids,
            &ProviderProfile::default()
        )
        .is_err()
    );
    library::delete(&lib, Path::new(&b.path)).unwrap();
    e.release_book(&b.path, None);
    assert!(e.list().is_empty());
    assert!(store::save_job(Path::new(&b.path), &jobs[0]).is_err());
    assert!(!Path::new(&b.path).exists());
    assert!(store::page(Path::new(&b.path), &jobs[0].page_id).is_err());
    assert!(store::jobs(Path::new(&b.path)).is_err());
    assert!(store::cache_get(Path::new(&b.path), "late").is_err());
    assert!(!Path::new(&b.path).exists());
    assert!(raw.exists());
}

#[tokio::test]
async fn active_mock_request_is_cancelled_before_atomic_organization() {
    use std::time::Duration;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({}))
                .set_delay(Duration::from_secs(10)),
        )
        .mount(&server)
        .await;
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("原稿.png");
    image(&raw);
    let lib = t.path().join("library");
    let b = library::create(&lib, meta(), scan(&raw), None).unwrap();
    let path = Path::new(&b.path);
    let mut p = store::page(path, &b.chapters[0].page_ids[0]).unwrap();
    p.regions = vec![Region {
        source: "こんにちは".into(),
        bbox: [5., 5., 55., 90.],
        overlay_only: true,
        ..Default::default()
    }];
    let revision = p.revision;
    assert!(store::save_page(path, &mut p, revision).unwrap());
    let settings = TranslationSettings {
        mode: "local".into(),
        ..Default::default()
    };
    store::settings(path, &settings).unwrap();
    let profile = ProviderProfile {
        endpoint: server.uri(),
        simple_translation: false,
        vision: false,
        model: "mock".into(),
        ..Default::default()
    };
    let e = Engine::new(
        root().join("assets/models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| Ok("mock-key".into())),
    )
    .unwrap();
    e.enqueue(&b.path, &b.chapters[0].page_ids, &profile)
        .unwrap();
    e.start();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            assert_ne!(e.list()[0].status, "failed", "{:?}", e.list()[0].error);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let start = std::time::Instant::now();
    e.quiesce(&b.path).await.unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    let mut chapters = b.chapters.clone();
    chapters[0].page_ids.clear();
    library::organize(&lib, path, b.revision, chapters, vec![], vec![]).unwrap();
    e.release_book(&b.path, Some(&[]));
    assert!(store::open(path).unwrap().pages.is_empty());
    assert!(store::jobs(path).unwrap().is_empty());
    assert!(e.list().is_empty());
    assert!(raw.is_file());
}
