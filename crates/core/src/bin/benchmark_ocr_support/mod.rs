mod layout;
mod segmentation;
use crate::{Config, Events, Page, app_root, ms};
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, imageops::FilterType};
use ort::{
    session::Session,
    value::{DynValue, Tensor},
};
use segmentation::{Segment, bbox_quad, quad_bbox};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use umanga_core::{inference::Inference, types::TranslationSettings};

#[derive(Default)]
pub struct Outcome {
    pub completed: usize,
    pub failed: usize,
}
#[derive(Default, Clone)]
struct Reading {
    text: String,
    cap: bool,
    eos: bool,
    repeated: bool,
    tokens: usize,
    width_clamped: bool,
}
#[derive(Default, Clone, serde::Serialize)]
struct Timings {
    preprocess: f64,
    inference: f64,
    decoding: f64,
}
impl Timings {
    fn add(&mut self, other: &Self) {
        self.preprocess += other.preprocess;
        self.inference += other.inference;
        self.decoding += other.decoding;
    }
}
#[derive(Clone)]
struct Sample {
    region: usize,
    line: String,
    image: DynamicImage,
    quad: [[f32; 2]; 4],
    vertical: bool,
    score: Option<f32>,
    source_clip: Option<[u32; 4]>,
    use_pp: bool,
}
pub struct Runner {
    c: Config,
    sessions: HashMap<String, Session>,
    current: Option<Inference>,
    secondary: Option<Box<Runner>>,
    dictionary: Vec<String>,
    profile_root: PathBuf,
    actual_device: String,
    batch_timings: Timings,
    page_timings: Timings,
}
pub fn inspect(path: &Path) -> Result<()> {
    let s = Session::builder()?
        .with_intra_threads(4)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .commit_from_file(path)?;
    println!(
        "{}",
        json!({"path":path,"inputs":format!("{:?}",s.inputs()),"outputs":format!("{:?}",s.outputs())})
    );
    Ok(())
}
impl Runner {
    pub fn new(c: &Config, profile_root: &Path) -> Result<Self> {
        if c.pipeline == "current" {
            ensure!(
                !c.crop_repair && !c.layout_cleanup && !c.geometry_only,
                "Current application baseline does not enable layout experiments"
            );
            let batch = if c.model.starts_with("manga") && c.device == "cpu" {
                1
            } else {
                8
            };
            ensure!(
                c.threads == 4 && c.batch == batch,
                "Current app uses four threads and batch {batch} for this model/device"
            );
            ensure!(
                c.max_tokens == 160 && c.no_repeat_ngram == 3 && !c.dynamic_width,
                "Current baseline preserves its fixed decoder and width settings; use an experimental pipeline for ablations"
            );
            ensure!(
                ["manga", "manga_ocr", "ppv5_mobile", "pp"].contains(&c.model.as_str()),
                "Current app supports only Manga OCR or PPv5 mobile"
            );
        }
        let current = if c.pipeline == "current" {
            Some(Inference::new(
                c.baseline_root
                    .clone()
                    .unwrap_or_else(|| app_root().join("assets/models")),
            ))
        } else {
            None
        };
        let dictionary = if current.is_some() {
            Vec::new()
        } else if c.model.starts_with("pp") {
            let value: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(c.model_root.join("inference.yml"))?)?;
            let mut chars = vec![String::new()];
            chars.extend(
                value["PostProcess"]["character_dict"]
                    .as_sequence()
                    .context("Missing PP dictionary")?
                    .iter()
                    .map(|x| x.as_str().unwrap_or("").to_string()),
            );
            chars.push(" ".into());
            chars
        } else if c.model == "ndl_parseq" {
            let value: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(c.model_root.join("NDLmoji.yaml"))?)?;
            let chars = value["model"]["charset_train"]
                .as_str()
                .context("Missing NDL charset_train")?;
            std::iter::once(String::new())
                .chain(chars.chars().map(|c| c.to_string()))
                .collect()
        } else if c.model == "baberu" {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(c.model_root.join("vocab.json"))?)?;
            let chars = value
                .as_array()
                .map(|v| {
                    v.iter()
                        .map(|x| x.as_str().unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                })
                .or_else(|| {
                    value
                        .as_str()
                        .map(|s| s.chars().map(|c| c.to_string()).collect())
                })
                .context("Unexpected Baberu vocab (expected char array/string)")?;
            let mut out = vec![String::new(); 4];
            out.extend(chars);
            out
        } else {
            fs::read_to_string(c.model_root.join("vocab.txt"))?
                .lines()
                .map(str::to_string)
                .collect()
        };
        if c.profile {
            fs::create_dir_all(profile_root)?;
        }
        let mut out = Self {
            c: c.clone(),
            sessions: HashMap::new(),
            current,
            secondary: None,
            dictionary,
            profile_root: profile_root.to_owned(),
            actual_device: if c.device == "cpu" {
                "CPU".into()
            } else {
                "DirectML requested; operator placement recorded in profile run".into()
            },
            batch_timings: Timings::default(),
            page_timings: Timings::default(),
        };
        // Initialization includes graph compilation for experimental adapters.
        if out.current.is_none() {
            if c.geometry_only {
                // Export geometry without loading or invoking a recognizer.
            } else if c.model == "manga" || c.model == "manga_ocr" {
                out.load("encoder", "encoder_model.onnx")?;
                out.load("decoder", "decoder_model.onnx")?;
            } else if c.model == "baberu" {
                out.load("vision", "vision_fp16.onnx")?;
                out.load("prefill", "decoder_prefill_int8.onnx")?;
                out.load("step", "decoder_step_int8.onnx")?;
            } else {
                out.load("recognizer", "inference.onnx")?;
            }
            if ["region_lines", "page_lines", "hybrid"].contains(&c.pipeline.as_str()) {
                out.detector()?;
            }
        }
        if c.pipeline == "hybrid" && !c.geometry_only {
            ensure!(c.model == "manga", "Hybrid routing starts from Manga OCR");
            let mut secondary = c.clone();
            secondary.model = "ppv5_mobile".into();
            secondary.model_root = c
                .baseline_root
                .clone()
                .unwrap_or_else(|| app_root().join("assets/models"))
                .join("pp_cjk");
            secondary.pipeline = "region".into();
            secondary.batch = 8;
            secondary.crop_repair = false;
            secondary.layout_cleanup = false;
            out.secondary = Some(Box::new(Self::new(&secondary, &profile_root.join("ppv5"))?));
        }
        Ok(out)
    }
    fn make_session(&self, path: &Path, name: &str) -> Result<Session> {
        let mut builder = Session::builder()?
            .with_intra_threads(self.c.threads)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if self.c.device == "directml" {
            builder = builder
                .with_parallel_execution(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_memory_pattern(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        if self.c.profile {
            builder = builder
                .with_profiling(self.profile_root.join(name))
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        builder
            .commit_from_file(path)
            .with_context(|| format!("Cannot load {} for {}", path.display(), self.c.device))
    }
    fn load(&mut self, name: &str, file: &str) -> Result<&mut Session> {
        if !self.sessions.contains_key(name) {
            let s = self.make_session(&self.c.model_root.join(file), name)?;
            self.sessions.insert(name.into(), s);
        }
        Ok(self.sessions.get_mut(name).unwrap())
    }
    fn detector(&mut self) -> Result<&mut Session> {
        if !self.sessions.contains_key("detector") {
            let root = self
                .c
                .detector_root
                .clone()
                .unwrap_or_else(|| app_root().join("assets/models/pp_det"));
            let s = self.make_session(&root.join("inference.onnx"), "detector")?;
            self.sessions.insert("detector".into(), s);
        }
        Ok(self.sessions.get_mut("detector").unwrap())
    }
    pub fn actual_device(&self) -> &str {
        &self.actual_device
    }
    pub fn finish_profiles(&mut self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        if self.c.profile {
            for s in self.sessions.values_mut() {
                out.push(PathBuf::from(s.end_profiling()?));
            }
        }
        if let Some(secondary) = &mut self.secondary {
            out.extend(secondary.finish_profiles()?);
        }
        Ok(out)
    }
    fn detect_lines(&mut self, im: &DynamicImage) -> Result<Vec<Segment>> {
        let ratio = 960. / im.width().max(im.height()) as f32;
        let w = ((im.width() as f32 * ratio / 32.).round().max(1.) as u32) * 32;
        let h = ((im.height() as f32 * ratio / 32.).round().max(1.) as u32) * 32;
        let data = pixels(im, w, h, true, [0.485, 0.456, 0.406], [0.229, 0.224, 0.225]);
        let out = self.detector()?.run(ort::inputs![Tensor::from_array((
            [1, 3, h as usize, w as usize],
            data
        ))?])?;
        let (shape, prob) = out[0].try_extract_tensor::<f32>()?;
        Ok(segmentation::postprocess(
            prob,
            shape[shape.len() - 1] as u32,
            shape[shape.len() - 2] as u32,
            im.width(),
            im.height(),
        ))
    }
    pub fn page(
        &mut self,
        page: &Page,
        phase: &str,
        pass: usize,
        save: Option<&Path>,
        events: &mut Events,
    ) -> Result<Outcome> {
        events.emit(json!({"event":"page_start","page_id":page.id,"phase":phase,"pass":pass,"expected":page.regions.len()}))?;
        let total = Instant::now();
        let decode = Instant::now();
        let im = image::open(&page.image)?;
        let decode_ms = ms(decode);
        let segment = Instant::now();
        let mut samples = Vec::new();
        let mut association_page = page.clone();
        let parents = if page.association_regions.is_empty() {
            &page.regions
        } else {
            &page.association_regions
        };
        let mut crops = HashMap::new();
        for parent in parents {
            let original = expanded_bbox(&im, parent.bbox);
            let decision = if self.c.crop_repair {
                let neighbors = parents
                    .iter()
                    .filter(|r| r.id != parent.id)
                    .map(|r| r.bbox)
                    .collect::<Vec<_>>();
                layout::repair_crop(&im, original, &neighbors)
            } else {
                layout::CropDecision {
                    bounds: original,
                    reasons: Vec::new(),
                }
            };
            crops.insert(parent.id.clone(), (original, decision));
        }
        if self.c.crop_repair {
            association_page.association_regions = parents
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    let (original, decision) = &crops[&r.id];
                    if original != &decision.bounds {
                        r.bbox = decision.bounds.map(|v| v as f32);
                    }
                    r
                })
                .collect();
        }
        let mut layouts = page.regions.iter().map(|r| {
            let (original, decision) = &crops[&r.id];
            json!({"rules_version":layout::RULES_VERSION,"original_crop":original,"ocr_crop":decision.bounds,"crop_repair_reasons":decision.reasons,
                "layout_reasons":[],"suppressed":[],"raw_lines":0,"route":self.c.model})
        }).collect::<Vec<_>>();
        for r in &page.regions {
            events.emit(json!({"event":"region_start","page_id":page.id,"region_id":r.id,"phase":phase,"pass":pass,"shared":true}))?;
        }
        if self.current.is_some() || self.c.pipeline == "region" {
            for (i, r) in page.regions.iter().enumerate() {
                let b = crops[&r.id].1.bounds;
                let crop = im.crop_imm(b[0], b[1], b[2] - b[0], b[3] - b[1]);
                samples.push(Sample {
                    region: i,
                    line: "block".into(),
                    image: crop,
                    quad: bbox_quad(b.map(|v| v as f32)),
                    vertical: r.orientation == "vertical",
                    score: None,
                    source_clip: None,
                    use_pp: false,
                });
            }
        } else if self.c.pipeline == "checked_lines" {
            for (i, r) in page.regions.iter().enumerate() {
                let mut lines = r.lines.clone();
                lines.sort_by_key(|l| l.order);
                for l in lines {
                    let q = l
                        .quad
                        .or_else(|| l.bbox.map(bbox_quad))
                        .context("Checked line needs bbox or quad")?;
                    let vertical = l.orientation == "vertical"
                        || (l.orientation == "auto"
                            && segmentation::distance(q[0], q[3])
                                > segmentation::distance(q[0], q[1]) * 1.5);
                    let crop =
                        segmentation::rectify(&im.to_rgb8(), q, self.rotate_lines() && vertical)?;
                    samples.push(Sample {
                        region: i,
                        line: l.id,
                        image: crop,
                        quad: q,
                        vertical,
                        score: None,
                        source_clip: None,
                        use_pp: false,
                    });
                }
            }
        } else {
            let page_lines = if self.c.pipeline == "page_lines" || self.c.pipeline == "hybrid" {
                Some(self.detect_lines(&im)?)
            } else {
                None
            };
            for (i, r) in page.regions.iter().enumerate() {
                let mut lines = if let Some(lines) = &page_lines {
                    lines
                        .iter()
                        .filter(|l| associated_with(l, &association_page, &r.id))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    let b = crops[&r.id].1.bounds;
                    let crop = im.crop_imm(b[0], b[1], b[2] - b[0], b[3] - b[1]);
                    let mut lines = self.detect_lines(&crop)?;
                    for l in &mut lines {
                        for p in &mut l.quad {
                            p[0] += b[0] as f32;
                            p[1] += b[1] as f32;
                        }
                    }
                    lines
                };
                layouts[i]["raw_lines"] = json!(lines.len());
                if self.c.layout_cleanup {
                    let decision =
                        layout::group_lines(lines, crops[&r.id].1.bounds, &r.orientation);
                    layouts[i]["layout_reasons"] = json!(decision.reasons);
                    layouts[i]["suppressed"] = json!(
                        decision
                            .suppressed
                            .iter()
                            .map(|s| json!({"quad":s.quad,"vertical":s.vertical,"score":s.score}))
                            .collect::<Vec<_>>()
                    );
                    lines = decision.lines;
                } else {
                    segmentation::sort(&mut lines, &r.orientation);
                }
                if self.c.pipeline == "hybrid" {
                    let dense = layout::dense_route(&lines, crops[&r.id].1.bounds);
                    layouts[i]["dense"] = json!({"use_lines":dense.use_lines,"estimated_characters":dense.estimated_characters,"body_lines":dense.body_lines,"reason":dense.reason});
                    layouts[i]["route"] = json!(if dense.use_lines {
                        "ppv5_mobile"
                    } else {
                        "manga"
                    });
                    if !dense.use_lines {
                        let b = crops[&r.id].1.bounds;
                        samples.push(Sample {
                            region: i,
                            line: "block".into(),
                            image: im.crop_imm(b[0], b[1], b[2] - b[0], b[3] - b[1]),
                            quad: bbox_quad(b.map(|v| v as f32)),
                            vertical: r.orientation == "vertical",
                            score: None,
                            source_clip: Some(b),
                            use_pp: false,
                        });
                        continue;
                    }
                }
                for (n, line) in lines.into_iter().enumerate() {
                    let source_clip = page_lines.as_ref().map(|_| crops[&r.id].1.bounds);
                    let rotate =
                        (self.rotate_lines() || self.c.pipeline == "hybrid") && line.vertical;
                    let crop = if let Some(bounds) = source_clip {
                        segmentation::rectify_clipped(&im.to_rgb8(), line.quad, rotate, bounds)?
                    } else {
                        segmentation::rectify(&im.to_rgb8(), line.quad, rotate)?
                    };
                    samples.push(Sample {
                        region: i,
                        line: n.to_string(),
                        image: crop,
                        quad: line.quad,
                        vertical: line.vertical,
                        score: Some(line.score),
                        source_clip,
                        use_pp: self.c.pipeline == "hybrid",
                    });
                }
            }
        }
        let segment_ms = ms(segment);
        let recognition = Instant::now();
        self.page_timings = Timings::default();
        if self.current.is_some() {
            events.emit(json!({"event":"baseline_page_work","page_id":page.id,"phase":phase,"pass":pass,"per_region_deadline_available":false}))?;
        }
        let readings = if self.c.geometry_only {
            vec![Reading::default(); samples.len()]
        } else if self.c.pipeline == "hybrid" {
            let mut readings = vec![Reading::default(); samples.len()];
            let manga = samples
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.use_pp)
                .map(|(i, s)| (i, s.clone()))
                .collect::<Vec<_>>();
            let pp = samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.use_pp)
                .map(|(i, s)| (i, s.clone()))
                .collect::<Vec<_>>();
            if !manga.is_empty() {
                let input = manga.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>();
                let values = self.recognize(&input, phase, pass, &page.id, events)?;
                for ((i, _), value) in manga.into_iter().zip(values) {
                    readings[i] = value;
                }
            }
            if !pp.is_empty() {
                let input = pp.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>();
                let secondary = self
                    .secondary
                    .as_mut()
                    .context("Missing PPv5 routing recognizer")?;
                secondary.page_timings = Timings::default();
                let values = secondary.recognize(&input, phase, pass, &page.id, events)?;
                self.page_timings.add(&secondary.page_timings);
                for ((i, _), value) in pp.into_iter().zip(values) {
                    readings[i] = value;
                }
            }
            readings
        } else if let Some(current) = &mut self.current {
            let crops = samples.iter().map(|s| s.image.clone()).collect::<Vec<_>>();
            let settings = TranslationSettings {
                ocr: if self.c.model.starts_with("manga") {
                    "manga"
                } else {
                    "pp"
                }
                .into(),
                source_language: "ja".into(),
                device: self.c.device.clone(),
                ..Default::default()
            };
            let texts = current.recognize(&crops, &settings)?;
            self.actual_device = current.device_message.clone();
            texts
                .into_iter()
                .map(|text| Reading {
                    tokens: text.chars().count(),
                    cap: self.c.model.starts_with("manga") && text.chars().count() >= 159,
                    text,
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        } else {
            self.recognize(&samples, phase, pass, &page.id, events)?
        };
        let recognition_ms = ms(recognition);
        ensure!(
            readings.len() == samples.len(),
            "Recognizer result count mismatch"
        );
        let reconstruct = Instant::now();
        let mut outcome = Outcome::default();
        let mut region_events = Vec::new();
        for (i, r) in page.regions.iter().enumerate() {
            let pairs = samples
                .iter()
                .zip(&readings)
                .filter(|(s, _)| s.region == i)
                .collect::<Vec<_>>();
            let vertical = r.orientation == "vertical"
                || (r.orientation == "auto"
                    && pairs.iter().filter(|(s, _)| s.vertical).count() * 2 > pairs.len());
            let text = pairs
                .iter()
                .map(|(_, v)| v.text.clone())
                .collect::<Vec<_>>()
                .join(if vertical { "" } else { "\n" });
            let status = if self.c.geometry_only {
                "geometry_only"
            } else if pairs.is_empty() {
                "no_lines"
            } else if text.is_empty() {
                "empty"
            } else {
                "ok"
            };
            outcome.completed += 1;
            region_events.push(json!({"event":"region","page_id":page.id,"region_id":r.id,"phase":phase,"pass":pass,"text":text,"status":status,"cap":pairs.iter().any(|(_,r)|r.cap),"repeated":pairs.iter().any(|(_,r)|r.repeated),"layout":layouts[i],"geometry_only":self.c.geometry_only,"tokens":pairs.iter().map(|(_,r)|r.tokens).sum::<usize>(),"width_clamped":pairs.iter().any(|(_,r)|r.width_clamped),"eos":pairs.iter().all(|(_,r)|r.eos),"timings_ms":{"shared_page_recognition":recognition_ms},"lines":pairs.iter().map(|(s,v)|json!({"id":s.line,"quad":s.quad,"vertical":s.vertical,"score":s.score,"width":s.image.width(),"height":s.image.height(),"text":v.text,"cap":v.cap,"source_clip":s.source_clip,"recognizer":if s.use_pp {"ppv5_mobile"} else {self.c.model.as_str()}})).collect::<Vec<_>>()}));
        }
        let reconstruct_ms = ms(reconstruct);
        let total_ms = ms(total);
        for e in region_events {
            events.emit(e)?;
        }
        let saving = Instant::now();
        if let Some(root) = save {
            let root = root.join(&page.id);
            fs::create_dir_all(&root)?;
            for s in &samples {
                let filename = format!("r{}-{}.png", page.regions[s.region].id, s.line);
                s.image.save(root.join(filename))?;
            }
        }
        let save_ms = if save.is_some() { ms(saving) } else { 0. };
        events.emit(json!({"event":"page","page_id":page.id,"phase":phase,"pass":pass,"status":"complete","completed":outcome.completed,"expected":page.regions.len(),"samples":samples.len(),"actual_device":self.actual_device,"cap_observability":if self.current.is_some(){"baseline length heuristic; EOS not exposed"}else{"decoder observed"},"recognizer_timing_available":self.current.is_none(),"timings_ms":{"total":total_ms,"image_decode":decode_ms,"segmentation":segment_ms,"recognition":recognition_ms,"preprocess":self.page_timings.preprocess,"inference":self.page_timings.inference,"decoding":self.page_timings.decoding,"reconstruct":reconstruct_ms,"diagnostic_save":save_ms}}))?;
        Ok(outcome)
    }
    fn rotate_lines(&self) -> bool {
        self.c.model.starts_with("pp") || self.c.model == "ndl_parseq"
    }
    fn recognize(
        &mut self,
        samples: &[Sample],
        phase: &str,
        pass: usize,
        page: &str,
        events: &mut Events,
    ) -> Result<Vec<Reading>> {
        let batch = if ["baberu", "ndl_parseq"].contains(&self.c.model.as_str()) {
            1
        } else {
            self.c.batch
        };
        let mut result = Vec::new();
        for (index, group) in samples.chunks(batch).enumerate() {
            events.emit(json!({"event":"batch_start","page_id":page,"phase":phase,"pass":pass,"batch":index,"samples":group.len(),"effective_batch":batch}))?;
            let started = Instant::now();
            self.batch_timings = Timings::default();
            let readings = if self.c.model.starts_with("pp") {
                self.pp(group)?
            } else if self.c.model == "ndl_parseq" {
                vec![self.ndl(&group[0].image)?]
            } else if self.c.model == "baberu" {
                vec![self.baberu(&group[0].image, events, phase, pass, page)?]
            } else {
                self.manga(group, events, phase, pass, page)?
            };
            self.page_timings.add(&self.batch_timings);
            events.emit(json!({"event":"batch","page_id":page,"phase":phase,"pass":pass,"batch":index,"samples":group.len(),"effective_batch":batch,"milliseconds":ms(started),"timings_ms":self.batch_timings}))?;
            result.extend(readings);
        }
        Ok(result)
    }
    fn pp(&mut self, group: &[Sample]) -> Result<Vec<Reading>> {
        let pre = Instant::now();
        let natural = group
            .iter()
            .map(|s| (s.image.width() as f32 / s.image.height() as f32 * 48.).ceil() as usize)
            .collect::<Vec<_>>();
        let width = group
            .iter()
            .map(|s| (s.image.width() as f32 / s.image.height() as f32 * 48.).floor() as usize)
            .max()
            .unwrap()
            .max(if self.c.dynamic_width { 16 } else { 320 })
            .min(3200);
        let widths = natural
            .iter()
            .map(|w| (*w).clamp(1, width))
            .collect::<Vec<_>>();
        let mut data = vec![0.; group.len() * 3 * 48 * width];
        for (b, s) in group.iter().enumerate() {
            let v = pixels(&s.image, widths[b] as u32, 48, true, [0.5; 3], [0.5; 3]);
            for c in 0..3 {
                for y in 0..48 {
                    let src = (c * 48 + y) * widths[b];
                    let dst = ((b * 3 + c) * 48 + y) * width;
                    data[dst..dst + widths[b]].copy_from_slice(&v[src..src + widths[b]]);
                }
            }
        }
        self.batch_timings.preprocess = ms(pre);
        let inference = Instant::now();
        let out = self
            .load("recognizer", "inference.onnx")?
            .run(ort::inputs![Tensor::from_array((
                [group.len(), 3, 48, width],
                data
            ))?])?;
        let (shape, prob) = out[0].try_extract_tensor::<f32>()?;
        let steps = shape[1] as usize;
        let classes = shape[2] as usize;
        let raw = prob.to_vec();
        drop(out);
        self.batch_timings.inference = ms(inference);
        let decoding = Instant::now();
        ensure!(
            raw.iter().all(|v| v.is_finite()),
            "Non-finite PP model output"
        );
        ensure!(
            classes == self.dictionary.len(),
            "PP dictionary/model class count differs: {classes} vs {}",
            self.dictionary.len()
        );
        let mut result = Vec::new();
        for b in 0..group.len() {
            let ids = (0..steps)
                .map(|t| argmax(&raw[(b * steps + t) * classes..(b * steps + t + 1) * classes]))
                .collect::<Vec<_>>();
            let text = ctc_decode(&ids, &self.dictionary);
            result.push(Reading {
                tokens: text.chars().count(),
                text,
                eos: true,
                width_clamped: natural[b] > 3200,
                ..Default::default()
            });
        }
        self.batch_timings.decoding = ms(decoding);
        Ok(result)
    }
    fn ndl(&mut self, im: &DynamicImage) -> Result<Reading> {
        let pre = Instant::now();
        let rotated = if im.height() as f32 > im.width() as f32 * 0.8 {
            im.rotate270()
        } else {
            im.clone()
        };
        let data = pixels(&rotated, 768, 24, true, [0.5; 3], [0.5; 3]);
        self.batch_timings.preprocess = ms(pre);
        let inference = Instant::now();
        let out = self
            .load("recognizer", "inference.onnx")?
            .run(ort::inputs![Tensor::from_array(([1, 3, 24, 768], data))?])?;
        let (shape, prob) = out[0].try_extract_tensor::<f32>()?;
        let classes = *shape.last().unwrap() as usize;
        let steps = prob.len() / classes;
        let raw = prob.to_vec();
        ensure!(
            raw.iter().all(|v| v.is_finite()),
            "Non-finite NDL model output"
        );
        drop(out);
        self.batch_timings.inference = ms(inference);
        let decoding = Instant::now();
        let ids = (0..steps)
            .map(|t| argmax(&raw[t * classes..(t + 1) * classes]))
            .collect::<Vec<_>>();
        let eos = ids.contains(&0);
        let text = ids
            .iter()
            .take_while(|id| **id != 0)
            .filter_map(|id| self.dictionary.get(*id))
            .cloned()
            .collect::<String>();
        self.batch_timings.decoding = ms(decoding);
        Ok(Reading {
            tokens: text.chars().count(),
            text,
            cap: !eos,
            eos,
            ..Default::default()
        })
    }
    fn manga(
        &mut self,
        group: &[Sample],
        events: &mut Events,
        phase: &str,
        pass: usize,
        page: &str,
    ) -> Result<Vec<Reading>> {
        let pre = Instant::now();
        let n = group.len();
        let data = group
            .iter()
            .flat_map(|s| {
                let rgb = s.image.to_rgb8();
                let gray = image::GrayImage::from_fn(rgb.width(), rgb.height(), |x, y| {
                    let p = rgb.get_pixel(x, y);
                    image::Luma([((p[0] as u32 * 19595
                        + p[1] as u32 * 38470
                        + p[2] as u32 * 7471
                        + 32768)
                        >> 16) as u8])
                });
                pixels(
                    &DynamicImage::ImageLuma8(gray),
                    224,
                    224,
                    false,
                    [0.5; 3],
                    [0.5; 3],
                )
            })
            .collect::<Vec<_>>();
        self.batch_timings.preprocess = ms(pre);
        let inference = Instant::now();
        let out = self
            .load("encoder", "encoder_model.onnx")?
            .run(ort::inputs![Tensor::from_array(([n, 3, 224, 224], data))?])?;
        let (shape, hidden) = out[0].try_extract_tensor::<f32>()?;
        let shape = shape.to_vec();
        let hidden = hidden.to_vec();
        drop(out);
        self.batch_timings.inference = ms(inference);
        let mut tokens = vec![vec![2i64]; n];
        let mut done = vec![false; n];
        let mut last = Instant::now();
        for step in 0..self.c.max_tokens {
            let len = tokens[0].len();
            let ids = tokens.iter().flatten().copied().collect::<Vec<_>>();
            let inference = Instant::now();
            let out=self.load("decoder","decoder_model.onnx")?.run(ort::inputs!["input_ids"=>Tensor::from_array(([n,len],ids))?,"encoder_hidden_states"=>Tensor::from_array((shape.clone(),hidden.clone()))?])?;
            let (shape, logits) = out[0].try_extract_tensor::<f32>()?;
            let classes = *shape.last().unwrap() as usize;
            let seq = shape[1] as usize;
            let logits = logits.to_vec();
            drop(out);
            self.batch_timings.inference += ms(inference);
            let decoding = Instant::now();
            ensure!(
                logits.iter().all(|v| v.is_finite()),
                "Non-finite Manga model output"
            );
            for b in 0..n {
                if done[b] {
                    tokens[b].push(0);
                    continue;
                }
                let banned = banned_tokens(&tokens[b], self.c.no_repeat_ngram);
                let offset = (b * seq + seq - 1) * classes;
                let id = logits[offset..offset + classes]
                    .iter()
                    .enumerate()
                    .filter(|(id, _)| !banned.contains(&(*id as i64)))
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .context("No decoder token")?
                    .0 as i64;
                tokens[b].push(id);
                done[b] = id == 3;
            }
            self.batch_timings.decoding += ms(decoding);
            if last.elapsed().as_secs() >= 2 {
                events.emit(json!({"event":"decode_progress","page_id":page,"phase":phase,"pass":pass,"step":step,"samples":n}))?;
                last = Instant::now();
            }
            if done.iter().all(|x| *x) {
                break;
            }
        }
        let decoding = Instant::now();
        let result = tokens
            .iter()
            .enumerate()
            .map(|(b, row)| {
                let ids = row
                    .iter()
                    .skip(1)
                    .take_while(|id| **id != 3)
                    .copied()
                    .collect::<Vec<_>>();
                let text = ids
                    .iter()
                    .filter_map(|id| self.dictionary.get(*id as usize))
                    .filter(|v| !v.starts_with('['))
                    .map(|v| v.trim_start_matches("##"))
                    .collect::<String>();
                Reading {
                    text,
                    cap: !done[b],
                    eos: done[b],
                    tokens: ids.len(),
                    ..Default::default()
                }
            })
            .collect();
        self.batch_timings.decoding += ms(decoding);
        Ok(result)
    }
    fn baberu(
        &mut self,
        im: &DynamicImage,
        events: &mut Events,
        phase: &str,
        pass: usize,
        page: &str,
    ) -> Result<Reading> {
        let pre = Instant::now();
        let resized =
            DynamicImage::ImageRgb8(im.resize_exact(224, 224, FilterType::CatmullRom).to_rgb8());
        let data = pixels(
            &resized,
            224,
            224,
            false,
            [0.485, 0.456, 0.406],
            [0.229, 0.224, 0.225],
        );
        self.batch_timings.preprocess = ms(pre);
        let inference = Instant::now();
        let mut out = self
            .load("vision", "vision_fp16.onnx")?
            .run(ort::inputs!["pixel_values"=>Tensor::from_array(([1,3,224,224],data))?])?;
        let embeds = out
            .remove("vision_embeds")
            .context("Missing vision_embeds")?;
        drop(out);
        let mut out=self.load("prefill","decoder_prefill_int8.onnx")?.run(ort::inputs!["vision_embeds"=>embeds,"input_ids"=>Tensor::from_array(([1,1],vec![1i64]))?])?;
        let mut logits = last_logits(&out["logits"])?;
        let mut cache = take_cache(&mut out)?;
        drop(out);
        self.batch_timings.inference = ms(inference);
        let mut ids = Vec::new();
        let mut eos = false;
        let mut repeated = false;
        let mut last = Instant::now();
        let limit = self.c.max_tokens.min(128);
        for step in 0..limit {
            let decoding = Instant::now();
            for id in std::iter::once(1)
                .chain(ids.iter().copied())
                .collect::<HashSet<usize>>()
            {
                logits[id] = if logits[id] > 0. {
                    logits[id] / 1.2
                } else {
                    logits[id] * 1.2
                };
            }
            if let Some(&id) = ids.last() {
                let content = self
                    .dictionary
                    .get(id)
                    .map(|s| {
                        s.chars().count() == 1
                            && s.chars().next().unwrap().is_alphanumeric()
                            && !["ー", "ｰ", "〜", "~"].contains(&s.as_str())
                    })
                    .unwrap_or(false);
                if content && ids.iter().rev().take_while(|i| **i == id).count() >= 12 {
                    logits[id] = f32::NEG_INFINITY;
                    repeated = true;
                }
            }
            let id = argmax(&logits);
            self.batch_timings.decoding += ms(decoding);
            if id == 2 {
                eos = true;
                break;
            }
            ids.push(id);
            if step + 1 == limit {
                break;
            }
            let mut inputs = ort::inputs!["input_ids"=>Tensor::from_array(([1,1],vec![id as i64]))?,"position_ids"=>Tensor::from_array(([1,1],vec![257+step as i64]))?];
            for (name, value) in cache {
                inputs.push((name.into(), value.into()));
            }
            let inference = Instant::now();
            let mut out = self.load("step", "decoder_step_int8.onnx")?.run(inputs)?;
            logits = last_logits(&out["logits"])?;
            cache = take_cache(&mut out)?;
            drop(out);
            self.batch_timings.inference += ms(inference);
            if last.elapsed().as_secs() >= 2 {
                events.emit(json!({"event":"decode_progress","page_id":page,"phase":phase,"pass":pass,"step":step,"samples":1}))?;
                last = Instant::now();
            }
        }
        let decoding = Instant::now();
        let text = ids
            .iter()
            .filter_map(|id| self.dictionary.get(*id))
            .cloned()
            .collect::<String>();
        self.batch_timings.decoding += ms(decoding);
        Ok(Reading {
            tokens: ids.len(),
            text,
            cap: !eos,
            eos,
            repeated,
            ..Default::default()
        })
    }
}
fn last_logits(value: &DynValue) -> Result<Vec<f32>> {
    let (shape, values) = value.try_extract_tensor::<f32>()?;
    let n = *shape.last().unwrap() as usize;
    ensure!(
        values[values.len() - n..].iter().all(|v| v.is_finite()),
        "Non-finite decoder output"
    );
    Ok(values[values.len() - n..].to_vec())
}
fn take_cache(out: &mut ort::session::SessionOutputs<'_>) -> Result<Vec<(String, DynValue)>> {
    let mut result = Vec::new();
    for kind in ["k", "v"] {
        for i in 0..6 {
            let name = format!("present_{kind}{i}");
            let value = out
                .remove(&name)
                .with_context(|| format!("Missing {name}"))?;
            result.push((format!("past_{kind}{i}"), value));
        }
    }
    Ok(result)
}
fn expanded_bbox(im: &DynamicImage, b: [f32; 4]) -> [u32; 4] {
    let b = umanga_core::inference::clamp(
        [
            b[0].floor() - 2.,
            b[1].floor() - 2.,
            b[2].floor() + 3.,
            b[3].floor() + 3.,
        ],
        im,
    );
    [b[0] as u32, b[1] as u32, b[2] as u32, b[3] as u32]
}
fn associated_with(line: &Segment, page: &Page, selected_id: &str) -> bool {
    let b = quad_bbox(&line.quad);
    let center = [(b[0] + b[2]) / 2., (b[1] + b[3]) / 2.];
    let parents = if page.association_regions.is_empty() {
        &page.regions
    } else {
        &page.association_regions
    };
    let chosen = parents
        .iter()
        .filter(|r| {
            center[0] >= r.bbox[0]
                && center[0] <= r.bbox[2]
                && center[1] >= r.bbox[1]
                && center[1] <= r.bbox[3]
        })
        .min_by(|a, b| {
            ((a.bbox[2] - a.bbox[0]) * (a.bbox[3] - a.bbox[1]))
                .total_cmp(&((b.bbox[2] - b.bbox[0]) * (b.bbox[3] - b.bbox[1])))
        });
    chosen.is_some_and(|r| r.id == selected_id)
}
fn pixels(im: &DynamicImage, w: u32, h: u32, bgr: bool, mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
    // Paddle/NDL use OpenCV INTER_LINEAR without downsampling antialiasing.
    // Manga OCR uses Pillow's antialiased bilinear image resize.
    let im = if bgr {
        resize_linear(&im.to_rgb8(), w, h)
    } else {
        im.resize_exact(w, h, FilterType::Triangle).to_rgb8()
    };
    let n = (w * h) as usize;
    let mut data = vec![0.; n * 3];
    for (i, p) in im.pixels().enumerate() {
        for c in 0..3 {
            data[c * n + i] = (p[if bgr { 2 - c } else { c }] as f32 / 255. - mean[c]) / std[c];
        }
    }
    data
}
fn resize_linear(im: &image::RgbImage, w: u32, h: u32) -> image::RgbImage {
    if im.width() == w && im.height() == h {
        return im.clone();
    }
    image::RgbImage::from_fn(w, h, |x, y| {
        let sx = ((x as f64 + 0.5) * im.width() as f64 / w as f64 - 0.5)
            .clamp(0., im.width() as f64 - 1.);
        let sy = ((y as f64 + 0.5) * im.height() as f64 / h as f64 - 0.5)
            .clamp(0., im.height() as f64 - 1.);
        let x0 = sx.floor() as u32;
        let y0 = sy.floor() as u32;
        let x1 = (x0 + 1).min(im.width() - 1);
        let y1 = (y0 + 1).min(im.height() - 1);
        let dx = sx - x0 as f64;
        let dy = sy - y0 as f64;
        image::Rgb(std::array::from_fn(|c| {
            let a =
                im.get_pixel(x0, y0)[c] as f64 * (1. - dx) + im.get_pixel(x1, y0)[c] as f64 * dx;
            let b =
                im.get_pixel(x0, y1)[c] as f64 * (1. - dx) + im.get_pixel(x1, y1)[c] as f64 * dx;
            (a * (1. - dy) + b * dy).round().clamp(0., 255.) as u8
        }))
    })
}
fn argmax(row: &[f32]) -> usize {
    row.iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0)
}
fn ctc_decode(ids: &[usize], dictionary: &[String]) -> String {
    let mut last = 0;
    let mut out = String::new();
    for id in ids {
        if *id != 0
            && *id != last
            && let Some(ch) = dictionary.get(*id)
        {
            out.push_str(ch);
        }
        last = *id;
    }
    out
}
fn banned_tokens(tokens: &[i64], size: usize) -> HashSet<i64> {
    if size == 0 || tokens.len() + 1 < size {
        return HashSet::new();
    }
    let prefix = &tokens[tokens.len() - (size - 1)..];
    tokens
        .windows(size)
        .filter(|w| &w[..size - 1] == prefix)
        .map(|w| w[size - 1])
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctc_blank_keeps_repeated_characters() {
        let d = vec!["".into(), "あ".into(), "。".into()];
        assert_eq!(ctc_decode(&[1, 1, 0, 1, 2, 2], &d), "ああ。");
    }
    #[test]
    fn no_repeat_ngram_is_explicit() {
        assert!(banned_tokens(&[2, 4, 5, 6, 4, 5], 3).contains(&6));
        assert!(banned_tokens(&[2, 4, 5], 0).is_empty());
    }
    #[test]
    fn rgb_bgr_normalization_contract() {
        let im =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([255, 127, 0])));
        let p = pixels(&im, 1, 1, true, [0.5; 3], [0.5; 3]);
        assert_eq!(p[0], -1.);
        assert_eq!(p[2], 1.);
    }
    #[test]
    fn filtering_outputs_cannot_reassign_overlapping_parent_lines() {
        let large = crate::Region {
            id: "large".into(),
            bbox: [0., 0., 100., 100.],
            orientation: "auto".into(),
            lines: vec![],
        };
        let small = crate::Region {
            id: "small".into(),
            bbox: [10., 10., 30., 30.],
            orientation: "auto".into(),
            lines: vec![],
        };
        let line = Segment {
            quad: bbox_quad([12., 12., 28., 28.]),
            vertical: false,
            score: 1.,
        };
        let page = Page {
            id: "test".into(),
            image: PathBuf::new(),
            regions: vec![large.clone()],
            association_regions: vec![large, small],
        };
        assert!(!associated_with(&line, &page, "large"));
        assert!(associated_with(&line, &page, "small"));
    }
}
