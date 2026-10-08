use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use umanga_core::{
    documents,
    library::{self, Book, Chapter, ImportPreview, Metadata},
    pipeline::{BatchMode, Engine},
    store,
    types::*,
};

fn fixture() -> (tempfile::TempDir, Book, Arc<Engine>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let image = temp.path().join("original.png");
    image::RgbImage::new(64, 96).save(&image).unwrap();
    let base = documents::import(&[image.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let pages: Vec<_> = (0..5)
        .map(|number| Page {
            id: uid(),
            number,
            regions: vec![Region {
                source: format!("Original {number}"),
                target: if number == 0 {
                    "Manual translation".into()
                } else {
                    String::new()
                },
                bbox: [1., 1., 50., 60.],
                ..Default::default()
            }],
            ..base.clone()
        })
        .collect();
    let chapters = vec![
        Chapter {
            id: uid(),
            title: "One".into(),
            page_ids: pages[..3].iter().map(|p| p.id.clone()).collect(),
            read: false,
        },
        Chapter {
            id: uid(),
            title: "Two".into(),
            page_ids: pages[3..].iter().map(|p| p.id.clone()).collect(),
            read: false,
        },
    ];
    let book = library::create(
        &temp.path().join("library"),
        Metadata {
            title: "Omission".into(),
            ..Default::default()
        },
        ImportPreview {
            chapters,
            pages,
            omitted_page_ids: vec![],
            warnings: vec![],
        },
        None,
    )
    .unwrap();
    let engine = Engine::new(
        temp.path().join("models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Submission must not call a provider")),
    )
    .unwrap();
    (temp, book, engine)
}

fn organize(root: &Path, book: &Book, chapters: Vec<Chapter>, omitted: Vec<String>) -> Book {
    library::organize(
        &root.join("library"),
        Path::new(&book.path),
        book.revision,
        chapters,
        vec![],
        omitted,
    )
    .unwrap()
}

#[test]
fn omission_only_organization_does_not_invalidate_page_jobs() {
    let (temp, book, engine) = fixture();
    let path = Path::new(&book.path);
    let before = store::open(path).unwrap().pages;
    let preview = engine.preview_batch(&book.path, None).unwrap();
    let versions = preview
        .pages
        .iter()
        .map(|p| p.version.clone())
        .collect::<Vec<_>>();
    let submitted = engine
        .enqueue_batch(
            &book.path,
            &book.id,
            None,
            &versions,
            BatchMode::Replace,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            temp.path(),
        )
        .unwrap();
    organize(
        temp.path(),
        &book,
        book.chapters.clone(),
        vec![before[1].id.clone()],
    );
    let after = store::open(path).unwrap().pages;
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    for job in submitted.jobs {
        assert_eq!(
            job.page_revision,
            Some(store::page(path, &job.page_id).unwrap().revision)
        );
    }
}

#[test]
fn omission_follows_page_identity_and_survives_current_format_copy_and_relocation() {
    let (temp, mut book, _) = fixture();
    let ids = book
        .chapters
        .iter()
        .flat_map(|c| c.page_ids.clone())
        .collect::<Vec<_>>();
    let path = Path::new(&book.path).to_owned();
    let saved_target = store::page(&path, &ids[0]).unwrap().regions[0]
        .target
        .clone();
    book = organize(
        temp.path(),
        &book,
        book.chapters.clone(),
        vec![ids[1].clone(), ids[3].clone()],
    );
    assert_eq!(
        store::open(&path).unwrap().omitted_page_ids,
        book.omitted_page_ids
    );
    // Reorder, move, split and merge membership while retaining the same marks.
    let mut chapters = book.chapters.clone();
    chapters[0].page_ids.swap(0, 1);
    let moved = chapters[0].page_ids.remove(0);
    chapters[1].page_ids.push(moved);
    let split = chapters[1].page_ids.split_off(1);
    chapters.push(Chapter {
        id: uid(),
        title: "Split".into(),
        page_ids: split,
        read: false,
    });
    book = organize(temp.path(), &book, chapters, book.omitted_page_ids.clone());
    let mut chapters = book.chapters.clone();
    let split = chapters.pop().unwrap();
    chapters[1].page_ids.extend(split.page_ids);
    book = organize(temp.path(), &book, chapters, book.omitted_page_ids.clone());
    assert!(book.omitted_page_ids.contains(&ids[1]));
    assert!(book.omitted_page_ids.contains(&ids[3]));
    assert_eq!(
        store::page(&path, &ids[0]).unwrap().regions[0].target,
        saved_target
    );
    let copied = library::import_book(&temp.path().join("copied"), &path).unwrap();
    assert_eq!(copied.omitted_page_ids, book.omitted_page_ids);
    let relocated = temp.path().join("relocated.umanga");
    std::fs::copy(&copied.path, &relocated).unwrap();
    assert_eq!(
        library::open(&relocated).unwrap().omitted_page_ids,
        book.omitted_page_ids
    );
    let mut chapters = book.chapters.clone();
    for chapter in &mut chapters {
        chapter.page_ids.retain(|id| id != &ids[1]);
    }
    book = organize(temp.path(), &book, chapters, vec![ids[3].clone()]);
    assert_eq!(book.omitted_page_ids, vec![ids[3].clone()]);
    assert!(store::page(&path, &ids[1]).is_err());
}

#[test]
fn invalid_omission_and_stale_organization_roll_back_page_and_book_changes() {
    let (temp, book, _) = fixture();
    let path = Path::new(&book.path);
    let before = serde_json::to_value(store::open(path).unwrap()).unwrap();
    let id = book.chapters[0].page_ids[0].clone();
    for omitted in [vec![uid()], vec![id.clone(), id.clone()]] {
        assert!(
            library::organize(
                &temp.path().join("library"),
                path,
                book.revision,
                book.chapters.clone(),
                vec![],
                omitted
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(store::open(path).unwrap()).unwrap(),
            before
        );
        assert_eq!(library::open(path).unwrap().revision, book.revision);
    }
    let next = organize(temp.path(), &book, book.chapters.clone(), vec![id]);
    assert!(
        library::organize(
            &temp.path().join("library"),
            path,
            book.revision,
            book.chapters,
            vec![],
            vec![]
        )
        .is_err()
    );
    assert_eq!(
        library::open(path).unwrap().omitted_page_ids,
        next.omitted_page_ids
    );
}

#[test]
fn batch_modes_and_reader_skip_omission_but_explicit_page_request_overrides_it() {
    for mode in [BatchMode::SkipTranslated, BatchMode::Replace] {
        let (temp, book, engine) = fixture();
        let ids = book
            .chapters
            .iter()
            .flat_map(|c| c.page_ids.clone())
            .collect::<Vec<_>>();
        let book = organize(
            temp.path(),
            &book,
            book.chapters.clone(),
            vec![ids[0].clone(), ids[1].clone()],
        );
        let preview = engine.preview_batch(&book.path, None).unwrap();
        assert_eq!(preview.pages.iter().filter(|p| p.omitted).count(), 2);
        let versions = preview
            .pages
            .iter()
            .map(|p| p.version.clone())
            .collect::<Vec<_>>();
        let result = engine
            .enqueue_batch(
                &book.path,
                &book.id,
                None,
                &versions,
                mode,
                &ProviderProfile::default(),
                &TranslationSettings::default(),
                Path::new("models"),
            )
            .unwrap();
        assert_eq!(result.outcome.omitted, 2);
        assert_eq!(result.outcome.skipped, 0);
        assert_eq!(result.jobs.len(), 3);
        assert!(
            result
                .jobs
                .iter()
                .all(|j| !book.omitted_page_ids.contains(&j.page_id))
        );
        let reader = engine
            .enqueue(&book.path, &ids, &ProviderProfile::default())
            .unwrap();
        assert!(reader.is_empty()); // Included pages are already queued; omitted pages stay excluded.
        let page = store::page(Path::new(&book.path), &ids[0]).unwrap();
        let explicit = engine
            .enqueue_fresh(
                &book.path,
                &page.id,
                page.revision,
                true,
                &ProviderProfile::default(),
                &TranslationSettings::default(),
                Path::new("models"),
            )
            .unwrap();
        assert_eq!(explicit.len(), 1);
        assert_eq!(explicit[0].page_id, ids[0]);
        assert_eq!(
            library::open(Path::new(&book.path))
                .unwrap()
                .omitted_page_ids,
            book.omitted_page_ids
        );
        assert_eq!(
            store::page(Path::new(&book.path), &page.id)
                .unwrap()
                .regions[0]
                .target,
            "Manual translation"
        );
    }
}

#[test]
fn all_omitted_batches_are_noops_and_prior_originals_remain_context() {
    let (temp, book, engine) = fixture();
    let ids = book
        .chapters
        .iter()
        .flat_map(|c| c.page_ids.clone())
        .collect::<Vec<_>>();
    let book = organize(temp.path(), &book, book.chapters.clone(), ids.clone());
    assert!(
        engine
            .enqueue(&book.path, &ids, &ProviderProfile::default())
            .unwrap()
            .is_empty()
    );
    for chapter in [None, Some(book.chapters[0].id.as_str())] {
        let preview = engine.preview_batch(&book.path, chapter).unwrap();
        for mode in [BatchMode::SkipTranslated, BatchMode::Replace] {
            let versions = preview
                .pages
                .iter()
                .map(|p| p.version.clone())
                .collect::<Vec<_>>();
            let result = engine
                .enqueue_batch(
                    &book.path,
                    &book.id,
                    chapter,
                    &versions,
                    mode,
                    &ProviderProfile::default(),
                    &TranslationSettings::default(),
                    Path::new("models"),
                )
                .unwrap();
            assert!(result.jobs.is_empty());
            assert_eq!(result.outcome.omitted, preview.pages.len());
        }
    }
    let context = store::preceding_context(Path::new(&book.path), 4, 4).unwrap();
    assert_eq!(context.pages.len(), 4);
    assert!(context.for_llm().contains("Original 1"));
    assert!(context.for_llm().contains("Original 3"));
    assert_eq!(store::open(Path::new(&book.path)).unwrap().pages.len(), 5);
}

#[test]
fn stale_previews_cannot_enqueue_after_omission_changes_and_existing_jobs_are_retained() {
    let (temp, book, engine) = fixture();
    let id = book.chapters[0].page_ids[1].clone();
    let existing = engine
        .enqueue(
            &book.path,
            std::slice::from_ref(&id),
            &ProviderProfile::default(),
        )
        .unwrap();
    let before = engine.preview_batch(&book.path, None).unwrap();
    let book = organize(temp.path(), &book, book.chapters.clone(), vec![id]);
    let versions = before
        .pages
        .iter()
        .map(|p| p.version.clone())
        .collect::<Vec<_>>();
    let result = engine
        .enqueue_batch(
            &book.path,
            &book.id,
            None,
            &versions,
            BatchMode::Replace,
            &ProviderProfile::default(),
            &TranslationSettings::default(),
            Path::new("models"),
        )
        .unwrap();
    assert_eq!(result.jobs.len(), 4);
    assert_eq!(result.outcome.omitted, 1);
    assert_eq!(result.outcome.changed, 0);
    assert_eq!(
        store::jobs(Path::new(&book.path))
            .unwrap()
            .into_iter()
            .find(|j| j.id == existing[0].id)
            .unwrap()
            .id,
        existing[0].id
    );
    assert_eq!(
        store::jobs(Path::new(&book.path))
            .unwrap()
            .into_iter()
            .find(|j| j.id == existing[0].id)
            .unwrap()
            .status,
        existing[0].status
    );
}

#[test]
fn import_creation_saves_marks_and_previous_formats_are_rejected_without_deletion() {
    let (temp, book, _) = fixture();
    let project = store::open(Path::new(&book.path)).unwrap();
    let copied = library::create(
        &temp.path().join("created"),
        book.metadata.clone(),
        ImportPreview {
            chapters: book.chapters.clone(),
            pages: project.pages,
            omitted_page_ids: vec![book.chapters[0].page_ids[0].clone()],
            warnings: vec![],
        },
        None,
    )
    .unwrap();
    assert_eq!(copied.omitted_page_ids.len(), 1);
    let c = rusqlite::Connection::open(&copied.path).unwrap();
    c.execute("UPDATE meta SET value='16' WHERE key='version'", [])
        .unwrap();
    drop(c);
    assert!(
        library::open(Path::new(&copied.path))
            .unwrap_err()
            .to_string()
            .contains("requires format 20")
    );
    assert!(Path::new(&copied.path).is_file());
    let index = temp.path().join("created/library.sqlite");
    let c = rusqlite::Connection::open(&index).unwrap();
    c.pragma_update(None, "user_version", 16).unwrap();
    drop(c);
    assert!(
        library::list(&temp.path().join("created"))
            .unwrap_err()
            .to_string()
            .contains("Unsupported library format")
    );
    assert!(index.is_file());
}

#[test]
fn narrow_omission_save_merges_concurrent_choices_and_preserves_pages_and_metadata() {
    let (temp, book, _) = fixture();
    let path = Path::new(&book.path);
    let root = temp.path().join("library");
    let ids = &book.chapters[0].page_ids;
    let pages = serde_json::to_value(store::open(path).unwrap().pages).unwrap();
    library::update_omissions(&root, path, &[], &[ids[0].clone()]).unwrap();
    // A second stale Organizer changes a different mark; neither edit overwrites the other.
    let merged = library::update_omissions(&root, path, &[], &[ids[1].clone()]).unwrap();
    assert_eq!(merged.omitted_page_ids, ids[..2]);
    assert_eq!(
        serde_json::to_value(&merged.metadata).unwrap(),
        serde_json::to_value(&book.metadata).unwrap()
    );
    assert_eq!(
        serde_json::to_value(store::open(path).unwrap().pages).unwrap(),
        pages
    );
    let revision = merged.revision;
    let no_op = library::update_omissions(
        &root,
        path,
        &merged.omitted_page_ids,
        &merged.omitted_page_ids,
    )
    .unwrap();
    assert_eq!(no_op.revision, revision);
    assert!(library::update_omissions(&root, path, &[], &[uid()]).is_err());
    assert_eq!(library::open(path).unwrap().revision, revision);
}
