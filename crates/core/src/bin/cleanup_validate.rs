//! Production renderer validation. Never creates OCR, detection, or service requests.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use umanga_core::{documents, inference, pipeline::Engine, store, types::*};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 4,
        "cleanup_validate MODEL automatic|all_regions cpu|directml [PAGE]"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    inference::init(&root.join("assets/runtime"))?;
    let settings = TranslationSettings {
        cleanup: CleanupSettings {
            method: serde_json::from_value(json!(args[1]))?,
            strategy: serde_json::from_value(json!(args[2]))?,
            device: serde_json::from_value(json!(args[3]))?,
        },
        ..Default::default()
    };
    let name = format!("{}-{}-{}", args[1], args[2], args[3]);
    let out = std::env::var_os("U_MANGA_VALIDATION_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("test-output/inpainting-integration"))
        .join(&name);
    fs::create_dir_all(&out)?;
    let models = std::env::var_os("U_MANGA_VALIDATION_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("test-output/inpainting-integration/models"));
    let engine = Engine::new(
        models.clone(),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("Validation cannot call a provider")),
    )?;
    let dataset_path = std::env::var_os("U_MANGA_VALIDATION_DATASET")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("test-output/inpainting/dataset/frozen.json"));
    let dataset: Vec<Value> = serde_json::from_slice(&fs::read(dataset_path)?)?;
    let mut rows = vec![];
    for item in dataset {
        let id = item["id"].as_str().unwrap();
        if args.get(4).is_some_and(|s| s != id) {
            continue;
        }
        let source = item["image"].as_str().unwrap();
        let before = store::digest(&fs::read(source)?);
        let original = image::open(source)?;
        let mut page = documents::import(&[source.into()])?.remove(0);
        page.regions = item["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                // Historical benchmark input predates manual preparation. This adapter
                // never reads or converts a user's persisted book.
                let mut region = r["region"].clone();
                region
                    .as_object_mut()
                    .unwrap()
                    .entry("prepared")
                    .or_insert(json!(false));
                serde_json::from_value(region)
            })
            .collect::<std::result::Result<_, _>>()?;
        let book = out.join(format!("{id}.umanga"));
        // Each run has independent data; never reopen or migrate a baseline book.
        if !book.exists() {
            store::create(&book, id, &[page.clone()])?;
        }
        let start = Instant::now();
        let events = Arc::new(Mutex::new(vec![]));
        let messages = events.clone();
        let rendered=engine.render_page(&original,&mut page,&settings,&book,&models,true,&CancellationToken::new(),Arc::new(move|device,n,total|{
            messages.lock().unwrap().push(json!({"device":device,"tile":n,"total":total,"ms":start.elapsed().as_secs_f64()*1000.}));
        }))?;
        let render_ms = start.elapsed().as_secs_f64() * 1000.;
        let cleanup = page.cleanup.as_ref().unwrap().clone();
        let reviews: std::collections::BTreeMap<_, _> = page
            .regions
            .iter()
            .filter_map(|r| {
                r.review
                    .as_ref()
                    .map(|reason| (r.id.clone(), reason.clone()))
            })
            .collect();
        let background = image::open(&cleanup.path)?.to_rgb8();
        let rgb = original.to_rgb8();
        let mut full = image::GrayImage::new(rgb.width(), rgb.height());
        let bubbles: Vec<_> = page.regions.iter().filter_map(|r| r.bubble).collect();
        for region in &page.regions {
            let c = umanga_core::cleanup::analyze_with_bubbles(&rgb, region, &bubbles);
            for (x, y, m) in c.mask.enumerate_pixels() {
                if m[0] > 0 {
                    full.put_pixel(x + c.origin[0], y + c.origin[1], image::Luma([255]));
                }
            }
        }
        let outside = rgb
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                full.get_pixel(*x, *y)[0] == 0 && background.get_pixel(*x, *y) != *p
            })
            .count();
        ensure!(outside == 0, "Outside-mask pixels changed on {id}");
        ensure!(
            before == store::digest(&fs::read(source)?),
            "Original modified"
        );
        let save = Instant::now();
        rendered.save(out.join(format!("{id}-lettered.png")))?;
        background.save(out.join(format!("{id}-clean.png")))?;
        full.save(out.join(format!("{id}-mask.png")))?;
        let save_ms = save.elapsed().as_secs_f64() * 1000.;
        let first_path = cleanup.path.clone();
        let first_key = cleanup.key.clone();
        let mut changed = settings.clone();
        changed.cleanup.method = CleanupMethod::Solid;
        for r in &mut page.regions {
            r.style.color = "#303030".into();
        }
        let hit = Instant::now();
        engine.render_page(
            &original,
            &mut page,
            &changed,
            &book,
            &models,
            false,
            &CancellationToken::new(),
            Arc::new(|_, _, _| {}),
        )?;
        ensure!(
            page.cleanup.as_ref().unwrap().key == first_key
                && page.cleanup.as_ref().unwrap().path == first_path,
            "Typography edit missed cleanup cache on {id}"
        );
        ensure!(
            page.cleanup.as_ref().unwrap().settings.method == settings.cleanup.method,
            "Settings change replaced saved recipe"
        );
        rows.push(json!({"page":id,"renderMs":render_ms,"saveMs":save_ms,"cachedRenderMs":hit.elapsed().as_secs_f64()*1000.,"outsideMaskChanged":outside,"device":cleanup.device,"reviews":reviews,"events":*events.lock().unwrap()}));
        fs::write(out.join("results.json"), serde_json::to_vec_pretty(&rows)?)?;
        println!(
            "{name} {id}: {render_ms:.0} ms; {} reviews; outside mask unchanged",
            reviews.len()
        );
    }
    Ok(())
}
