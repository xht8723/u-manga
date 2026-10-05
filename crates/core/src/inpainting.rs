//! Single-session background inpainting, shared by Jobs and the Editor.
use crate::{models, types::*};
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Rgb, RgbImage};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
#[path = "inpainting_tensor.rs"]
mod tensor;
pub use tensor::ModelSession;
#[path = "inpainting_geometry.rs"]
mod geometry;
pub use geometry::{compose, reflect, tile_touches};
pub const ALGORITHM_VERSION: &str = "fixed-context-1";
pub type Progress = Arc<dyn Fn(&str, usize, usize) + Send + Sync>;

/// Identical 512/64/128 geometry to the frozen benchmark, including edge anchoring.
pub fn tiles(mask: &GrayImage) -> Vec<[i32; 2]> {
    let mut bounds = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    for (x, y, p) in mask.enumerate_pixels() {
        if p[0] > 0 {
            bounds[0] = bounds[0].min(x as i32);
            bounds[1] = bounds[1].min(y as i32);
            bounds[2] = bounds[2].max(x as i32);
            bounds[3] = bounds[3].max(y as i32);
        }
    }
    if bounds[0] == i32::MAX {
        return vec![];
    }
    fn axis(a: i32, b: i32) -> Vec<i32> {
        if b - a <= 512 {
            return vec![(a + b - 512).div_euclid(2)];
        }
        let mut v: Vec<_> = (a..=b - 512).step_by(384).collect();
        if v.last() != Some(&(b - 512)) {
            v.push(b - 512)
        }
        v
    }
    let mut out = vec![];
    for y in axis(bounds[1] - 64, bounds[3] + 65) {
        for x in axis(bounds[0] - 64, bounds[2] + 65) {
            if tile_touches(mask, x, y) {
                out.push([x, y]);
            }
        }
    }
    out
}

pub struct Request {
    pub image: RgbImage,
    pub full_mask: GrayImage,
    pub mask: GrayImage,
    pub base: RgbImage,
    pub settings: CleanupSettings,
    pub directory: PathBuf,
    pub cancel: CancellationToken,
    pub progress: Progress,
    pub retry: bool,
}
pub struct Output {
    pub image: RgbImage,
    pub device: String,
}
struct Message {
    request: Request,
    reply: mpsc::SyncSender<Result<Output>>,
}
pub struct Worker {
    tx: mpsc::Sender<Message>,
}
impl Worker {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        std::thread::Builder::new()
            .name("u-manga-inpainting".into())
            .spawn(move || {
                let mut session: Option<(String, ModelSession)> = None;
                let mut failed_gpu = std::collections::HashSet::new();
                let mut verification = models::VerificationCache::default();
                loop {
                    match rx.recv_timeout(Duration::from_secs(60)) {
                        Ok(m) => {
                            let result =
                                run(&m.request, &mut session, &mut failed_gpu, &mut verification);
                            let _ = m.reply.send(result);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            session = None;
                            failed_gpu.clear();
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("Cannot start native cleanup worker");
        Self { tx }
    }
    pub fn process(&self, request: Request) -> Result<Output> {
        let cancel = request.cancel.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .send(Message { request, reply: tx })
            .map_err(|_| anyhow::anyhow!("Cleanup worker stopped"))?;
        // Wait for the actual worker to exit, even after cancellation: no overlapping attempt.
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(r) => return r,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = cancel.is_cancelled();
                }
                Err(_) => anyhow::bail!("Cleanup worker stopped"),
            }
        }
    }
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}

fn run(
    r: &Request,
    session: &mut Option<(String, ModelSession)>,
    failed_gpu: &mut std::collections::HashSet<String>,
    verification: &mut models::VerificationCache,
) -> Result<Output> {
    ensure!(!r.cancel.is_cancelled(), "Cancelled");
    let pack_id = r
        .settings
        .method
        .pack()
        .context("Solid fill does not need inpainting")?;
    let pack = models::catalog()?
        .into_iter()
        .find(|p| p.id == pack_id)
        .context("Unknown cleanup model")?;
    ensure!(
        verification.check_cancellable(&r.directory, &pack, false, &r.cancel)?,
        "{} is missing or damaged. Download or verify it in Settings → Translation → Local models.",
        pack.name
    );
    let path = r.directory.join(pack_id).join(&pack.files[0].name);
    let identity = format!("{}:{}", path.display(), pack.files[0].sha256);
    if r.retry {
        failed_gpu.remove(&identity);
    }
    let gpu = r.settings.device != CleanupDevice::Cpu
        && !failed_gpu.contains(&identity)
        && cfg!(target_os = "windows");
    let geometry = tiles(&r.full_mask);
    attempt_backends(&r.cancel, r.settings.device, gpu, |provider, label| {
        (r.progress)(
            label,
            0,
            geometry
                .iter()
                .filter(|[x, y]| tile_touches(&r.mask, *x, *y))
                .count(),
        );
        let result = (|| -> Result<RgbImage> {
            let key = format!("{identity}:{provider}");
            if session.as_ref().is_none_or(|(k, _)| k != &key) {
                *session = None;
                *session = Some((
                    key,
                    ModelSession::new(r.settings.method.id(), &path, provider, 4, None)?,
                ));
            }
            ensure!(!r.cancel.is_cancelled(), "Cancelled");
            let s = &mut session.as_mut().unwrap().1;
            s.options = Arc::new(ort::session::RunOptions::new()?);
            let _guard = crate::inference_run::RunGuard::new(r.cancel.clone(), s.options.clone());
            Ok(compose(
                &r.image,
                &r.full_mask,
                &r.mask,
                &geometry,
                r.base.clone(),
                s,
                &r.cancel,
                &|n, total| (r.progress)(label, n, total),
            )?
            .0)
        })();
        if result.is_err() {
            *session = None;
            if provider == "directml" && !r.cancel.is_cancelled() {
                failed_gpu.insert(identity.clone());
            }
        }
        result
    })
}
fn attempt_backends(
    cancel: &CancellationToken,
    device: CleanupDevice,
    gpu: bool,
    mut attempt: impl FnMut(&str, &str) -> Result<RgbImage>,
) -> Result<Output> {
    let providers = if gpu {
        vec!["directml", "cpu"]
    } else {
        vec!["cpu"]
    };
    let mut gpu_error = None;
    for provider in providers {
        ensure!(!cancel.is_cancelled(), "Cancelled");
        let label = if provider == "directml" {
            "DirectML"
        } else if device != CleanupDevice::Cpu {
            "CPU fallback"
        } else {
            "CPU"
        };
        let result = attempt(provider, label);
        ensure!(!cancel.is_cancelled(), "Cancelled");
        match result {
            Ok(image) => {
                return Ok(Output {
                    image,
                    device: label.into(),
                });
            }
            Err(e) => {
                if provider == "directml" {
                    gpu_error = Some(e.to_string());
                } else {
                    return Err(anyhow::anyhow!(
                        "CPU inpainting failed: {e}{}",
                        gpu_error
                            .map(|g| format!("; DirectML failed: {g}"))
                            .unwrap_or_default()
                    ));
                }
            }
        }
    }
    anyhow::bail!("No cleanup backend available")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backend_fallback_discards_partial_gpu_results_and_cancellation_never_retries() {
        let cancel = CancellationToken::new();
        let mut calls = vec![];
        let out = attempt_backends(&cancel, CleanupDevice::Auto, true, |provider, _| {
            calls.push(provider.to_string());
            if provider == "directml" {
                anyhow::bail!("GPU lost")
            }
            Ok(RgbImage::new(2, 2))
        })
        .unwrap();
        assert_eq!(calls, vec!["directml", "cpu"]);
        assert_eq!(out.device, "CPU fallback");
        calls.clear();
        let result = attempt_backends(&cancel, CleanupDevice::Directml, true, |provider, _| {
            calls.push(provider.to_string());
            anyhow::bail!("unavailable")
        });
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("CPU inpainting failed")
        );
        assert_eq!(calls.len(), 2);
        calls.clear();
        let result = attempt_backends(&cancel, CleanupDevice::Auto, true, |provider, _| {
            calls.push(provider.to_string());
            cancel.cancel();
            anyhow::bail!("terminated")
        });
        assert_eq!(result.err().unwrap().to_string(), "Cancelled");
        assert_eq!(calls, vec!["directml"]);
    }
    #[test]
    fn tile_geometry_keeps_edges_and_full_context() {
        let mut mask = GrayImage::new(1000, 1000);
        mask.put_pixel(0, 0, image::Luma([255]));
        mask.put_pixel(999, 999, image::Luma([255]));
        let full = tiles(&mask);
        assert!(full.contains(&[-64, -64]));
        assert!(full.contains(&[552, 552]));
        mask.put_pixel(0, 0, image::Luma([0]));
        let remaining = full
            .iter()
            .filter(|[x, y]| tile_touches(&mask, *x, *y))
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![[552, 552]]);
        assert_ne!(remaining, tiles(&mask));
    }
}
