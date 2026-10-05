//! Isolated OCR benchmark. Reads images/models; never opens books or calls a service.
#[path = "benchmark_ocr_support/mod.rs"]
mod support;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};
use support::Runner;

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    pub corpus: PathBuf,
    pub model: String,
    pub model_root: PathBuf,
    #[serde(default = "region")]
    pub pipeline: String,
    #[serde(default = "cpu")]
    pub device: String,
    #[serde(default = "four")]
    pub threads: usize,
    #[serde(default = "eight")]
    pub batch: usize,
    #[serde(default)]
    pub warmups: usize,
    #[serde(default = "one")]
    pub passes: usize,
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub save_crops: Option<PathBuf>,
    #[serde(default)]
    pub page_ids: Vec<String>,
    #[serde(default)]
    pub region_ids: Vec<String>,
    #[serde(default = "tokens")]
    pub max_tokens: usize,
    #[serde(default = "three")]
    pub no_repeat_ngram: usize,
    #[serde(default)]
    pub detector_root: Option<PathBuf>,
    #[serde(default)]
    pub baseline_root: Option<PathBuf>,
    #[serde(default)]
    pub runtime: Option<PathBuf>,
    #[serde(default)]
    pub dynamic_width: bool,
    #[serde(default)]
    pub crop_repair: bool,
    #[serde(default)]
    pub layout_cleanup: bool,
    #[serde(default)]
    pub geometry_only: bool,
}
fn region() -> String {
    "region".into()
}
fn cpu() -> String {
    "cpu".into()
}
fn four() -> usize {
    4
}
fn eight() -> usize {
    8
}
fn one() -> usize {
    1
}
fn three() -> usize {
    3
}
fn tokens() -> usize {
    160
}

#[derive(Clone, Deserialize)]
pub struct Corpus {
    pub version: u32,
    pub pages: Vec<Page>,
}
#[derive(Clone, Deserialize)]
pub struct Page {
    pub id: String,
    pub image: PathBuf,
    pub regions: Vec<Region>,
    // Keep detector ownership stable when only a diagnostic subset is requested.
    #[serde(skip)]
    pub association_regions: Vec<Region>,
}
#[derive(Clone, Deserialize)]
pub struct Region {
    pub id: String,
    pub bbox: [f32; 4],
    #[serde(default = "auto")]
    pub orientation: String,
    #[serde(default)]
    pub lines: Vec<Line>,
}
#[derive(Clone, Deserialize)]
pub struct Line {
    pub id: String,
    #[serde(default)]
    pub bbox: Option<[f32; 4]>,
    #[serde(default)]
    pub quad: Option<[[f32; 2]; 4]>,
    #[serde(default = "auto")]
    pub orientation: String,
    #[serde(default)]
    pub order: usize,
}
fn auto() -> String {
    "auto".into()
}

pub struct Events {
    file: fs::File,
    started: Instant,
}
impl Events {
    fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Refuse accidental replacement of measurements from a previous run.
        Ok(Self {
            file: fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?,
            started: Instant::now(),
        })
    }
    pub fn emit(&mut self, mut value: Value) -> Result<()> {
        value["elapsed_ms"] = json!(self.started.elapsed().as_secs_f64() * 1000.);
        let line = serde_json::to_string(&value)?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        println!("{line}");
        Ok(())
    }
}
pub fn app_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
pub fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}

fn run(config_path: &Path, output: &Path) -> Result<()> {
    let mut config: Config = serde_json::from_slice(&fs::read(config_path)?)?;
    ensure!(
        config.threads > 0 && config.batch > 0 && config.batch <= 64,
        "Invalid threads/batch size"
    );
    ensure!(
        (1..=512).contains(&config.max_tokens),
        "max_tokens must be 1..512"
    );
    ensure!(
        ["cpu", "directml"].contains(&config.device.as_str()),
        "Only CPU and DirectML are native benchmark targets"
    );
    ensure!(
        [
            "current",
            "region",
            "region_lines",
            "page_lines",
            "checked_lines",
            "hybrid"
        ]
        .contains(&config.pipeline.as_str()),
        "Unknown pipeline"
    );
    ensure!(
        !config.layout_cleanup
            || ["page_lines", "region_lines", "hybrid"].contains(&config.pipeline.as_str()),
        "Line grouping requires a line-based or hybrid pipeline"
    );
    ensure!(
        !config.crop_repair || config.pipeline != "checked_lines",
        "Checked reference geometry cannot be silently repaired"
    );
    ensure!(
        config.pipeline != "hybrid" || config.model == "manga",
        "Hybrid routing uses the existing Manga OCR and PPv5 Mobile recognizers"
    );
    let mut corpus: Corpus = serde_json::from_slice(&fs::read(&config.corpus)?)?;
    ensure!(corpus.version == 1, "Unsupported corpus version");
    if !config.page_ids.is_empty() {
        corpus.pages.retain(|p| config.page_ids.contains(&p.id));
    }
    for page in &mut corpus.pages {
        page.association_regions = page.regions.clone();
        if !config.region_ids.is_empty() {
            page.regions.retain(|r| {
                config.region_ids.contains(&r.id)
                    || config.region_ids.contains(&format!("{}:{}", page.id, r.id))
            });
        }
    }
    corpus.pages.retain(|p| !p.regions.is_empty());
    ensure!(!corpus.pages.is_empty(), "Empty selected corpus");
    let mut events = Events::new(output)?;
    events.emit(json!({"event":"start", "config":config, "pages":corpus.pages.len(), "regions":corpus.pages.iter().map(|p| p.regions.len()).sum::<usize>()}))?;
    let init = Instant::now();
    let runtime = config
        .runtime
        .clone()
        .unwrap_or_else(|| app_root().join("assets/runtime"));
    umanga_core::inference::init(&runtime)?;
    // Diagnostic crops are emitted once, never in repeated timed passes.
    let crop_path = config.save_crops.take();
    let profile_root = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ort-profile");
    let runner = Runner::new(&config, &profile_root);
    let mut runner = match runner {
        Ok(r) => r,
        Err(e) => {
            events.emit(json!({"event":"error", "stage":"init", "status":"failed", "error":format!("{e:#}")}))?;
            return Err(e);
        }
    };
    events.emit(json!({"event":"init", "milliseconds":ms(init), "model":config.model, "requested_device":config.device, "actual_device":runner.actual_device(), "baseline_fixed_settings":config.pipeline=="current"}))?;
    let first = Instant::now();
    let first_result = runner.page(
        &corpus.pages[0],
        "first",
        0,
        crop_path.as_deref(),
        &mut events,
    );
    if let Err(error) = first_result {
        events.emit(json!({"event":"error","phase":"first","stage":"first_result","page_id":corpus.pages[0].id,"status":"failed","error":format!("{error:#}")}))?;
        return Err(error);
    }
    events.emit(json!({"event":"first_result", "milliseconds":ms(first)}))?;
    for repeat in 0..config.warmups {
        runner.page(&corpus.pages[0], "warmup", repeat, None, &mut events)?;
    }
    let mut completed = 0;
    let mut failed = 0;
    let mut totals = Vec::new();
    for pass in 0..config.passes {
        let pass_start = Instant::now();
        for (page_index, page) in corpus.pages.iter().enumerate() {
            let save = if pass == 0 && page_index != 0 {
                crop_path.as_deref()
            } else {
                None
            };
            match runner.page(page, "timed", pass, save, &mut events) {
                Ok(outcome) => {
                    completed += outcome.completed;
                    failed += outcome.failed;
                }
                Err(e) => {
                    failed += page.regions.len();
                    for region in &page.regions {
                        events.emit(json!({"event":"region","page_id":page.id,"region_id":region.id,"phase":"timed","pass":pass,"status":"failed","error":format!("{e:#}"),"text":"","cap":false}))?;
                    }
                    events.emit(json!({"event":"page", "page_id":page.id,"phase":"timed","pass":pass,"status":"failed","error":format!("{e:#}"),"completed":0,"expected":page.regions.len()}))?;
                }
            }
        }
        totals.push(ms(pass_start));
        events.emit(json!({"event":"pass", "pass":pass, "milliseconds":totals.last()}))?;
    }
    let profiles = runner.finish_profiles()?;
    events.emit(json!({"event":"summary", "status":if failed==0 {"complete"} else {"partial"}, "completed":completed,"failed":failed,
        "expected":corpus.pages.iter().map(|p|p.regions.len()).sum::<usize>()*config.passes,"pass_wall_ms":totals,"actual_device":runner.actual_device(),"profiles":profiles}))?;
    Ok(())
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(i) = args.iter().position(|a| a == "--inspect-model") {
        umanga_core::inference::init(&app_root().join("assets/runtime"))?;
        return support::inspect(Path::new(args.get(i + 1).context("Missing model path")?));
    }
    let arg = |name: &str| -> Result<&str> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .with_context(|| format!("Missing {name}"))
    };
    run(Path::new(arg("--config")?), Path::new(arg("--output")?))
}
