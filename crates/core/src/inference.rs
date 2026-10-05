use crate::types::*;
use anyhow::{Context, Result, bail};
use image::{DynamicImage, imageops::FilterType};
use ort::{session::Session, value::Tensor};
use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
};

pub fn init(runtime: &Path) -> Result<()> {
    ort::init_from(runtime.join(if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }))?
    .with_name("U-Manga")
    .commit();
    Ok(())
}
pub struct Inference {
    #[cfg(test)]
    recognized_crops: usize,
    pub root: PathBuf,
    bundled_root: PathBuf,
    sessions: HashMap<String, Session>,
    pub device_message: String,
    directml: bool,
    verified: std::collections::HashSet<String>,
    accelerator: Option<Box<Inference>>,
    accelerator_key: String,
    failed_accelerators: std::collections::HashSet<String>,
    options: Option<std::sync::Arc<ort::session::RunOptions>>,
    cancel: Option<tokio_util::sync::CancellationToken>,
    run_guard: Option<crate::inference_run::RunGuard>,
}
impl Inference {
    pub fn new(root: PathBuf) -> Self {
        Self::with_locations(crate::models::ModelLocations {
            bundled: root.clone(),
            downloaded: root,
        })
    }
    pub fn with_locations(locations: crate::models::ModelLocations) -> Self {
        Self {
            #[cfg(test)]
            recognized_crops: 0,
            root: locations.downloaded,
            bundled_root: locations.bundled,
            sessions: HashMap::new(),
            device_message: "CPU".into(),
            directml: false,
            verified: Default::default(),
            accelerator: None,
            accelerator_key: String::new(),
            failed_accelerators: Default::default(),
            options: None,
            cancel: None,
            run_guard: None,
        }
    }
    fn check_cancel(&self) -> Result<()> {
        anyhow::ensure!(
            self.cancel.as_ref().is_none_or(|c| !c.is_cancelled()),
            "Cancelled"
        );
        Ok(())
    }
    fn options(&mut self) -> Result<std::sync::Arc<ort::session::RunOptions>> {
        self.check_cancel()?;
        if let Some(options) = &self.options {
            return Ok(options.clone());
        }
        let options = std::sync::Arc::new(ort::session::RunOptions::new()?);
        if let Some(cancel) = &self.cancel {
            self.run_guard = Some(crate::inference_run::RunGuard::new(
                cancel.clone(),
                options.clone(),
            ));
        }
        self.options = Some(options.clone());
        Ok(options)
    }
    fn cancellable<T>(
        &mut self,
        cancel: &tokio_util::sync::CancellationToken,
        run: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        self.cancel = Some(cancel.clone());
        self.options = None;
        if let Some(gpu) = &mut self.accelerator {
            gpu.cancel = self.cancel.clone();
            gpu.options = None;
        }
        let result = run(self);
        self.run_guard = None;
        self.cancel = None;
        self.options = None;
        if let Some(gpu) = &mut self.accelerator {
            gpu.run_guard = None;
            gpu.cancel = None;
            gpu.options = None;
        }
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        result
    }
    pub fn detect_cancellable(
        &mut self,
        image: &DynamicImage,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<Region>> {
        self.cancellable(cancel, |s| s.detect(image))
    }
    pub fn recognize_cancellable(
        &mut self,
        crops: &[DynamicImage],
        settings: &TranslationSettings,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<String>> {
        self.cancellable(cancel, |s| s.recognize(crops, settings))
    }
    pub fn set_root(&mut self, root: PathBuf) {
        if self.root != root {
            self.root = root;
            self.sessions.clear();
            self.verified.clear();
            self.accelerator = None;
            self.accelerator_key.clear();
            self.failed_accelerators.clear();
        }
    }
    fn session(&mut self, pack: &str, file: &str) -> Result<&mut Session> {
        let root = if pack == "rtdetr_int8" {
            &self.bundled_root
        } else {
            &self.root
        };
        if !self.verified.contains(pack) {
            let spec = crate::models::catalog()?
                .into_iter()
                .find(|p| p.id == pack)
                .context("Unknown model pack")?;
            if !crate::models::verify_cancellable(
                root,
                &spec,
                self.cancel
                    .as_ref()
                    .unwrap_or(&tokio_util::sync::CancellationToken::new()),
            )? {
                if spec.distribution == "bundled" {
                    bail!(
                        "Included dialogue detector needs repair. Restart U-Manga to restore it from the executable."
                    );
                }
                bail!("Model pack {pack} is missing or corrupt; download/verify it in Settings")
            };
            self.verified.insert(pack.into());
        }
        let key = format!("{pack}/{file}");
        if !self.sessions.contains_key(&key) {
            let path = root.join(pack).join(file);
            if !path.is_file() {
                if pack == "rtdetr_int8" {
                    bail!(
                        "Included dialogue detector is missing. Restart U-Manga to restore it from the executable."
                    );
                }
                bail!("Download model pack {pack} first")
            };
            let mut builder = Session::builder()?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            if self.directml && pack != "rtdetr_int8" {
                builder = builder
                    .with_parallel_execution(false)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?
                    .with_memory_pattern(false)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?
                    .with_execution_providers([ort::ep::DirectML::default()
                        .build()
                        .error_on_failure()])
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            }
            let session = builder.commit_from_file(path)?;
            self.sessions.insert(key.clone(), session);
        }
        Ok(self.sessions.get_mut(&key).unwrap())
    }
    pub fn recognize(
        &mut self,
        crops: &[DynamicImage],
        settings: &TranslationSettings,
    ) -> Result<Vec<String>> {
        self.recognize_loaded(crops.len(), settings, &mut |index| {
            Ok(Cow::Borrowed(&crops[index]))
        })
    }
    pub fn recognize_regions_cancellable(
        &mut self,
        image: &DynamicImage,
        boxes: &[[f32; 4]],
        settings: &TranslationSettings,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<String>> {
        self.cancellable(cancel, |s| {
            s.recognize_loaded(boxes.len(), settings, &mut |index| {
                Ok(Cow::Owned(crop(image, boxes[index])))
            })
        })
    }
    fn recognize_range<'a>(
        &mut self,
        range: std::ops::Range<usize>,
        settings: &TranslationSettings,
        load: &mut impl FnMut(usize) -> Result<Cow<'a, DynamicImage>>,
    ) -> Result<Vec<String>> {
        if settings.ocr == "manga" {
            return self.manga_loaded(range, load);
        }
        let mut result = Vec::new();
        for index in range {
            self.check_cancel()?;
            let image = load(index)?;
            self.check_cancel()?;
            result.push(self.pp(&image, &settings.source_language)?);
        }
        Ok(result)
    }
    fn recognize_loaded<'a>(
        &mut self,
        count: usize,
        settings: &TranslationSettings,
        load: &mut impl FnMut(usize) -> Result<Cow<'a, DynamicImage>>,
    ) -> Result<Vec<String>> {
        self.check_cancel()?;
        let key = format!("{}:{}", settings.ocr, settings.source_language);
        let mut validated = Vec::new();
        if settings.device == "directml"
            && cfg!(windows)
            && count > 0
            && !self.failed_accelerators.contains(&key)
        {
            if self.accelerator_key != key {
                self.accelerator = None;
            }
            if self.accelerator.is_none() {
                let sample = 0..count.min(8);
                let reference = self.recognize_range(sample.clone(), settings, load)?;
                let mut gpu = Inference::new(self.root.clone());
                gpu.directml = true;
                gpu.cancel = self.cancel.clone();
                gpu.options = self.options.clone();
                let check = gpu.recognize_range(sample, settings, load);
                self.check_cancel()?;
                match check {
                    Ok(result) if result == reference => {
                        validated = result;
                        self.accelerator = Some(Box::new(gpu));
                        self.accelerator_key = key.clone();
                        self.device_message = "DirectML · output validated against CPU".into()
                    }
                    _ => {
                        validated = reference;
                        self.failed_accelerators.insert(key.clone());
                        self.device_message =
                            "CPU fallback · DirectML failed validation or is unavailable".into();
                    }
                }
            }
            if let Some(gpu) = &mut self.accelerator {
                let result = gpu.recognize_range(validated.len()..count, settings, load);
                if let Ok(rest) = result {
                    validated.extend(rest);
                    self.device_message = "DirectML · output validated against CPU".into();
                    return Ok(validated);
                }
                self.check_cancel()?;
                self.accelerator = None;
                self.failed_accelerators.insert(key.clone());
                self.device_message = "CPU fallback · DirectML execution failed".into();
            }
        } else {
            self.device_message =
                if settings.device == "directml" && self.failed_accelerators.contains(&key) {
                    "CPU fallback · DirectML failed validation"
                } else {
                    "CPU"
                }
                .into();
        }
        self.check_cancel()?;
        // Replay the same immutable inputs after a GPU failure, preserving the
        // validated sample and the original whole-request CPU fallback semantics.
        let rest = self.recognize_range(validated.len()..count, settings, load)?;
        validated.extend(rest);
        Ok(validated)
    }
    pub fn detect(&mut self, image: &DynamicImage) -> Result<Vec<Region>> {
        self.check_cancel()?;
        self.session("rtdetr_int8", "detector-v4-s_int8.onnx")?;
        let options = self.options()?;
        let pixels = rgb_tensor(image, 640, 640, false);
        let session = self.session("rtdetr_int8", "detector-v4-s_int8.onnx")?;
        let out=session.run_with_options(ort::inputs!["images"=>Tensor::from_array(([1,3,640,640],pixels))?,"orig_target_sizes"=>Tensor::from_array(([1,2],vec![image.width()as i64,image.height()as i64]))?], &*options)?;
        let (_, labels) = out["labels"].try_extract_tensor::<i64>()?;
        let (_, boxes) = out["boxes"].try_extract_tensor::<f32>()?;
        let (_, scores) = out["scores"].try_extract_tensor::<f32>()?;
        let mut raw = Vec::new();
        for i in 0..scores.len() {
            if scores[i] >= 0.3 {
                let b = clamp(
                    [
                        boxes[i * 4],
                        boxes[i * 4 + 1],
                        boxes[i * 4 + 2],
                        boxes[i * 4 + 3],
                    ],
                    image,
                );
                if b[2] - b[0] > 2. && b[3] - b[1] > 2. {
                    raw.push((labels[i], b, scores[i]));
                }
            }
        }
        let bubbles: Vec<_> = raw.iter().filter(|r| r.0 == 0).map(|r| r.1).collect();
        let mut regions: Vec<Region> = Vec::new();
        raw.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (label, b, score) in raw {
            if label == 0 {
                continue;
            }
            if regions
                .iter()
                .any(|r| intersection(r.bbox, b) / area(r.bbox).min(area(b)).max(1.) > 0.9)
            {
                continue;
            }
            let bubble = bubbles
                .iter()
                .filter(|bb| intersection(**bb, b) / area(b) > 0.6)
                .min_by(|a, b| area(**a).total_cmp(&area(**b)))
                .copied();
            let kind = if label == 2 { "free" } else { "text" };
            regions.push(Region {
                bbox: b,
                bubble,
                score,
                kind: kind.into(),
                style: TextStyle::detected(kind, bubble.is_some()),
                direction: "auto".into(),
                ..Default::default()
            });
        }
        order_detected_regions(&mut regions);
        Ok(regions)
    }
    pub fn manga(&mut self, crops: &[DynamicImage]) -> Result<Vec<String>> {
        self.manga_loaded(0..crops.len(), &mut |index| {
            Ok(Cow::Borrowed(&crops[index]))
        })
    }
    fn manga_loaded<'a>(
        &mut self,
        range: std::ops::Range<usize>,
        load: &mut impl FnMut(usize) -> Result<Cow<'a, DynamicImage>>,
    ) -> Result<Vec<String>> {
        self.check_cancel()?;
        if range.is_empty() {
            return Ok(vec![]);
        }
        #[cfg(test)]
        {
            self.recognized_crops += range.len();
        }
        let vocab = std::fs::read_to_string(self.root.join("manga_ocr_onnx/vocab.txt"))?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.session("manga_ocr_onnx", "encoder_model.onnx")?;
        let options = self.options()?;
        let cancel = self.cancel.clone();
        let mut result = Vec::new();
        // On CPU, finish each crop independently instead of decoding completed
        // rows until the longest text finishes. DirectML benefits from batching.
        let batch_size = if self.directml { 8 } else { 1 };
        for start in (range.start..range.end).step_by(batch_size) {
            self.check_cancel()?;
            let batch = start..(start + batch_size).min(range.end);
            let n = batch.len();
            let pixels = load_manga_pixels(batch, load, || self.check_cancel())?;
            let enc = self.session("manga_ocr_onnx", "encoder_model.onnx")?;
            let output = enc.run_with_options(
                ort::inputs![Tensor::from_array(([n, 3, 224, 224], pixels))?],
                &*options,
            )?;
            let (shape, data) = output[0].try_extract_tensor::<f32>()?;
            let hidden_shape = shape.to_vec();
            let hidden = data.to_vec();
            drop(output);
            let decoder = self.session("manga_ocr_onnx", "decoder_model.onnx")?;
            let mut tokens = vec![vec![2i64]; n];
            let mut done = vec![false; n];
            for _ in 0..160 {
                anyhow::ensure!(
                    cancel.as_ref().is_none_or(|c| !c.is_cancelled()),
                    "Cancelled"
                );
                let len = tokens[0].len();
                let ids = tokens.iter().flatten().copied().collect::<Vec<_>>();
                let output=decoder.run_with_options(ort::inputs!["input_ids"=>Tensor::from_array(([n,len],ids))?,"encoder_hidden_states"=>Tensor::from_array((hidden_shape.clone(),hidden.clone()))?], &*options)?;
                let (shape, logits) = output[0].try_extract_tensor::<f32>()?;
                let vs = *shape.last().context("Invalid decoder output")? as usize;
                let seq = shape[1] as usize;
                for b in 0..n {
                    if done[b] {
                        tokens[b].push(0);
                        continue;
                    }
                    let start = (b * seq + seq - 1) * vs;
                    let row = &logits[start..start + vs];
                    let prefix = &tokens[b];
                    let mut banned = std::collections::HashSet::new();
                    if prefix.len() >= 2 {
                        for w in prefix.windows(3) {
                            if w[..2] == prefix[prefix.len() - 2..] {
                                banned.insert(w[2]);
                            }
                        }
                    }
                    let id = row
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !banned.contains(&(*i as i64)))
                        .max_by(|a, b| a.1.total_cmp(b.1))
                        .context("Empty decoder vocabulary")?
                        .0 as i64;
                    tokens[b].push(id);
                    done[b] = id == 3;
                }
                if done.iter().all(|b| *b) {
                    break;
                }
            }
            for row in tokens {
                let text = row
                    .iter()
                    .skip(1)
                    .take_while(|id| **id != 3)
                    .filter_map(|id| vocab.get(*id as usize))
                    .filter(|v| !v.starts_with('['))
                    .map(|v| v.trim_start_matches("##"))
                    .collect::<String>();
                result.push(text);
            }
        }
        Ok(result)
    }
    pub fn pp(&mut self, im: &DynamicImage, language: &str) -> Result<String> {
        self.check_cancel()?;
        self.session("pp_det", "inference.onnx")?;
        let options = self.options()?;
        let cancel = self.cancel.clone();
        let pack = crate::models::catalog()?
            .into_iter()
            .find(|p| p.kind == "recognizer" && p.languages.iter().any(|l| l == language))
            .context("No local OCR pack for this language; select another mode explicitly")?;
        let ratio = 960. / im.width().max(im.height()) as f32;
        let w = ((im.width() as f32 * ratio / 32.).round().max(1.) as u32) * 32;
        let h = ((im.height() as f32 * ratio / 32.).round().max(1.) as u32) * 32;
        let mut data = rgb_tensor(im, w, h, false);
        let n = (w * h) as usize;
        for i in 0..n {
            data.swap(i, n * 2 + i)
        }
        for c in 0..3 {
            for v in &mut data[c * n..(c + 1) * n] {
                *v = (*v - [0.485, 0.456, 0.406][c]) / [0.229, 0.224, 0.225][c]
            }
        }
        let session = self.session("pp_det", "inference.onnx")?;
        let output = session.run_with_options(
            ort::inputs![Tensor::from_array(([1, 3, h as usize, w as usize], data))?],
            &*options,
        )?;
        let (shape, prob) = output[0].try_extract_tensor::<f32>()?;
        let mh = shape[shape.len() - 2] as u32;
        let mw = shape[shape.len() - 1] as u32;
        let mask = image::GrayImage::from_raw(
            mw,
            mh,
            prob.iter()
                .map(|p| if *p > 0.3 { 255 } else { 0 })
                .collect(),
        )
        .context("Invalid line mask")?;
        let mut lines = Vec::new();
        for contour in imageproc::contours::find_contours::<i32>(&mask)
            .into_iter()
            .filter(|c| c.parent.is_none())
            .take(1000)
        {
            anyhow::ensure!(
                cancel.as_ref().is_none_or(|c| !c.is_cancelled()),
                "Cancelled"
            );
            if contour.points.len() < 4 {
                continue;
            }
            let mut rect = imageproc::geometry::min_area_rect(&contour.points)
                .map(|p| (p.x as f32, p.y as f32));
            rect.sort_by(|a, b| a.1.total_cmp(&b.1));
            if rect[0].0 > rect[1].0 {
                rect.swap(0, 1)
            }
            if rect[2].0 > rect[3].0 {
                rect.swap(2, 3)
            }
            rect.swap(2, 3);
            let distance =
                |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            let rw = distance(rect[0], rect[1]);
            let rh = distance(rect[0], rect[3]);
            if rw.min(rh) < 3. {
                continue;
            }
            let minx = rect.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(0.) as u32;
            let maxx = rect.iter().map(|p| p.0).fold(0., f32::max).min(mw as f32) as u32;
            let miny = rect.iter().map(|p| p.1).fold(f32::MAX, f32::min).max(0.) as u32;
            let maxy = rect.iter().map(|p| p.1).fold(0., f32::max).min(mh as f32) as u32;
            let mut sum = 0.;
            let mut count = 0.;
            for y in miny..maxy {
                for x in minx..maxx {
                    if mask.get_pixel(x, y)[0] > 0 {
                        sum += prob[(y * mw + x) as usize];
                        count += 1.;
                    }
                }
            }
            if count == 0. || sum / count < 0.6 {
                continue;
            }
            let cx = rect.iter().map(|p| p.0).sum::<f32>() / 4.;
            let cy = rect.iter().map(|p| p.1).sum::<f32>() / 4.;
            let expand = rw * rh * 1.5 / (2. * (rw + rh));
            for p in &mut rect {
                let dx = p.0 - cx;
                let dy = p.1 - cy;
                p.0 = (cx + dx * (1. + 2. * expand / rw)) * im.width() as f32 / mw as f32;
                p.1 = (cy + dy * (1. + 2. * expand / rh)) * im.height() as f32 / mh as f32;
            }
            let dw = distance(rect[0], rect[1]).ceil().max(1.) as u32;
            let dh = distance(rect[0], rect[3]).ceil().max(1.) as u32;
            if dw > 10000 || dh > 10000 {
                continue;
            }
            let to = [
                (0., 0.),
                (dw as f32, 0.),
                (dw as f32, dh as f32),
                (0., dh as f32),
            ];
            if let Some(projection) =
                imageproc::geometric_transformations::Projection::from_control_points(rect, to)
            {
                let mut line = image::RgbImage::new(dw, dh);
                imageproc::geometric_transformations::warp_into(
                    &im.to_rgb8(),
                    &projection,
                    imageproc::geometric_transformations::Interpolation::Bilinear,
                    image::Rgb([255, 255, 255]),
                    &mut line,
                );
                let vertical = dh as f32 > dw as f32 * 1.5;
                let line = DynamicImage::ImageRgb8(line);
                lines.push((
                    cx,
                    cy,
                    vertical,
                    if vertical { line.rotate270() } else { line },
                ));
            }
        }
        drop(output);
        let vertical = lines.iter().filter(|l| l.2).count() * 2 > lines.len();
        lines.sort_by(|a, b| {
            if vertical {
                b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1))
            } else {
                a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0))
            }
        });
        let yaml: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(
            self.root.join(&pack.id).join("inference.yml"),
        )?)?;
        let mut dictionary = vec![String::new()];
        for v in yaml["PostProcess"]["character_dict"]
            .as_sequence()
            .context("Missing OCR dictionary")?
        {
            dictionary.push(v.as_str().unwrap_or("").to_owned())
        }
        dictionary.push(" ".into());
        let mut result = Vec::new();
        for batch in lines.chunks(8) {
            self.check_cancel()?;
            let widths = batch
                .iter()
                .map(|l| {
                    ((l.3.width() as f32 / l.3.height() as f32 * 48.).ceil() as usize)
                        .clamp(16, 3200)
                })
                .collect::<Vec<_>>();
            let width = *widths.iter().max().unwrap();
            let mut pixels = vec![0.; batch.len() * 3 * 48 * width];
            for (b, line) in batch.iter().enumerate() {
                let img = line
                    .3
                    .resize_exact(widths[b] as u32, 48, FilterType::Triangle)
                    .to_rgb8();
                for (y, row) in img.rows().enumerate() {
                    for (x, p) in row.enumerate() {
                        for c in 0..3 {
                            pixels[((b * 3 + c) * 48 + y) * width + x] =
                                (p[2 - c] as f32 / 255. - 0.5) / 0.5;
                        }
                    }
                }
            }
            let session = self.session(&pack.id, "inference.onnx")?;
            let output = session.run_with_options(
                ort::inputs![Tensor::from_array(([batch.len(), 3, 48, width], pixels))?],
                &*options,
            )?;
            let (shape, prob) = output[0].try_extract_tensor::<f32>()?;
            let steps = shape[1] as usize;
            let classes = shape[2] as usize;
            for b in 0..batch.len() {
                let mut text = String::new();
                let mut last = 0;
                for t in 0..steps {
                    let row = &prob[(b * steps + t) * classes..(b * steps + t + 1) * classes];
                    let id = row
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.total_cmp(b.1))
                        .unwrap()
                        .0;
                    if id != 0
                        && id != last
                        && let Some(ch) = dictionary.get(id)
                    {
                        text.push_str(ch)
                    }
                    last = id;
                }
                result.push(text);
            }
        }
        Ok(result.join(if vertical { "" } else { "\n" }))
    }
}
pub fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.) * (b[3] - b[1]).max(0.)
}
pub fn intersection(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0.) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.)
}
pub fn clamp(b: [f32; 4], im: &DynamicImage) -> [f32; 4] {
    [
        b[0].max(0.).min(im.width() as f32 - 1.),
        b[1].max(0.).min(im.height() as f32 - 1.),
        b[2].max(1.).min(im.width() as f32),
        b[3].max(1.).min(im.height() as f32),
    ]
}
pub fn crop(im: &DynamicImage, b: [f32; 4]) -> DynamicImage {
    let b = clamp(
        [
            b[0].floor() - 2.,
            b[1].floor() - 2.,
            b[2].floor() + 3.,
            b[3].floor() + 3.,
        ],
        im,
    );
    im.crop_imm(
        b[0] as u32,
        b[1] as u32,
        (b[2] - b[0]).max(1.) as u32,
        (b[3] - b[1]).max(1.) as u32,
    )
}
/// Order newly detected regions in fixed rows, without changing any region data.
/// Saved pages retain their persisted order; this runs only at detector output.
fn order_detected_regions(regions: &mut Vec<Region>) {
    regions.sort_by(|a, b| {
        a.bbox[1]
            .total_cmp(&b.bbox[1])
            .then(b.bbox[0].total_cmp(&a.bbox[0]))
    });
    let mut pending = std::mem::take(regions);
    while let Some(leader) = pending.first() {
        // Convert before subtraction to match the frozen experiment at boundaries.
        let top = f64::from(leader.bbox[1]);
        let limit = top + (f64::from(leader.bbox[3]) - top) * 0.35;
        let (mut row, rest): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|region| f64::from(region.bbox[1]) <= limit);
        row.sort_by(|a, b| b.bbox[0].total_cmp(&a.bbox[0]));
        regions.append(&mut row);
        pending = rest;
    }
}

fn rgb_tensor(im: &DynamicImage, w: u32, h: u32, normalize: bool) -> Vec<f32> {
    let resized = im.resize_exact(w, h, FilterType::Triangle).to_rgb8();
    let n = (w * h) as usize;
    let mut data = vec![0.; n * 3];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            let v = p[c] as f32 / 255.;
            data[c * n + i] = if normalize { (v - 0.5) / 0.5 } else { v };
        }
    }
    data
}

fn manga_pixels(im: &DynamicImage) -> Vec<f32> {
    let rgb = im.to_rgb8();
    let gray = image::GrayImage::from_fn(rgb.width(), rgb.height(), |x, y| {
        let p = rgb.get_pixel(x, y);
        image::Luma([
            ((p[0] as u32 * 19595 + p[1] as u32 * 38470 + p[2] as u32 * 7471 + 32768) >> 16) as u8,
        ])
    });
    rgb_tensor(&DynamicImage::ImageLuma8(gray), 224, 224, true)
}

// Only one full-size crop lives at a time. GPU batches retain fixed-size tensors,
// rather than a page-sized DynamicImage for every saved region.
fn load_manga_pixels<'a>(
    range: std::ops::Range<usize>,
    load: &mut impl FnMut(usize) -> Result<Cow<'a, DynamicImage>>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Vec<f32>> {
    let mut pixels = Vec::with_capacity(range.len() * 3 * 224 * 224);
    for index in range {
        check()?;
        let image = load(index)?;
        check()?;
        pixels.extend(manga_pixels(&image));
        check()?;
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(id: &str, bbox: [f32; 4]) -> Region {
        Region {
            id: id.into(),
            bbox,
            ..Default::default()
        }
    }

    fn ordered_ids(regions: &[Region]) -> Vec<&str> {
        regions.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn row_order_empty_single_and_staggered_rows() {
        let mut empty = vec![];
        order_detected_regions(&mut empty);
        assert!(empty.is_empty());
        let mut one = vec![detected("one", [10., 20., 50., 80.])];
        let original = serde_json::to_value(&one).unwrap();
        order_detected_regions(&mut one);
        assert_eq!(serde_json::to_value(&one).unwrap(), original);
        let mut regions = vec![
            detected("lower-left", [10., 210., 40., 310.]),
            detected("upper-left", [10., 0., 40., 100.]),
            detected("lower-right", [90., 220., 120., 280.]),
            detected("upper-right", [90., 10., 120., 70.]),
        ];
        order_detected_regions(&mut regions);
        assert_eq!(
            ordered_ids(&regions),
            ["upper-right", "upper-left", "lower-right", "lower-left"]
        );
    }

    #[test]
    fn row_order_boundary_is_inclusive_and_uses_only_leader_height() {
        let mut regions = vec![
            detected("leader", [10., 0., 30., 100.]),
            detected("boundary", [40., 35., 60., 100.]),
            detected(
                "before",
                [60., f32::from_bits(35_f32.to_bits() - 1), 80., 100.],
            ),
            detected(
                "after",
                [90., f32::from_bits(35_f32.to_bits() + 1), 110., 100.],
            ),
        ];
        order_detected_regions(&mut regions);
        assert_eq!(
            ordered_ids(&regions),
            ["before", "boundary", "leader", "after"]
        );
        let mut regions = vec![
            detected("leader", [10., 0., 30., 100.]),
            detected("tall-member", [40., 34., 60., 500.]),
            detected("next-row", [90., 36., 110., 100.]),
        ];
        order_detected_regions(&mut regions);
        assert_eq!(ordered_ids(&regions), ["tall-member", "leader", "next-row"]);
    }

    #[test]
    fn row_order_is_stable_idempotent_and_preserves_all_region_fields() {
        let mut regions = vec![
            detected("late", [40., 20., 60., 70.]),
            detected("leader", [10., 0., 30., 100.]),
            detected("early-a", [40., 10., 60., 70.]),
            detected("early-b", [40., 10., 60., 70.]),
        ];
        for (i, region) in regions.iter_mut().enumerate() {
            region.bubble = Some([0., 0., 150., 150.]);
            region.kind = if i % 2 == 0 { "free" } else { "text" }.into();
            region.score = i as f32 / 10.;
            region.source = format!("source {i}");
            region.target = format!("manual，text。·=！\nline {i}");
            region.direction = "vertical".into();
            region.style.size = Some(11. + i as f32);
            region.style.outline_enabled = true;
            region.allow_fill = true;
            region.overlay_only = true;
            region.prepared = true;
            region.review = Some("Existing review".into());
        }
        let original = regions.clone();
        order_detected_regions(&mut regions);
        assert_eq!(
            ordered_ids(&regions),
            ["early-a", "early-b", "late", "leader"]
        );
        for region in &regions {
            assert_eq!(
                serde_json::to_value(region).unwrap(),
                serde_json::to_value(original.iter().find(|r| r.id == region.id).unwrap()).unwrap()
            );
        }
        let ordered = serde_json::to_value(&regions).unwrap();
        order_detected_regions(&mut regions);
        assert_eq!(serde_json::to_value(&regions).unwrap(), ordered);
    }

    #[test]
    fn row_order_replays_frozen_nineteen_page_observations() {
        #[derive(serde::Deserialize)]
        struct FrozenRegion {
            id: String,
            bbox: [f32; 4],
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Case {
            page: String,
            regions: Vec<FrozenRegion>,
            expected_order: Vec<String>,
            before: Vec<[String; 2]>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fixture {
            observations_sha256: String,
            annotations_sha256: String,
            expected_regions: usize,
            expected_pairs: usize,
            expected_errors: usize,
            pages: Vec<Case>,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("../tests/fixtures/reading-order-v1.json")).unwrap();
        assert_eq!(fixture.pages.len(), 19);
        let (mut count, mut pairs, mut errors) = (0, 0, 0);
        let mut results = Vec::new();
        for case in fixture.pages {
            let mut regions = case
                .regions
                .iter()
                .map(|r| detected(&r.id, r.bbox))
                .collect::<Vec<_>>();
            let original = regions.clone();
            order_detected_regions(&mut regions);
            assert_eq!(ordered_ids(&regions), case.expected_order, "{}", case.page);
            for region in &regions {
                assert_eq!(
                    serde_json::to_value(region).unwrap(),
                    serde_json::to_value(original.iter().find(|r| r.id == region.id).unwrap())
                        .unwrap()
                );
            }
            let rank = regions
                .iter()
                .enumerate()
                .map(|(i, r)| (r.id.as_str(), i))
                .collect::<HashMap<_, _>>();
            let wrong = case
                .before
                .iter()
                .filter(|[a, b]| rank[a.as_str()] > rank[b.as_str()])
                .count();
            count += regions.len();
            pairs += case.before.len();
            errors += wrong;
            results.push(serde_json::json!({"page":case.page,"regions":regions.len(),"errors":wrong,"pairs":case.before.len(),"order":ordered_ids(&regions)}));
        }
        assert_eq!(count, fixture.expected_regions);
        assert_eq!(pairs, fixture.expected_pairs);
        assert_eq!(errors, fixture.expected_errors);
        let output =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-output/row-ordering");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("corpus-replay.json"), serde_json::to_vec_pretty(&serde_json::json!({"method":"Production row sorter; replay of frozen boxes, no inference","pages":19,"regions":count,"pairs":pairs,"errors":errors,"observationsSha256":fixture.observations_sha256,"annotationsSha256":fixture.annotations_sha256,"rows":results})).unwrap()).unwrap();
    }

    #[test]
    fn row_order_flows_into_requests_and_context_but_does_not_reorder_saved_pages() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("source.png");
        image::RgbImage::new(200, 300).save(&image).unwrap();
        let mut page = crate::documents::import(&[image.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        page.regions = vec![
            detected("left", [10., 0., 40., 100.]),
            detected("right", [90., 10., 120., 80.]),
        ];
        for r in &mut page.regions {
            r.source = r.id.clone();
            r.target = "manual，text。·=！\nnext column".into();
        }
        let path = temp.path().join("saved.umanga");
        crate::store::create(&path, "Saved order", &[page.clone()]).unwrap();
        let original = crate::store::page(&path, &page.id).unwrap();
        let settings = TranslationSettings::default();
        let provider = ProviderProfile::default();
        let old_key = crate::store::cache_key(&original, &settings, &provider, "").unwrap();
        order_detected_regions(&mut page.regions);
        assert_eq!(
            crate::text_batches::current_text(&page.regions, true).unwrap(),
            "[1] right\n[2] left"
        );
        let structured = crate::text_batches::current_text(&page.regions, false).unwrap();
        assert!(structured.find("right").unwrap() < structured.find("left").unwrap());
        assert_eq!(
            crate::providers::translation_schema(&page.regions)["properties"]["regions"]["items"]["properties"]
                ["id"]["enum"],
            serde_json::json!(["right", "left"])
        );
        assert_ne!(
            old_key,
            crate::store::cache_key(&page, &settings, &provider, "").unwrap()
        );
        assert_eq!(
            crate::store::preceding_context(&path, 1, 1).unwrap().pages[0].sources,
            ["left", "right"]
        );
        assert_eq!(
            serde_json::to_value(crate::store::page(&path, &page.id).unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        let path = temp.path().join("new.umanga");
        crate::store::create(&path, "New order", &[page.clone()]).unwrap();
        assert_eq!(
            crate::store::preceding_context(&path, 1, 1).unwrap().pages[0].sources,
            ["right", "left"]
        );
        assert_eq!(
            serde_json::to_value(crate::store::page(&path, &page.id).unwrap()).unwrap(),
            serde_json::to_value(page).unwrap()
        );
    }

    #[test]
    fn lazy_manga_tensors_preserve_crop_order_pixels_and_cancellation() {
        let crops = (0..11)
            .map(|i| {
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    17 + i,
                    25 + i,
                    image::Rgb([i as u8, 99, 201]),
                ))
            })
            .collect::<Vec<_>>();
        let expected = crops.iter().flat_map(manga_pixels).collect::<Vec<_>>();
        let mut indices = Vec::new();
        let actual = load_manga_pixels(
            0..crops.len(),
            &mut |i| {
                indices.push(i);
                Ok(Cow::Borrowed(&crops[i]))
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(indices, (0..11).collect::<Vec<_>>());
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut loaded = 0;
        let result = load_manga_pixels(
            0..400,
            &mut |_| {
                loaded += 1;
                if loaded == 2 {
                    cancel.cancel();
                }
                Ok(Cow::Owned(DynamicImage::new_rgb8(10, 10)))
            },
            || {
                anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
                Ok(())
            },
        );
        assert!(result.unwrap_err().to_string().contains("Cancelled"));
        assert_eq!(loaded, 2, "no later crop may be loaded after cancellation");
        let mut infer = Inference::new(PathBuf::new());
        let settings = TranslationSettings::default();
        assert!(
            infer
                .recognize_regions_cancellable(&crops[0], &[[0., 0., 17., 25.]], &settings, &cancel)
                .unwrap_err()
                .to_string()
                .contains("Cancelled")
        );
        assert!(infer.sessions.is_empty());
    }

    #[test]
    fn cancellation_and_missing_models_do_not_initialize_or_fallback() {
        let root = tempfile::tempdir().unwrap();
        let mut infer = Inference::new(root.path().to_owned());
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let image = DynamicImage::new_rgb8(20, 30);
        let settings = TranslationSettings::default();
        assert!(
            infer
                .detect_cancellable(&image, &cancel)
                .unwrap_err()
                .to_string()
                .contains("Cancelled")
        );
        assert!(
            infer
                .recognize_cancellable(std::slice::from_ref(&image), &settings, &cancel)
                .unwrap_err()
                .to_string()
                .contains("Cancelled")
        );
        assert!(infer.failed_accelerators.is_empty());
        assert!(infer.options.is_none());
        assert!(infer.sessions.is_empty());
        let cancel = tokio_util::sync::CancellationToken::new();
        assert!(infer.detect_cancellable(&image, &cancel).is_err());
        assert!(infer.options.is_none());
        assert!(infer.run_guard.is_none());
        assert!(infer.cancel.is_none());
        assert!(
            infer
                .recognize_cancellable(&[], &settings, &cancel)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    #[ignore = "requires prepared Manga OCR assets and U_MANGA_OCR_PARITY_FIXTURE"]
    fn manga_cpu_fallback_preserves_frozen_text_and_order() -> Result<()> {
        #[derive(serde::Deserialize)]
        struct Case {
            image: PathBuf,
            bbox: [f32; 4],
            expected: String,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            runtime: PathBuf,
            model_root: PathBuf,
            cases: Vec<Case>,
        }
        let fixture: Fixture = serde_json::from_slice(&std::fs::read(std::env::var(
            "U_MANGA_OCR_PARITY_FIXTURE",
        )?)?)?;
        init(&fixture.runtime)?;
        let crops = fixture
            .cases
            .iter()
            .map(|case| Ok(crop(&image::open(&case.image)?, case.bbox)))
            .collect::<Result<Vec<_>>>()?;
        let expected = fixture
            .cases
            .iter()
            .map(|case| case.expected.clone())
            .collect::<Vec<_>>();
        assert!(crops.len() > 8 && crops.len() % 8 != 0);
        let mut inference = Inference::new(fixture.model_root.clone());
        // Simulate a rejected accelerator. The requested device must not make
        // this CPU instance use GPU-sized batches or change crop ordering.
        inference.failed_accelerators.insert("manga:ja".into());
        let settings = TranslationSettings {
            ocr: "manga".into(),
            source_language: "ja".into(),
            device: "directml".into(),
            ..Default::default()
        };
        assert_eq!(inference.recognize(&crops, &settings)?, expected);
        assert!(inference.device_message.starts_with("CPU fallback"));
        assert!(inference.accelerator.is_none());
        let reversed = crops.iter().cloned().rev().collect::<Vec<_>>();
        assert_eq!(
            inference.recognize(&reversed, &settings)?,
            expected.iter().cloned().rev().collect::<Vec<_>>()
        );
        assert!(inference.recognize(&[], &settings)?.is_empty());
        assert_eq!(inference.sessions.len(), 2);
        assert!(
            inference
                .sessions
                .keys()
                .all(|key| key.starts_with("manga_ocr_onnx/"))
        );
        assert_eq!(inference.recognized_crops, crops.len() * 2);
        drop(inference);
        let mut gpu = Inference::new(fixture.model_root);
        assert_eq!(gpu.recognize(&crops, &settings)?, expected);
        let accelerator = gpu
            .accelerator
            .as_ref()
            .context("DirectML parity unavailable on this machine")?;
        assert_eq!(
            accelerator.recognized_crops,
            crops.len(),
            "Validation crops must not run twice on DirectML"
        );
        assert_eq!(
            gpu.recognized_crops, 8,
            "Only the validation sample should run on CPU"
        );
        assert_eq!(gpu.recognize(&crops[..2], &settings)?, expected[..2]);
        assert!(gpu.device_message.starts_with("DirectML"));
        assert_eq!(
            gpu.accelerator.as_ref().unwrap().recognized_crops,
            crops.len() + 2
        );
        let report = serde_json::json!({"crops":crops.len(),"cpuValidationCrops":8,"gpuCropsFirstRequest":crops.len(),"gpuCropsSecondRequest":2,"textParity":true});
        std::fs::write(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-output/correctness/ocr-calls.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(())
    }
}
