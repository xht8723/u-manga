//! Synthetic storage benchmark; creates only a unique fixture under test-output.
use anyhow::Result;
use std::{path::PathBuf, time::Instant};
use umanga_core::{documents, library, store, types::*};
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-output/correctness")
        .join(format!("storage-{}", uid()));
    std::fs::create_dir_all(&root)?;
    let source = root.join("original.png");
    image::RgbImage::new(64, 64).save(&source)?;
    let sample = documents::import(&[source.to_string_lossy().into()])?.remove(0);
    let pages: Vec<_> = (0..1000)
        .map(|number| {
            let mut p = sample.clone();
            p.id = uid();
            p.number = number;
            p
        })
        .collect();
    let ids: Vec<_> = pages.iter().map(|p| p.id.clone()).collect();
    let start = Instant::now();
    let book = library::create(
        &root.join("library"),
        library::Metadata {
            title: "1,000 pages".into(),
            ..Default::default()
        },
        library::ImportPreview {
            omitted_page_ids: vec![],
            chapters: vec![library::Chapter {
                id: uid(),
                title: "Chapter".into(),
                page_ids: ids.clone(),
                read: false,
            }],
            pages,
            warnings: vec![],
        },
        None,
    )?;
    let import_ms = start.elapsed().as_secs_f64() * 1000.;
    let path = std::path::Path::new(&book.path);
    let mut results = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        assert_eq!(
            library::chapter_pages(path, &book.chapters[0].id)?.len(),
            1000
        );
        let chapter_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        assert_eq!(library::source_identities(path)?.len(), 1000);
        let identities_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        for id in &ids {
            store::page(path, id)?;
        }
        results.push(serde_json::json!({"chapterMs":chapter_ms,"sourceIdentityMs":identities_ms,"legacyOneConnectionPerPageMs":start.elapsed().as_secs_f64()*1000.}));
    }
    let report =
        serde_json::json!({"pages":1000,"importMs":import_ms,"passes":results,"fixture":root});
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-output/correctness/storage.json");
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
