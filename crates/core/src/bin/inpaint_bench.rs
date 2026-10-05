//! Standalone offline benchmark. Never opens books, invokes providers or changes app settings.
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma, Rgb, RgbImage};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    time::Instant,
};
use umanga_core::{cleanup, inference, render::Renderer, store, types::Region};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn write_json(path: &Path, v: &impl serde::Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn freeze() -> Result<()> {
    let root = root();
    let dest = root.join("test-output/inpainting/dataset");
    fs::create_dir_all(&dest)?;
    let old: Vec<Value> =
        serde_json::from_slice(&fs::read(root.join("test-output/native/report.json"))?)?;
    let mut pages = Vec::new();
    for p in old {
        let name = p["page"].as_str().unwrap();
        let src = root.join(format!("test-output/native/{name}-original.png"));
        let im = image::open(&src)?.to_rgb8();
        let mut union = GrayImage::new(im.width(), im.height());
        let mut fallback = union.clone();
        let mut fill = im.clone();
        let regions: Vec<Region> = serde_json::from_value(p["regions"].clone())?;
        let bubbles: Vec<_> = regions.iter().filter_map(|r| r.bubble).collect();
        let mut details = Vec::new();
        for (i, r) in regions.iter().enumerate() {
            if r.target.trim().is_empty() {
                continue;
            }
            let c = cleanup::analyze_with_bubbles(&im, r, &bubbles);
            let file = format!("{name}-r{:02}-mask.png", i + 1);
            c.mask.save(dest.join(&file))?;
            for (x, y, m) in c.mask.enumerate_pixels() {
                if m[0] > 0 {
                    let (x, y) = (x + c.origin[0], y + c.origin[1]);
                    union.put_pixel(x, y, Luma([255]));
                    if c.interior.is_none() {
                        fallback.put_pixel(x, y, Luma([255]));
                    }
                    fill.put_pixel(x, y, Rgb(c.fill));
                }
            }
            details.push(
                json!({"index":i+1,"id":r.id,"bbox":r.bbox,"bubble":r.bubble,
                "origin":c.origin,"mask":file,"fill":c.fill,"enclosed":c.interior.is_some(),
                "mask_pixels":c.mask.as_raw().iter().filter(|&&v|v>0).count(),"region":r}),
            );
        }
        union.save(dest.join(format!("{name}-mask.png")))?;
        fallback.save(dest.join(format!("{name}-fallback.png")))?;
        fill.save(dest.join(format!("{name}-fill.png")))?;
        let original = root.join(format!("../sample_raw_manga/sample_page_styles/{name}.jpg"));
        pages.push(
            json!({"id":name,"image":src,"width":im.width(),"height":im.height(),
            "image_sha256":store::digest(&fs::read(&src)?),"original":original,
            "original_sha256":store::digest(&fs::read(original)?),"regions":details}),
        );
    }
    write_json(&dest.join("frozen.json"), &pages)?;
    println!(
        "{}",
        json!({"event":"frozen","pages":pages.len(),"regions":pages.iter().map(|p|p["regions"].as_array().unwrap().len()).sum::<usize>()})
    );
    Ok(())
}

#[derive(Clone, Deserialize)]
struct Case {
    id: String,
    kind: String,
    image: PathBuf,
    mask: PathBuf,
    fallback_mask: Option<PathBuf>,
    fill: PathBuf,
    tiles: Vec<[i32; 2]>,
    fallback_tiles: Option<Vec<[i32; 2]>>,
}
struct Loaded {
    case: Case,
    image: RgbImage,
    mask: GrayImage,
    fill: RgbImage,
    fallback: GrayImage,
}
#[derive(Clone, Deserialize)]
struct Config {
    dataset: PathBuf,
    model: Option<PathBuf>,
    adapter: String,
    provider: String,
    threads: usize,
    output: PathBuf,
    repeats: usize,
    warmups: usize,
    #[serde(default)]
    profile: bool,
    #[serde(default)]
    hybrid: bool,
    #[serde(default)]
    fixed_context: bool,
    #[serde(default)]
    limit: usize,
    #[serde(default = "one")]
    workers: usize,
}
fn one() -> usize {
    1
}
#[cfg(test)]
use umanga_core::inpainting::{reflect, tile_touches};

struct Runner {
    native: umanga_core::inpainting::ModelSession,
    adapter: String,
}
impl Runner {
    fn new(c: &Config, worker: usize) -> Result<Self> {
        let profile = c.output.join(format!("profile-{worker}"));
        Ok(Self {
            native: umanga_core::inpainting::ModelSession::new(
                &c.adapter,
                c.model.as_deref().unwrap_or(Path::new("")),
                &c.provider,
                c.threads,
                c.profile.then_some(profile.as_path()),
            )?,
            adapter: c.adapter.clone(),
        })
    }
    fn process(&mut self, p: &Loaded, hybrid: bool, fixed: bool) -> Result<(RgbImage, Value)> {
        let start = Instant::now();
        let output_mask = if hybrid { &p.fallback } else { &p.mask };
        let input_mask = if hybrid && !fixed {
            &p.fallback
        } else {
            &p.mask
        };
        let tiles = if hybrid && !fixed {
            p.case.fallback_tiles.as_ref().unwrap_or(&p.case.tiles)
        } else {
            &p.case.tiles
        };
        let base = if hybrid {
            p.fill.clone()
        } else {
            p.image.clone()
        };
        let (image, timing) = if self.adapter == "solid" {
            (p.fill.clone(), vec![])
        } else {
            umanga_core::inpainting::compose(
                &p.image,
                input_mask,
                output_mask,
                tiles,
                base,
                &mut self.native,
                &tokio_util::sync::CancellationToken::new(),
                &|_, _| {},
            )?
        };
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let outside = p
            .image
            .enumerate_pixels()
            .filter(|(x, y, v)| p.mask.get_pixel(*x, *y)[0] == 0 && image.get_pixel(*x, *y) != *v)
            .count();
        ensure!(outside == 0, "Outside-mask pixels changed");
        Ok((
            image,
            json!({"id":p.case.id,"kind":p.case.kind,"elapsed_ms":elapsed,"tiles":timing,"outside_changed":outside}),
        ))
    }
}

fn work(
    c: Config,
    cases: Vec<Case>,
    worker: usize,
    barrier: Option<Arc<Barrier>>,
) -> Result<Value> {
    let start = Instant::now();
    let mut runner = Runner::new(&c, worker)?;
    let load = start.elapsed().as_secs_f64() * 1000.;
    println!(
        "{}",
        json!({"event":"model_loaded","worker":worker,"load_ms":load})
    );
    std::io::stdout().flush()?;
    let mut loaded = Vec::new();
    for case in cases {
        let image = image::open(&case.image)?.to_rgb8();
        let mask = image::open(&case.mask)?.to_luma8();
        let fallback = case
            .fallback_mask
            .as_ref()
            .map(|f| image::open(f).map(|x| x.to_luma8()))
            .transpose()?
            .unwrap_or_else(|| mask.clone());
        let fill = image::open(&case.fill)?.to_rgb8();
        ensure!(
            image.dimensions() == mask.dimensions() && image.dimensions() == fill.dimensions(),
            "Image/mask size mismatch"
        );
        loaded.push(Loaded {
            case,
            image,
            mask,
            fill,
            fallback,
        });
    }
    ensure!(!loaded.is_empty(), "Empty dataset");
    let (_, first) = runner.process(&loaded[0], c.hybrid, c.fixed_context)?;
    println!(
        "{}",
        json!({"event":"first_result","worker":worker,"result":first})
    );
    // A plain first page can contain no hybrid inference tiles. Warm a page
    // that actually calls the model, without changing first-result semantics.
    let warm_index = if c.hybrid {
        loaded
            .iter()
            .position(|p| {
                !p.case
                    .fallback_tiles
                    .as_ref()
                    .unwrap_or(&p.case.tiles)
                    .is_empty()
            })
            .unwrap_or(0)
    } else {
        0
    };
    for _ in 0..c.warmups {
        runner.process(&loaded[warm_index], c.hybrid, c.fixed_context)?;
    }
    let mut rows = Vec::new();
    let mut image_io_ms = 0.;
    let mut pass_wall_ms = Vec::new();
    for repeat in 0..c.repeats {
        if let Some(b) = &barrier {
            b.wait();
        }
        let pass_start = Instant::now();
        for p in &loaded {
            if repeat > 0 && p.case.kind != "production" {
                continue;
            }
            println!(
                "{}",
                json!({"event":"case_start","worker":worker,"id":p.case.id,"repeat":repeat})
            );
            std::io::stdout().flush()?;
            let (im, mut row) = runner.process(p, c.hybrid, c.fixed_context)?;
            row["repeat"] = json!(repeat);
            row["worker"] = json!(worker);
            // Check concurrent output without saving PNGs in throughput trials.
            // Hashing is outside the measured cleanup interval.
            row["rgb_sha256"] = json!(format!("{:x}", Sha256::digest(im.as_raw())));
            println!(
                "{}",
                json!({"event":"case","worker":worker,"id":p.case.id,"repeat":repeat,"elapsed_ms":row["elapsed_ms"]})
            );
            std::io::stdout().flush()?;
            // Concurrent runs measure throughput only. Quality images come from the
            // equivalent one-session run; PNG saves must not serialize workers.
            if repeat == 0 && c.workers == 1 {
                let t = Instant::now();
                im.save(c.output.join(format!("{}.png", p.case.id)))?;
                image_io_ms += t.elapsed().as_secs_f64() * 1000.;
            }
            rows.push(row);
            write_json(
                &c.output.join(format!("worker{worker}-partial.json")),
                &rows,
            )?;
        }
        pass_wall_ms.push(pass_start.elapsed().as_secs_f64() * 1000.);
        if let Some(b) = &barrier {
            b.wait();
        }
    }
    let profile = if c.profile {
        runner
            .native
            .session
            .as_mut()
            .map(|s| s.end_profiling())
            .transpose()?
    } else {
        None
    };
    Ok(
        json!({"load_ms":load,"first_result":first,"rows":rows,"image_io_ms":image_io_ms,"profile":profile,"pass_wall_ms":pass_wall_ms}),
    )
}

fn run(path: &Path) -> Result<()> {
    let c: Config = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        c.workers > 0 && c.threads > 0,
        "Threads and workers must be positive"
    );
    fs::create_dir_all(&c.output)?;
    let init = Instant::now();
    inference::init(&root().join("assets/runtime"))?;
    let init_ms = init.elapsed().as_secs_f64() * 1000.;
    let mut cases: Vec<Case> = serde_json::from_slice(&fs::read(&c.dataset)?)?;
    if c.limit > 0 {
        cases.truncate(c.limit)
    }
    let start = Instant::now();
    let mut groups = vec![Vec::new(); c.workers];
    for (i, p) in cases.into_iter().enumerate() {
        groups[i % c.workers].push(p)
    }
    ensure!(
        groups.iter().all(|g| !g.is_empty()),
        "More workers than cases"
    );
    let barrier = if c.workers > 1 {
        Some(Arc::new(Barrier::new(c.workers)))
    } else {
        None
    };
    let mut handles = Vec::new();
    for (i, g) in groups.into_iter().enumerate() {
        let c = c.clone();
        let b = barrier.clone();
        handles.push(std::thread::spawn(move || {
            // A failed participant cannot leave the other session waiting forever at
            // the barrier. This is an isolated benchmark process, not the app.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(c, g, i, b))) {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) => {
                    eprintln!("worker {i}: {e:#}");
                    std::process::exit(2)
                }
                Err(_) => {
                    eprintln!("worker {i} panicked");
                    std::process::exit(2)
                }
            }
        }));
    }
    let results: Vec<Value> = handles
        .into_iter()
        .map(|h| h.join().map_err(|_| anyhow::anyhow!("Worker panicked"))?)
        .collect::<Result<_>>()?;
    write_json(
        &c.output.join("result.json"),
        &json!({"provider":c.provider,"adapter":c.adapter,"threads":c.threads,"workers":c.workers,"hybrid":c.hybrid,"fixed_context":c.fixed_context,"runtime_init_ms":init_ms,"wall_ms":start.elapsed().as_secs_f64()*1000.,"results":results}),
    )?;
    Ok(())
}

fn lettering(dir: &Path) -> Result<()> {
    let root = root();
    let saved: Vec<Value> =
        serde_json::from_slice(&fs::read(root.join("test-output/native/report.json"))?)?;
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut report = Vec::new();
    for p in saved {
        let name = p["page"].as_str().unwrap();
        let clean = dir.join(format!("page-{name}.png"));
        if !clean.exists() {
            continue;
        }
        let orig = image::open(root.join(format!("test-output/native/{name}-original.png")))?;
        let rgb = orig.to_rgb8();
        let cleaned = image::open(clean)?;
        let mut regions: Vec<Region> = serde_json::from_value(p["regions"].clone())?;
        let bubbles: Vec<_> = regions.iter().filter_map(|r| r.bubble).collect();
        // Freeze the production renderer's automatic contrast decision across all methods.
        for r in &mut regions {
            r.review = None;
            let c = cleanup::analyze_with_bubbles(&rgb, r, &bubbles);
            let fg = umanga_core::render::color(&r.style.color);
            if !r.allow_fill && (0..3).all(|i| fg[i].abs_diff(c.fill[i]) < 55) {
                let l = c.fill[0] as u32 * 299 + c.fill[1] as u32 * 587 + c.fill[2] as u32 * 114;
                r.style.color = if l < 128000 { "#ffffff" } else { "#000000" }.into();
            }
        }
        let t = Instant::now();
        let out = renderer.render_benchmark(&orig, &mut regions, "zh-Hans", Some(&cleaned))?;
        let ms = t.elapsed().as_secs_f64() * 1000.;
        out.save(dir.join(format!("page-{name}-lettered.png")))?;
        report.push(json!({"page":name,"lettering_ms":ms,"reviews":regions.iter().filter_map(|r|r.review.as_ref()).collect::<Vec<_>>()}));
    }
    write_json(&dir.join("lettering.json"), &report)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("freeze") => freeze(),
        Some("run") => run(Path::new(args.get(2).context("Config path")?)),
        Some("letter") => lettering(Path::new(args.get(2).context("Result directory")?)),
        _ => anyhow::bail!("Usage: inpaint_bench freeze | run CONFIG.json | letter RESULT_DIR"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflection_handles_small_pages() {
        assert_eq!(reflect(-1, 10), 1);
        assert_eq!(reflect(10, 10), 8);
        assert_eq!(reflect(-513, 1), 0);
    }
    #[test]
    fn tile_selection_covers_edges_without_reflected_only_tiles() {
        let mut mask = GrayImage::new(1025, 700);
        mask.put_pixel(0, 0, Luma([255]));
        mask.put_pixel(1024, 699, Luma([255]));
        assert!(tile_touches(&mask, -192, -192));
        assert!(tile_touches(&mask, 1024, 699));
        assert!(!tile_touches(&mask, 1, 1));
        assert!(!tile_touches(&mask, -512, 0));
        assert!(!tile_touches(&mask, 1025, 699));
    }
}
