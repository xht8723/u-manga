//! Create an isolated Windows UI fixture from saved translations, without provider calls.
use anyhow::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use umanga_core::{documents, library, store, types::*};

fn main() -> Result<()> {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = app
        .join("test-output/reader-cleanup")
        .join(format!("windows-{}", uid()));
    let library = root.join("library");
    let baseline = app.join("test-output/native");
    let saved: Vec<Value> = serde_json::from_slice(&std::fs::read(baseline.join("report.json"))?)?;
    let names = [
        "0007", "0005", "0006", "001", "0013", "0018", "0029", "0046", "0047", "0048", "0059",
        "0060", "011", "015", "0166", "0173", "018", "0182", "026",
    ];
    let paths: Vec<String> = names
        .iter()
        .map(|n| {
            baseline
                .join(format!("{n}-original.png"))
                .to_string_lossy()
                .into()
        })
        .collect();
    let mut pages = documents::import(&paths)?;
    // Import naturally sorts; set the focused regression case first deliberately.
    pages.sort_by_key(|p| {
        names
            .iter()
            .position(|n| p.name == format!("{n}-original.png"))
            .unwrap()
    });
    for (i, p) in pages.iter_mut().enumerate() {
        p.number = i;
        p.regions = serde_json::from_value(
            saved.iter().find(|s| s["page"] == names[i]).unwrap()["regions"].clone(),
        )?;
        p.status = "review".into();
    }
    let chapters = vec![
        library::Chapter {
            id: uid(),
            title: "Dialogue regression".into(),
            page_ids: pages[..3].iter().map(|p| p.id.clone()).collect(),
            read: false,
        },
        library::Chapter {
            id: uid(),
            title: "Other styles".into(),
            page_ids: pages[3..].iter().map(|p| p.id.clone()).collect(),
            read: false,
        },
    ];
    let book = library::create(
        &library,
        library::Metadata {
            title: "Reader cleanup validation".into(),
            language: "ja".into(),
            ..Default::default()
        },
        library::ImportPreview {
            omitted_page_ids: vec![],
            chapters,
            pages,
            warnings: vec![],
        },
        None,
    )?;
    let path = Path::new(&book.path);
    for (i, mut page) in store::open(path)?.pages.into_iter().enumerate() {
        let dest = store::assets(path).join(format!("baseline-{}.png", page.id));
        std::fs::copy(baseline.join(format!("{}.png", names[i])), &dest)?;
        page.rendered = Some(dest.to_string_lossy().into());
        let revision = page.revision;
        anyhow::ensure!(
            store::save_page(path, &mut page, revision)?,
            "Fixture save conflict"
        );
    }
    library::refresh(&library, path)?;
    let mut settings = AppSettings {
        library_directory: library.to_string_lossy().into(),
        ..Default::default()
    };
    settings.setup.completed = true; // Isolated rendering/navigation fixture, not setup validation.
    settings.setup.step = "review".into();
    store::atomic_write(
        &root.join("preferences.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )?;
    let info = json!({"root":root,"book":book.path,"sourceRoot":baseline});
    store::atomic_write(
        &app.join("test-output/reader-cleanup/windows-session.json"),
        &serde_json::to_vec_pretty(&info)?,
    )?;
    println!("{info}");
    Ok(())
}
