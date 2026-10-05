//! Opt-in packaged-app regression harness. Requires an isolated, explicitly marked data directory.
use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub fn requested() -> bool {
    std::env::args().any(|arg| arg == "--jobs-smoke")
}
pub fn trace(message: &str) {
    if requested()
        && let Ok(root) = output()
    {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("startup-trace.txt"))
        {
            let _ = writeln!(file, "{message}");
        }
    }
}
fn output() -> anyhow::Result<PathBuf> {
    let root = PathBuf::from(std::env::var("U_MANGA_DATA_DIR")?);
    anyhow::ensure!(
        root.is_absolute()
            && std::fs::read_to_string(root.join("ALLOW_JOBS_SMOKE"))?.trim() == "synthetic-only",
        "Jobs smoke requires a marked isolated data directory"
    );
    Ok(root)
}
pub fn prepare() -> anyhow::Result<AppSettings> {
    let root = output()?;
    let library = root.join("library");
    std::fs::create_dir_all(&library)?;
    let picture = root.join("synthetic.png");
    image::RgbImage::from_pixel(16, 24, image::Rgb([255, 255, 255])).save(&picture)?;
    let base = documents::import(&[picture.to_string_lossy().into()])?.remove(0);
    let pages: Vec<_> = (0..1000)
        .map(|i| {
            let mut p = base.clone();
            p.id = uid();
            p.number = i;
            p.regions = vec![Region {
                id: "synthetic-dialogue".into(),
                bbox: [2., 2., 10., 18.],
                target: "测试".into(),
                ..Default::default()
            }];
            p.source.path = root
                .join("missing-synthetic-source.png")
                .to_string_lossy()
                .into();
            p
        })
        .collect();
    let path = library.join(format!("{}.umanga", uid()));
    store::create(&path, "Jobs performance fixture", &pages)?;
    let provider = ProviderProfile {
        endpoint: "https://example.invalid".into(),
        ..Default::default()
    };
    let history: Vec<_> = (0..10000)
        .map(|i| Job {
            kind: JobKind::Translation,
            cleanup_model: None,
            model_directory: String::new(),
            steps: vec![],
            page_revision: None,
            fresh: false,
            glossary_checkpoint: None,
            id: uid(),
            project: path.to_string_lossy().into(),
            page_id: pages[i % 1000].id.clone(),
            status: "complete".into(),
            stage: "done".into(),
            stage_detail: String::new(),
            error: None,
            created: now(),
            elapsed_ms: 100,
            device: String::new(),
            settings: TranslationSettings::default(),
            provider: provider.clone(),
        })
        .collect();
    store::save_jobs(&path, &history)?;
    umanga_core::library::refresh(&library, &path)?;
    store::atomic_write(
        &root.join("fixture.json"),
        &serde_json::to_vec(
            &serde_json::json!({"path":path,"ids":pages.iter().map(|p|p.id.clone()).collect::<Vec<_>>()}),
        )?,
    )?;
    let mut settings = AppSettings {
        library_directory: library.to_string_lossy().into(),
        ..Default::default()
    };
    settings.setup.completed = true;
    settings.setup.step = "review".into();
    settings.appearance.active = "night".into();
    Ok(settings)
}
pub fn launch(app: tauri::AppHandle) {
    if !requested() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = run(app.clone()).await;
        if let Err(error) = result
            && let Ok(root) = output()
        {
            let _ = store::atomic_write(
                &root.join("jobs-smoke.json"),
                serde_json::json!({"error":error.to_string()})
                    .to_string()
                    .as_bytes(),
            );
        }
    });
}
async fn run(app: tauri::AppHandle) -> anyhow::Result<()> {
    let root = output()?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| anyhow::anyhow!("No main window"))?;
    tokio::time::timeout(Duration::from_secs(30), async {
        while !window.is_visible().unwrap_or(false) {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await?;
    app.state::<AppState>().jobs_hub.wait_recovery().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    store::atomic_write(&root.join("jobs-smoke-started"), b"ready")?;
    let done = Arc::new(AtomicBool::new(false));
    let heartbeat_done = done.clone();
    let heartbeat_app = app.clone();
    let heartbeat = tauri::async_runtime::spawn(async move {
        let mut samples = vec![];
        while !heartbeat_done.load(Ordering::SeqCst) {
            let (send, receive) = tokio::sync::oneshot::channel();
            let start = Instant::now();
            if heartbeat_app
                .run_on_main_thread(move || {
                    let _ = send.send(start.elapsed().as_micros() as u64);
                })
                .is_err()
            {
                break;
            }
            if let Ok(Ok(delay)) = tokio::time::timeout(Duration::from_secs(2), receive).await {
                samples.push(delay);
            } else {
                samples.push(2_000_000);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        samples
    });
    let initial = jobs_list(app.state::<AppState>())
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(initial.jobs.len() == 10000, "Missing history");
    jobs_control_all(app.state::<AppState>(), "stop".into())
        .await
        .map_err(anyhow::Error::msg)?;
    let fixture: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?;
    let path = fixture["path"].as_str().unwrap().to_owned();
    let ids: Vec<String> = serde_json::from_value(fixture["ids"].clone())?;
    let engine = app.state::<AppState>().engine.clone();
    let submit = Instant::now();
    let added = tauri::async_runtime::spawn_blocking(move || {
        // Exercise runnable native jobs without inference packs or a service.
        // Missing synthetic sources stop workers before any rendering takes place.
        let mut settings = TranslationSettings::default();
        settings.cleanup.method = CleanupMethod::Solid;
        engine.enqueue_cleanup(&path, &ids, &settings)
    })
    .await??;
    let submit_ms = submit.elapsed().as_millis();
    anyhow::ensure!(
        added.len() == 1000 && added.iter().all(|j| j.status == "paused"),
        "Held submission failed"
    );
    let start = jobs_control_all(app.state::<AppState>(), "start".into())
        .await
        .map_err(anyhow::Error::msg)?;
    let stop = jobs_control_all(app.state::<AppState>(), "stop".into())
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        start.outcome.errors.is_empty() && stop.outcome.errors.is_empty(),
        "Bulk controls failed: start={:?}, stop={:?}",
        start.outcome.errors,
        stop.outcome.errors
    );
    tokio::time::sleep(Duration::from_millis(750)).await;
    let timer = Instant::now();
    let final_state = jobs_list(app.state::<AppState>())
        .await
        .map_err(anyhow::Error::msg)?;
    let list_ms = timer.elapsed().as_millis();
    anyhow::ensure!(
        final_state.held && final_state.jobs.len() == 11000,
        "Incorrect final jobs state"
    );
    anyhow::ensure!(
        final_state
            .jobs
            .iter()
            .filter(|j| j.job.status == "complete")
            .count()
            == 10000,
        "Completed history changed"
    );
    anyhow::ensure!(
        !final_state
            .jobs
            .iter()
            .any(|j| j.job.status == "running" || j.job.status == "queued"),
        "Jobs continued after Stop all"
    );
    done.store(true, Ordering::SeqCst);
    let samples = heartbeat.await?;
    let report = serde_json::json!({"passed":true,"history":10000,"submittedPages":1000,"submissionMs":submit_ms,"listMs":list_ms,"startChanged":start.outcome.changed,"stopChanged":stop.outcome.changed,"mainThreadHeartbeatSamples":samples.len(),"mainThreadMaxDelayUs":samples.iter().max(),"providerRequests":0,"note":"Synthetic missing sources fail before inference or service access. Exercises native handlers, scheduler, storage, events, and Jobs UI."});
    store::atomic_write(
        &root.join("jobs-smoke.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    app.emit("jobs-smoke-finished", ())?;
    Ok(())
}
#[tauri::command]
pub async fn jobs_smoke_ui_report(report: serde_json::Value) -> Api<()> {
    if !requested() {
        return Err("Jobs smoke is disabled".into());
    }
    let root = output().map_err(error)?;
    trace(&report.to_string());
    let filename = if report.get("frames").is_some() {
        "jobs-smoke-ui.json"
    } else {
        "startup-ui.json"
    };
    tauri::async_runtime::spawn_blocking(move || {
        store::atomic_write(&root.join(filename), &serde_json::to_vec_pretty(&report)?)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
