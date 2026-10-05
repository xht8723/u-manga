use std::{fs, path::Path};
use umanga_core::{
    documents,
    library::{self, ImportPreview, Metadata},
    store,
    types::*,
};
fn engine() -> std::sync::Arc<umanga_core::pipeline::Engine> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    umanga_core::pipeline::Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        std::sync::Arc::new(|_| {}),
        std::sync::Arc::new(|_| anyhow::bail!("No service calls")),
    )
    .unwrap()
}

fn fixture(root: &Path) -> library::Book {
    let original = root.join("source.png");
    image::RgbImage::from_pixel(64, 96, image::Rgb([248, 248, 248]))
        .save(&original)
        .unwrap();
    let pages = documents::import(&[original.to_string_lossy().into()]).unwrap();
    let chapter = library::Chapter {
        id: uid(),
        title: "Chapter".into(),
        page_ids: pages.iter().map(|p| p.id.clone()).collect(),
        read: false,
    };
    library::create(
        &root.join("library"),
        Metadata {
            title: "Audit".into(),
            ..Default::default()
        },
        ImportPreview {
            omitted_page_ids: vec![],
            pages,
            chapters: vec![chapter],
            warnings: vec![],
        },
        None,
    )
    .unwrap()
}

#[test]
fn organization_preserves_a_new_source_linked_to_a_removed_pages_render() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let old_id = &b.chapters[0].page_ids[0];
    let generated = store::assets(path).join(format!("{old_id}-{}.png", uid()));
    image::RgbImage::from_pixel(64, 96, image::Rgb([240, 240, 240]))
        .save(&generated)
        .unwrap();
    let bytes = fs::read(&generated).unwrap();
    let added = documents::import(&[generated.to_string_lossy().into()]).unwrap();
    let mut chapters = b.chapters.clone();
    chapters[0].page_ids = vec![added[0].id.clone()];
    let committed = library::organize(
        &t.path().join("library"),
        path,
        b.revision,
        chapters,
        added,
        vec![],
    )
    .unwrap();
    assert_eq!(
        fs::read(&generated).unwrap(),
        bytes,
        "a linked original must survive generated-asset retirement"
    );
    documents::source_health(&store::page(path, &committed.chapters[0].page_ids[0]).unwrap())
        .unwrap();
}

#[test]
fn organization_reports_committed_membership_when_retirement_cannot_run() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let assets = store::assets(path);
    fs::remove_dir_all(&assets).unwrap();
    // A non-directory at the managed assets path simulates a retirement failure after commit.
    fs::write(&assets, "occupied").unwrap();
    let mut chapters = b.chapters.clone();
    chapters[0].page_ids.clear();
    let result = library::organize(
        &t.path().join("library"),
        path,
        b.revision,
        chapters,
        vec![],
        vec![],
    );
    assert!(result.is_ok(), "membership was committed: {result:?}");
    let result = result.unwrap();
    assert!(result.chapters[0].page_ids.is_empty());
    assert!(
        !result.warnings.is_empty(),
        "retirement must remain actionable"
    );
    assert!(library::open(path).unwrap().chapters[0].page_ids.is_empty());
}

#[test]
fn unicode_blank_translations_agree_in_queries_and_library_counts() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let mut page = store::page(path, &b.chapters[0].page_ids[0]).unwrap();
    page.regions = vec![Region {
        id: uid(),
        bbox: [1., 1., 20., 30.],
        target: "\t\n\u{3000}\u{2003}\u{a0}".into(),
        ..Default::default()
    }];
    page.rendered = Some("test.png".into());
    let c = store::connection(path).unwrap();
    c.execute(
        "UPDATE pages SET data=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&page).unwrap(), page.id],
    )
    .unwrap();
    assert!(store::translated_page_ids(path).unwrap().is_empty());
    assert!(store::cleanup_page_ids(path).unwrap().is_empty());
    assert_eq!(
        library::refresh(&t.path().join("library"), path)
            .unwrap()
            .translated,
        0
    );
    page.regions[0].prepared = true;
    c.execute(
        "UPDATE pages SET data=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&page).unwrap(), page.id],
    )
    .unwrap();
    assert!(store::cleanup_page_ids(path).unwrap().contains(&page.id));
    assert!(store::translated_page_ids(path).unwrap().is_empty());
}

#[test]
fn invalid_job_rows_do_not_hide_valid_history_and_never_enter_dispatch() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let e = engine();
    let good = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let c = store::connection(Path::new(&b.path)).unwrap();
    c.execute("INSERT INTO jobs VALUES(?1,?2)", [uid(), "not JSON".into()])
        .unwrap();
    let mut hostile = good.clone();
    hostile.id = uid();
    hostile.page_id = uid();
    store::save_job(Path::new(&b.path), &hostile).unwrap();
    let restarted = engine();
    assert!(
        restarted.recover(&b.path).is_err(),
        "invalid rows need visible diagnostics"
    );
    let visible = restarted.list();
    assert_eq!(
        visible.len(),
        1,
        "valid history survives neighboring corruption"
    );
    assert_eq!(visible[0].id, good.id);
    assert_eq!(visible[0].status, "paused");
    assert_eq!(
        c.query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        3,
        "invalid rows must remain untouched"
    );
}

#[test]
fn import_rejects_mismatched_job_identity_before_copying() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let e = engine();
    let job = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let c = store::connection(Path::new(&b.path)).unwrap();
    c.execute(
        "UPDATE jobs SET id=?1 WHERE id=?2",
        rusqlite::params![uid(), job.id],
    )
    .unwrap();
    let target = t.path().join("imported");
    assert!(library::import_book(&target, Path::new(&b.path)).is_err());
    assert!(library::list(&target).unwrap().is_empty());
    assert_eq!(
        c.query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, usize>(0))
            .unwrap(),
        1
    );
    assert!(
        !fs::read_dir(target).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "umanga"))
    );
}

#[test]
fn dimensions_rejects_excessive_geometry_before_decoding_pixels() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("huge.png");
    let mut output = fs::File::create(&file).unwrap();
    // A tiny valid PNG header is enough to inspect geometry; no giant buffer is created.
    let encoder = image::codecs::png::PngEncoder::new(&mut output);
    use image::ImageEncoder;
    encoder
        .write_image(&[0, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    drop(output);
    let mut bytes = fs::read(&file).unwrap();
    bytes[16..20].copy_from_slice(&10001u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&10001u32.to_be_bytes());
    // CRC32 for the IHDR chunk, without introducing a new dependency.
    let mut crc = !0u32;
    for &byte in &bytes[12..29] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
    fs::File::create(&file).unwrap().write_all(&bytes).unwrap();
    let source = Source {
        path: file.to_string_lossy().into(),
        kind: "image".into(),
        entry: None,
        index: 0,
    };
    assert!(
        documents::dimensions(&source).is_err(),
        "unsafe geometry must be rejected at admission"
    );
}

#[test]
fn materialization_rebuilds_a_corrupt_cache_without_changing_originals() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let page = store::page(path, &b.chapters[0].page_ids[0]).unwrap();
    let original = fs::read(&page.source.path).unwrap();
    let cached = documents::materialize(path, &page).unwrap();
    fs::write(&cached, b"corrupt").unwrap();
    let restored = documents::materialize(path, &page).unwrap();
    assert!(
        image::open(restored).is_ok(),
        "a success stamp cannot authorize corrupt pixels"
    );
    assert_eq!(fs::read(&page.source.path).unwrap(), original);
}

#[test]
fn recovery_pause_write_failure_retains_visible_held_job() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let e = engine();
    let job = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let c = store::connection(Path::new(&b.path)).unwrap();
    c.execute_batch("CREATE TRIGGER reject_pause BEFORE INSERT ON jobs WHEN json_extract(NEW.data,'$.status')='paused' BEGIN SELECT RAISE(FAIL,'injected pause-write failure'); END;").unwrap();
    let resumed = engine();
    assert!(resumed.recover(&b.path).is_err());
    let visible = resumed.list();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, job.id);
    assert_eq!(visible[0].status, "paused");
    assert!(visible[0].error.is_some());
    assert_eq!(store::jobs(Path::new(&b.path)).unwrap()[0].status, "queued");
}

#[test]
fn conflicting_job_ids_and_invalid_captured_settings_are_reported() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let first = fixture(a.path());
    let second = fixture(b.path());
    let e = engine();
    let job = e
        .enqueue(
            &first.path,
            &[first.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let mut collision = job.clone();
    collision.project = second.path.clone();
    collision.page_id = second.chapters[0].page_ids[0].clone();
    store::save_job(Path::new(&second.path), &collision).unwrap();
    let resumed = engine();
    resumed.recover(&first.path).unwrap();
    assert!(resumed.recover(&second.path).is_err());
    assert_eq!(resumed.list().len(), 1);
    let mut malformed = collision;
    malformed.id = uid();
    malformed.settings.context_pages = 100;
    store::save_job(Path::new(&second.path), &malformed).unwrap();
    let rows = store::job_rows(&store::connection(Path::new(&second.path)).unwrap()).unwrap();
    assert_eq!(rows.invalid, 1);
    assert!(rows.errors[0].contains("settings"));
}

#[test]
fn provisional_outputs_and_scoped_references_preserve_shared_assets() {
    use umanga_core::assets;
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let image = image::DynamicImage::new_rgb8(64, 96);
    let failed = store::assets(path).join(format!("cover-{}.png", uid()));
    {
        let _output = assets::ProvisionalOutput::image(path, &failed, &image).unwrap();
        assert!(failed.exists());
    }
    assert!(!failed.exists(), "uncommitted new output must retire");
    let shared = store::assets(path).join(format!("cover-{}.png", uid()));
    image.save(&shared).unwrap();
    {
        let _existing = assets::ProvisionalOutput::image(path, &shared, &image).unwrap();
    }
    assert!(
        shared.exists(),
        "pre-existing output is not owned by a failed attempt"
    );
    let (_, references) = assets::protect_snapshot(|| Ok(((), vec![shared.clone()]))).unwrap();
    assets::collect(path, None).unwrap();
    assert!(shared.exists());
    // Keeping this snapshot must not take a global inference-length read lock.
    let other = tempfile::tempdir().unwrap();
    let other_book = fixture(other.path());
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        send.send(assets::collect(Path::new(&other_book.path), None))
            .unwrap()
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    drop(references);
    assets::collect(path, None).unwrap();
    assert!(!shared.exists());
}

#[test]
fn failed_stamp_publication_and_concurrent_materialization_recover() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let page = store::page(path, &b.chapters[0].page_ids[0]).unwrap();
    let original = fs::read(&page.source.path).unwrap();
    let stamp = umanga_core::safety::page_asset(path, &page.id, "stamp").unwrap();
    fs::create_dir(&stamp).unwrap();
    assert!(documents::materialize(path, &page).is_err());
    fs::remove_dir(&stamp).unwrap();
    let paths = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| documents::materialize(path, &page).unwrap()))
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(paths.iter().all(|p| p == &paths[0]));
    assert!(image::open(&paths[0]).is_ok());
    assert_eq!(fs::read(&page.source.path).unwrap(), original);
    assert!(!fs::read_dir(store::assets(path)).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "partial" || e == "tmp")
    }));
}

#[test]
fn location_lookup_failure_is_retryable_and_projection_keeps_receipt_warnings() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let e = engine();
    let mut job = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let absent = path.with_extension("held");
    fs::rename(path, &absent).unwrap();
    let mut cache = umanga_core::queue::LocationCache::default();
    let runtime = || {
        vec![umanga_core::job_view::RuntimeView {
            job: umanga_core::job_view::JobView::from_job(&job),
            sequence: 1,
            stopping: false,
            timer_running: false,
        }]
    };
    assert!(cache.describe_views(runtime())[0].location.is_none());
    assert_eq!(cache.errors().len(), 1);
    fs::rename(absent, path).unwrap();
    cache.invalidate(&b.path);
    assert!(cache.describe_views(runtime())[0].location.is_some());
    assert!(cache.errors().is_empty());
    job.error = Some("Injected storage warning".into());
    job.fresh = true;
    job.settings.glossary.push(GlossaryEntry {
        source: "private term".into(),
        target: "private translation".into(),
    });
    let receipt = umanga_core::queue::submission(vec![job], true);
    let value = serde_json::to_value(receipt).unwrap();
    assert_eq!(value["held"], true);
    assert_eq!(value["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(value["jobs"][0]["fresh"], true);
    assert!(value["jobs"][0]["settings"].get("glossary").is_none());
    assert!(value["jobs"][0].get("modelDirectory").is_none());
}

#[test]
fn maximum_escaped_glossaries_survive_enqueue_recovery_and_import() {
    for control in ['\n', '\u{1}'] {
        let t = tempfile::tempdir().unwrap();
        let mut b = fixture(t.path());
        let book_path = std::path::PathBuf::from(&b.path);
        let path = book_path.as_path();
        let target = format!(
            "x{}x",
            control
                .to_string()
                .repeat(umanga_core::glossary::MAX_FILE_BYTES - 3)
        );
        let entries = vec![GlossaryEntry {
            source: "A".into(),
            target,
        }];
        b.glossary.enabled = true;
        b.glossary.entries = entries.clone();
        b = library::save_glossary(
            &t.path().join("library"),
            path,
            b.glossary.revision,
            b.glossary,
        )
        .unwrap();
        let settings = library::effective(&b, &AppSettings::default());
        let e = engine();
        let mut job = e
            .enqueue_configured(
                &b.path,
                &[b.chapters[0].page_ids[0].clone()],
                &ProviderProfile::default(),
                &settings,
            )
            .unwrap()
            .remove(0);
        let mut page = store::page(path, &job.page_id).unwrap();
        job.page_revision = Some(page.revision + 1);
        let expected = page.revision;
        assert!(store::save_checkpoint(path, &mut page, expected, &job).unwrap());
        assert!(serde_json::to_string(&job).unwrap().len() > 8 * 1024 * 1024);
        let rows = store::job_rows(&store::connection(Path::new(&b.path)).unwrap()).unwrap();
        assert_eq!(
            rows.invalid, 0,
            "accepted glossary must not strand its durable job"
        );
        assert_eq!(rows.jobs[0].settings.glossary, entries);
        let restarted = engine();
        restarted.recover(&b.path).unwrap();
        assert_eq!(restarted.list()[0].status, "paused");
        assert_eq!(restarted.list()[0].settings.glossary, entries);
        let imported =
            library::import_book(&t.path().join("imported"), Path::new(&b.path)).unwrap();
        assert_eq!(
            store::jobs(Path::new(&imported.path)).unwrap()[0]
                .settings
                .glossary,
            entries
        );
    }
}

#[test]
fn oversized_non_glossary_job_data_rolls_back_the_whole_batch() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let e = engine();
    let original = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let mut first = original.clone();
    first.id = uid();
    let mut too_large = original.clone();
    too_large.id = uid();
    too_large.stage_detail = "x".repeat(8 * 1024 * 1024);
    let path = Path::new(&b.path);
    assert!(
        store::save_jobs(path, &[first, too_large])
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    let jobs = store::jobs(path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, original.id);
}

#[test]
fn failed_owned_exports_retire_partials_and_preserve_destinations() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let mut project = store::open(Path::new(&b.path)).unwrap();
    project.pages[0].rendered = Some(t.path().join("missing-render.png").to_string_lossy().into());
    for format in ["cbz", "pdf"] {
        let destination = t.path().join(format!("old-export.{format}"));
        fs::write(&destination, b"existing export").unwrap();
        // An unrelated partial must never be mistaken for this attempt's owned file.
        let unrelated = t.path().join("another-export.partial");
        fs::write(&unrelated, b"another attempt").unwrap();
        assert!(documents::export(&project, &destination, format).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"existing export");
        let partials = fs::read_dir(t.path())
            .unwrap()
            .filter_map(|e| {
                let path = e.unwrap().path();
                path.extension()
                    .is_some_and(|e| e == "partial")
                    .then_some(path)
            })
            .collect::<Vec<_>>();
        assert_eq!(partials, vec![unrelated]);
    }
}

#[test]
fn checkpoint_size_admission_preserves_page_and_job_atomically() {
    let t = tempfile::tempdir().unwrap();
    let b = fixture(t.path());
    let path = Path::new(&b.path);
    let e = engine();
    let original = e
        .enqueue(
            &b.path,
            &[b.chapters[0].page_ids[0].clone()],
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    let mut page = store::page(path, &original.page_id).unwrap();
    let before = serde_json::to_string(&page).unwrap();
    let expected = page.revision;
    let mut job = original.clone();
    job.page_revision = Some(expected + 1);
    job.error = Some("x".repeat(3 * 1024 * 1024));
    job.steps = vec![JobStep {
        stage: "translating".into(),
        status: "failed".into(),
        detail: String::new(),
        error: Some("x".repeat(3 * 1024 * 1024)),
    }];
    umanga_core::job_validation::validate(&job.id, &job, expected + 1, &Default::default())
        .unwrap();
    assert!(
        store::save_job(path, &job)
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    assert!(
        store::save_checkpoint(path, &mut page, expected, &job)
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    assert_eq!(serde_json::to_string(&page).unwrap(), before);
    assert_eq!(
        serde_json::to_string(&store::page(path, &page.id).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        serde_json::to_string(&store::jobs(path).unwrap()[0]).unwrap(),
        serde_json::to_string(&original).unwrap()
    );
    // A stale writer retains the existing conflict result even with an overbudget row.
    job.page_revision = Some(expected + 2);
    assert!(!store::save_checkpoint(path, &mut page, expected + 1, &job).unwrap());
    job.page_revision = Some(expected + 1);
    job.error = None;
    job.steps.clear();
    assert!(store::save_checkpoint(path, &mut page, expected, &job).unwrap());
    assert_eq!(page.revision, expected + 1);
    assert_eq!(
        store::jobs(path).unwrap()[0].page_revision,
        Some(expected + 1)
    );
}
