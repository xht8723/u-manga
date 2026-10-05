use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use umanga_core::{
    documents, library,
    pipeline::{BatchMode, BatchPreview, Engine, PageVersion},
    store,
    types::*,
};

fn fixture(count: usize) -> (tempfile::TempDir, Project, Arc<Engine>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = temp.path().join("original.png");
    image::RgbImage::new(32, 48).save(&source).unwrap();
    let base = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let pages = (0..count)
        .map(|i| Page {
            id: uid(),
            number: i,
            ..base.clone()
        })
        .collect::<Vec<_>>();
    let project = store::create(&temp.path().join("book.umanga"), "Book", &pages).unwrap();
    let engine = Engine::new(
        temp.path().join("no-models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Batch preview/submission must not access credentials or providers")),
    )
    .unwrap();
    (temp, project, engine)
}
fn versions(preview: &BatchPreview) -> Vec<PageVersion> {
    preview.pages.iter().map(|p| p.version.clone()).collect()
}
fn submit(
    engine: &Engine,
    project: &Project,
    preview: &BatchPreview,
    mode: BatchMode,
) -> umanga_core::pipeline::BatchSubmission {
    engine
        .enqueue_batch(
            &project.path,
            &preview.book_id,
            None,
            &versions(preview),
            mode,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            Path::new("captured-models"),
        )
        .unwrap()
}

#[test]
fn skip_means_any_nonwhitespace_translation_without_requiring_a_render() {
    let (_temp, project, engine) = fixture(4);
    for (i, target) in ["手工翻译", "partial", " \t\n\u{3000}", ""]
        .into_iter()
        .enumerate()
    {
        let mut p = project.pages[i].clone();
        p.regions = vec![Region {
            target: target.into(),
            source: "saved OCR".into(),
            bbox: [2., 2., 25., 30.],
            ..Default::default()
        }];
        store::save_page(Path::new(&project.path), &mut p, 0).unwrap();
    }
    let preview = engine.preview_batch(&project.path, None).unwrap();
    assert_eq!(
        preview
            .pages
            .iter()
            .map(|p| p.translated)
            .collect::<Vec<_>>(),
        [true, true, false, false]
    );
    assert!(preview.pages.iter().all(|p| p.has_work));
    assert!(engine.list().is_empty());
    let result = submit(&engine, &project, &preview, BatchMode::SkipTranslated);
    assert_eq!(result.outcome.skipped, 2);
    assert_eq!(
        result.jobs.iter().map(|j| &j.page_id).collect::<Vec<_>>(),
        project.pages[2..].iter().map(|p| &p.id).collect::<Vec<_>>()
    );
    assert!(result.jobs.iter().all(|j| !j.fresh));
    assert_eq!(
        store::page(Path::new(&project.path), &project.pages[0].id)
            .unwrap()
            .regions[0]
            .target,
        "手工翻译"
    );
}

#[test]
fn replace_captures_each_revision_and_does_not_reset_during_submission() {
    let (_temp, project, engine) = fixture(3);
    for i in 0..3 {
        let mut p = project.pages[i].clone();
        for _ in 0..=i {
            p.regions = vec![Region {
                source: "saved source".into(),
                target: "saved translation".into(),
                bbox: [2., 2., 25., 30.],
                ..Default::default()
            }];
            let revision = p.revision;
            store::save_page(Path::new(&project.path), &mut p, revision).unwrap();
        }
    }
    let preview = engine.preview_batch(&project.path, None).unwrap();
    let mut pages = versions(&preview);
    pages.reverse();
    let result = engine
        .enqueue_batch(
            &project.path,
            &preview.book_id,
            None,
            &pages,
            BatchMode::Replace,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            Path::new("captured-models"),
        )
        .unwrap();
    assert_eq!(
        result
            .jobs
            .iter()
            .map(|j| j.page_revision.unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        result.jobs.iter().map(|j| &j.page_id).collect::<Vec<_>>(),
        project.pages.iter().map(|p| &p.id).collect::<Vec<_>>()
    );
    assert!(result.jobs.iter().all(|j| j.fresh));
    for p in &project.pages {
        assert_eq!(
            store::page(Path::new(&project.path), &p.id)
                .unwrap()
                .regions[0]
                .target,
            "saved translation"
        );
    }
    assert!(
        store::jobs(Path::new(&project.path))
            .unwrap()
            .iter()
            .all(|j| j.fresh)
    );
}

#[test]
fn changed_and_missing_pages_are_skipped_independently() {
    let (_temp, project, engine) = fixture(3);
    let preview = engine.preview_batch(&project.path, None).unwrap();
    let mut p = project.pages[0].clone();
    store::save_page(Path::new(&project.path), &mut p, 0).unwrap();
    store::connection(Path::new(&project.path))
        .unwrap()
        .execute("DELETE FROM pages WHERE id=?1", [&project.pages[1].id])
        .unwrap();
    let result = submit(&engine, &project, &preview, BatchMode::Replace);
    assert_eq!(result.outcome.changed, 2);
    assert_eq!(result.jobs.len(), 1);
    assert_eq!(result.jobs[0].page_id, project.pages[2].id);
}

#[test]
fn waiting_paused_and_region_operations_do_not_block_other_eligible_pages() {
    let (_temp, project, engine) = fixture(4);
    let operation = engine
        .reserve_region(&uid(), &project.path, &project.pages[0].id)
        .unwrap();
    // Region requests own a page until their guard exits, including after global Stop.
    engine.control_all("stop").unwrap();
    let preview = engine.preview_batch(&project.path, None).unwrap();
    assert!(preview.pages[0].busy);
    let result = submit(&engine, &project, &preview, BatchMode::Replace);
    assert_eq!(result.outcome.busy, 1);
    assert!(result.outcome.held);
    assert_eq!(result.jobs.len(), 3);
    assert!(result.jobs.iter().all(|j| j.status == "paused"));
    assert!(
        engine
            .preview_batch(&project.path, None)
            .unwrap()
            .pages
            .iter()
            .all(|p| p.busy)
    );
    let again = submit(&engine, &project, &preview, BatchMode::Replace);
    assert!(again.jobs.is_empty());
    assert_eq!(again.outcome.busy, 4);
    drop(operation);
    engine.control_all("start").unwrap();
    let preview = engine.preview_batch(&project.path, None).unwrap();
    assert!(!preview.pages[0].busy);
    assert!(preview.pages[1..].iter().all(|p| p.busy));
}

#[test]
fn scope_is_validated_and_pages_added_after_preview_are_not_submitted() {
    let (_temp, project, engine) = fixture(3);
    let book = library::open(Path::new(&project.path)).unwrap();
    let chapter = &book.chapters[0];
    let preview = engine
        .preview_batch(&project.path, Some(&chapter.id))
        .unwrap();
    let only_first = vec![preview.pages[0].version.clone()];
    let result = engine
        .enqueue_batch(
            &project.path,
            &book.id,
            Some(&chapter.id),
            &only_first,
            BatchMode::Replace,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            Path::new("models"),
        )
        .unwrap();
    assert_eq!(result.jobs.len(), 1);
    assert!(
        engine
            .preview_batch(&project.path, Some("../outside"))
            .is_err()
    );
    assert!(engine.preview_batch(&project.path, Some(&uid())).is_err());
    assert!(
        engine
            .enqueue_batch(
                &project.path,
                &uid(),
                None,
                &[],
                BatchMode::Replace,
                &ProviderProfile::default(),
                &TranslationSettings::default(),
                Path::new("models")
            )
            .is_err()
    );
    let duplicate = vec![preview.pages[0].version.clone(); 2];
    assert!(
        engine
            .enqueue_batch(
                &project.path,
                &book.id,
                None,
                &duplicate,
                BatchMode::Replace,
                &ProviderProfile::default(),
                &TranslationSettings::default(),
                Path::new("models")
            )
            .is_err()
    );
}

#[test]
fn transient_mode_contract_rejects_unrecognized_intents() {
    assert_eq!(
        serde_json::to_string(&BatchMode::SkipTranslated).unwrap(),
        "\"skip_translated\""
    );
    assert!(serde_json::from_str::<BatchMode>("\"skip\"").is_err());
    let version: PageVersion =
        serde_json::from_str("{\"id\":\"identity\",\"revision\":7}").unwrap();
    assert_eq!(version.revision, 7);
}

#[test]
fn storage_failure_rolls_back_the_entire_batch_before_dispatch() {
    let (_temp, project, engine) = fixture(3);
    let preview = engine.preview_batch(&project.path, None).unwrap();
    let connection = store::connection(Path::new(&project.path)).unwrap();
    connection.execute_batch(&format!(
        "CREATE TRIGGER reject_batch BEFORE INSERT ON jobs WHEN json_extract(NEW.data,'$.pageId')='{}' BEGIN SELECT RAISE(ABORT,'injected write failure'); END;",
        project.pages[1].id
    )).unwrap();
    let result = engine.enqueue_batch(
        &project.path,
        &preview.book_id,
        None,
        &versions(&preview),
        BatchMode::Replace,
        &ProviderProfile::default(),
        &TranslationSettings::default(),
        Path::new("models"),
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("injected write failure")
    );
    assert!(store::jobs(Path::new(&project.path)).unwrap().is_empty());
    assert!(engine.list().is_empty());
    for p in &project.pages {
        assert_eq!(
            store::page(Path::new(&project.path), &p.id)
                .unwrap()
                .revision,
            0
        );
    }
}

#[test]
fn chapter_batches_cannot_retarget_pages_moved_to_another_chapter() {
    let (_temp, project, engine) = fixture(3);
    let path = Path::new(&project.path);
    let book = library::open(path).unwrap();
    let chapter_id = book.chapters[0].id.clone();
    let preview = engine
        .preview_batch(&project.path, Some(&chapter_id))
        .unwrap();
    let mut updated = book;
    updated.chapters[0].page_ids.remove(1);
    updated.chapters.push(library::Chapter {
        id: uid(),
        title: "Other".into(),
        page_ids: vec![project.pages[1].id.clone()],
        read: false,
    });
    store::connection(path)
        .unwrap()
        .execute(
            "UPDATE meta SET value=?1 WHERE key='book'",
            [serde_json::to_string(&updated).unwrap()],
        )
        .unwrap();
    let result = engine
        .enqueue_batch(
            &project.path,
            &preview.book_id,
            Some(&chapter_id),
            &versions(&preview),
            BatchMode::Replace,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            Path::new("models"),
        )
        .unwrap();
    assert_eq!(result.outcome.changed, 1);
    assert_eq!(
        result.jobs.iter().map(|j| &j.page_id).collect::<Vec<_>>(),
        vec![&project.pages[0].id, &project.pages[2].id]
    );
}

#[test]
fn thousand_page_batch_with_ten_thousand_history_records() {
    let (temp, project, engine) = fixture(1000);
    let template = engine
        .enqueue(
            &project.path,
            std::slice::from_ref(&project.pages[0].id),
            &ProviderProfile::default(),
        )
        .unwrap()
        .remove(0);
    engine.control(&template.id, "cancel").unwrap();
    let history = (0..10000)
        .map(|i| Job {
            id: uid(),
            page_id: project.pages[i % 1000].id.clone(),
            status: "complete".into(),
            stage: "done".into(),
            ..template.clone()
        })
        .collect::<Vec<_>>();
    store::save_jobs(Path::new(&project.path), &history).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine = Engine::new(
        temp.path().join("models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("No provider calls")),
    )
    .unwrap();
    let original_hash = store::file_hash(Path::new(&project.pages[0].source.path)).unwrap();
    let started = std::time::Instant::now();
    let preview = engine.preview_batch(&project.path, None).unwrap();
    let recovery_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(preview.pages.len(), 1000);
    let started = std::time::Instant::now();
    let preview = engine.preview_batch(&project.path, None).unwrap();
    let warm_preview_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = std::time::Instant::now();
    let result = submit(&engine, &project, &preview, BatchMode::Replace);
    let submit_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(result.jobs.len(), 1000);
    assert_eq!(engine.list().len(), 11001);
    assert_eq!(store::jobs(Path::new(&project.path)).unwrap().len(), 11001);
    assert_eq!(
        store::file_hash(Path::new(&project.pages[0].source.path)).unwrap(),
        original_hash
    );
    let output = root.join("test-output/batch-translation");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("native-load.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "passed": true, "pages":1000, "existingJobs":10001, "newJobs":1000,
        "recoveryAndPreviewMs":recovery_ms,"warmPreviewMs":warm_preview_ms,"submissionMs":submit_ms,
        "debugBuild":cfg!(debug_assertions),"scope":"Core projection/recovery and durable submission; excludes IPC/readiness/provider execution",
        "providerCalls":0,"originalUnchanged":true
    })).unwrap()).unwrap();
}
