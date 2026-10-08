//! Isolated HTTP acceptance/performance fixture. No user books, models, or service calls.
use anyhow::Result;
use std::path::PathBuf;
use umanga_core::{documents, library, store, types::*};
fn main() -> Result<()> {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = app
        .join("test-output/local-hosting")
        .join(format!("native-{}", uid()));
    std::fs::create_dir_all(&root)?;
    let source = root.join("original.png");
    let mut image = image::RgbImage::from_pixel(800, 1200, image::Rgb([249, 248, 242]));
    for y in 200..320 {
        for x in 130..160 {
            image.put_pixel(x, y, image::Rgb([40, 40, 40]));
        }
    }
    for y in 410..480 {
        for x in 380..395 {
            image.put_pixel(x, y, image::Rgb([50, 50, 50]));
        }
    }
    image.save(&source)?;
    let library = root.join("library");
    let mut books = Vec::new();
    for n in 0..5 {
        let mut pages = Vec::new();
        for i in 0..3 {
            let mut p = documents::import(&[source.to_string_lossy().into()])?.remove(0);
            p.id = uid();
            p.number = i;
            p.regions = vec![
                Region {
                    id: "dialogue".into(),
                    bbox: [90., 150., 230., 360.],
                    bubble: Some([70., 130., 250., 380.]),
                    source: "こんにちは".into(),
                    target: if i == 0 {
                        "你好".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                },
                Region {
                    id: "narration-2".into(),
                    bbox: [340., 380., 500., 560.],
                    bubble: Some([320., 360., 520., 580.]),
                    source: "城へ行こう".into(),
                    target: if i == 0 {
                        "去城堡吧".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                },
            ];
            pages.push(p);
        }
        let b = library::create(
            &library,
            library::Metadata {
                title: format!("Hosted manga {}", n + 1),
                language: "ja".into(),
                ..Default::default()
            },
            library::ImportPreview {
                omitted_page_ids: vec![],
                chapters: vec![library::Chapter {
                    id: uid(),
                    title: "Chapter 1".into(),
                    page_ids: pages.iter().map(|p| p.id.clone()).collect(),
                    read: false,
                }],
                pages,
                warnings: vec![],
            },
            None,
        )?;
        books.push(serde_json::json!({"id":b.id,"path":b.path,"chapterId":b.chapters[0].id,"pageIds":b.chapters[0].page_ids}));
    }
    let start = std::time::Instant::now();
    // Real current-format empty books exercise the library index without image preloading.
    // Clone a valid empty current-format database and publish its index in one
    // fixture transaction. Production import/organization scans are not timed.
    let template = root.join("empty-template.umanga");
    store::create(&template, "Empty template", &[])?;
    let template_book = library::open(&template)?;
    let mut index = rusqlite::Connection::open(library.join("library.sqlite"))?;
    let tx = index.transaction()?;
    for n in 5..1000 {
        let mut b = template_book.clone();
        b.id = uid();
        b.metadata.title = format!("Library item {n:04}");
        let path = library.join(format!("{}.umanga", uid()));
        b.path = path.to_string_lossy().into();
        std::fs::copy(&template, &path)?;
        let mut db = rusqlite::Connection::open(&path)?;
        let t = db.transaction()?;
        for (key, value) in [
            ("id", b.id.clone()),
            ("title", b.metadata.title.clone()),
            ("book", serde_json::to_string(&b)?),
        ] {
            t.execute(
                "UPDATE meta SET value=?1 WHERE key=?2",
                rusqlite::params![value, key],
            )?;
        }
        t.commit()?;
        let summary = library::BookSummary {
            path: b.path,
            id: b.id,
            metadata: b.metadata,
            chapters: 0,
            completed: 0,
            pages: 0,
            translated: 0,
            review: 0,
            cover: None,
            cover_page: None,
            updated: now(),
        };
        tx.execute(
            "INSERT INTO books VALUES(?1,?2)",
            rusqlite::params![summary.id, serde_json::to_string(&summary)?],
        )?;
    }
    tx.commit()?;
    anyhow::ensure!(
        library::list(&library)?.len() == 1000,
        "Fixture index is incomplete"
    );
    let history_book = books[0]["path"].as_str().unwrap();
    let mut settings = AppSettings::default();
    settings.setup.completed = true;
    settings.setup.step = "review".into();
    settings.library_directory = library.to_string_lossy().into();
    settings.translation.cleanup.method = CleanupMethod::Solid;
    settings.translation.mode = "vision".into();
    settings.ui_language = UiLanguage::English;
    let p = store::page(
        std::path::Path::new(history_book),
        books[0]["pageIds"][0].as_str().unwrap(),
    )?;
    let jobs: Vec<_> = (0..10_000)
        .map(|_| Job {
            id: uid(),
            project: history_book.into(),
            page_id: p.id.clone(),
            status: "cancelled".into(),
            stage: "loading".into(),
            stage_detail: String::new(),
            error: None,
            created: now(),
            elapsed_ms: 0,
            device: String::new(),
            settings: settings.translation.clone(),
            provider: ProviderProfile::default(),
            kind: JobKind::Translation,
            cleanup_model: None,
            model_directory: settings.model_directory().to_string_lossy().into(),
            steps: vec![],
            origin: umanga_core::job_state::JobOrigin::Explicit,
            settings_captured: true,
            attempt: 0,
            failure_kind: None,
            page_revision: Some(p.revision),
            fresh: false,
            glossary_checkpoint: None,
        })
        .collect();
    store::save_jobs(std::path::Path::new(history_book), &jobs)?;
    store::atomic_write(
        &root.join("preferences.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )?;
    let info = serde_json::json!({"root":root,"library":library,"original":source,"books":books,"bookCount":1000,"historyCount":10_000,"fixtureMs":start.elapsed().as_millis()});
    store::atomic_write(
        &app.join("test-output/local-hosting/fixture.json"),
        &serde_json::to_vec_pretty(&info)?,
    )?;
    println!("{info}");
    Ok(())
}
