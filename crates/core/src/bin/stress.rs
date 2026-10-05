use anyhow::Result;
use std::{path::PathBuf, time::Instant};
use umanga_core::{documents, store, types::*};
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    documents::init_pdfium(&root.join("assets/runtime"))?;
    let data = root.join("test-output/native");
    let latest = std::fs::read_dir(&data)?
        .filter_map(|p| p.ok())
        .map(|p| p.path())
        .filter(|p| p.extension().is_some_and(|s| s == "umanga"))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .unwrap();
    let original = store::open(&latest)?;
    let mut pages = vec![];
    for _ in 0..10 {
        for p in &original.pages {
            let mut p = p.clone();
            p.id = uid();
            p.number = pages.len();
            pages.push(p);
        }
    }
    let destination = root.join("test-output/stress");
    std::fs::create_dir_all(&destination)?;
    let path = destination.join(format!("long-book-{}.umanga", uid()));
    let project = store::create(&path, "190-page memory validation", &pages)?;
    let start = Instant::now();
    documents::export(&project, &destination.join("long-book.pdf"), "pdf")?;
    let report = serde_json::json!({"pages":pages.len(),"pdfExportMs":start.elapsed().as_millis(),"pdfBytes":std::fs::metadata(destination.join("long-book.pdf"))?.len()});
    store::atomic_write(
        &destination.join("report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
