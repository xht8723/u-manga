//! Reproducible submission/recovery/IPC measurements; synthetic book, no inference or services.
use anyhow::Result;
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use umanga_core::{documents, pipeline::Engine, store, types::*};
struct Count(u64);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn measurement<T: serde::Serialize>(make: impl Fn() -> T) -> Result<serde_json::Value> {
    let mut passes = vec![];
    let mut bytes = 0;
    for pass in 0..7 {
        let start = Instant::now();
        let value = make();
        let snapshot_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let mut count = Count(0);
        serde_json::to_writer(&mut count, &value)?;
        if pass >= 2 {
            passes.push(serde_json::json!({"snapshotMs":snapshot_ms,"serializeMs":start.elapsed().as_secs_f64()*1000.}));
        }
        bytes = count.0;
    }
    Ok(serde_json::json!({"bytes":bytes,"passes":passes}))
}
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join("test-output/audit-2026-09-30");
    fs::create_dir_all(&output)?;
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1116".into())
        .parse()?;
    anyhow::ensure!(count <= 1116, "Fixture glossary limit is 1116");
    let profile_name = if cfg!(debug_assertions) {
        "development-debug-1"
    } else {
        "release-thin-lto"
    };
    let folder = output.join(format!("metrics-{}", uid()));
    fs::create_dir_all(&folder)?;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _owned = Fixture(folder.clone());
    let image = folder.join("source.png");
    image::RgbImage::new(16, 16).save(&image)?;
    let base = documents::import(&[image.to_string_lossy().into()])?.remove(0);
    let pages: Vec<_> = (0..1000)
        .map(|n| {
            let mut p = base.clone();
            p.id = uid();
            p.number = n;
            p
        })
        .collect();
    let project = store::create(&folder.join("book.umanga"), "Synthetic 1000 pages", &pages)?;
    let settings = TranslationSettings {
        glossary: (0..count)
            .map(|n| GlossaryEntry {
                source: format!("固有名詞の測定用テキスト{n}"),
                target: format!("术语测量固定翻译名称{n}"),
            })
            .collect(),
        ..Default::default()
    };
    let profile = ProviderProfile::default();
    let ids: Vec<_> = pages.iter().map(|p| p.id.clone()).collect();
    let mut submission_passes = vec![];
    let mut submitted = vec![];
    for pass in 0..7 {
        store::connection(&PathBuf::from(&project.path))?.execute("DELETE FROM jobs", [])?;
        let engine = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| anyhow::bail!("Benchmark cannot call a provider")),
        )?;
        engine.request_control("stop")?;
        let start = Instant::now();
        let current = engine.enqueue_configured(&project.path, &ids, &profile, &settings)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if pass >= 2 {
            submission_passes.push(elapsed);
        }
        println!("Submission pass {pass}: 1000 held pages in {elapsed:.1} ms");
        submitted = current;
    }
    let mut history = Vec::with_capacity(10000);
    for n in 0..10000 {
        let mut job = submitted[n % submitted.len()].clone();
        job.id = uid();
        job.status = "complete".into();
        job.stage = "done".into();
        history.push(job);
    }
    store::connection(&PathBuf::from(&project.path))?.execute("DELETE FROM jobs", [])?;
    let start = Instant::now();
    store::save_jobs(&PathBuf::from(&project.path), &history)?;
    let storage_ms = start.elapsed().as_secs_f64() * 1000.;
    drop(history);
    drop(submitted);
    println!("Saved 10000 history rows in {storage_ms:.1} ms");
    let mut recovery_passes = vec![];
    let mut recovered = None;
    for pass in 0..7 {
        // Do not retain the previous history while measuring another activation.
        drop(recovered.take());
        let engine = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| anyhow::bail!("Benchmark cannot call a provider")),
        )?;
        let start = Instant::now();
        engine.recover(&project.path)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if pass >= 2 {
            recovery_passes.push(elapsed);
        }
        println!("Recovery pass {pass}: 10000 rows in {elapsed:.1} ms");
        recovered = Some(engine);
    }
    let engine = recovered.unwrap();
    let legacy = measurement(|| engine.snapshot(None))?;
    let compact = measurement(|| engine.snapshot_views(None))?;
    let report = serde_json::json!({"fixture":{"pages":1000,"jobs":10000,"glossaryEntriesPerJob":count,"synthetic":true},"submissionPassesMs":submission_passes,"write10000Ms":storage_ms,"writePasses":1,"recoveryPassesMs":recovery_passes,"legacySnapshot":legacy,"compactSnapshot":compact,"build":profile_name,"warmups":2,"services":0,"inference":0});
    store::atomic_write(
        &output.join(format!("native-metrics-{profile_name}-{count}.json")),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
