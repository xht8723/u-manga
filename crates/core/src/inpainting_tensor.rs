//! Shared, benchmarked ONNX adapters. No framework-specific native dependencies.
use anyhow::{Result, ensure};
use ort::{session::Session, value::Tensor};
use std::time::Instant;
pub struct ModelSession {
    pub session: Option<Session>,
    pub options: std::sync::Arc<ort::session::RunOptions>,
    adapter: String,
    names: Vec<String>,
}
impl ModelSession {
    pub fn new(
        adapter: &str,
        model: &std::path::Path,
        provider: &str,
        threads: usize,
        profile: Option<&std::path::Path>,
    ) -> Result<Self> {
        if adapter == "solid" {
            return Ok(Self {
                session: None,
                options: std::sync::Arc::new(ort::session::RunOptions::new()?),
                adapter: adapter.into(),
                names: vec![],
            });
        }
        let err =
            |e: ort::Error<ort::session::builder::SessionBuilder>| anyhow::anyhow!(e.to_string());
        let mut b = Session::builder()?
            .with_intra_threads(threads)
            .map_err(err)?
            .with_inter_threads(1)
            .map_err(err)?
            .with_parallel_execution(false)
            .map_err(err)?;
        for (name, size) in [
            ("batch", 1),
            ("batch_size", 1),
            ("h", 512),
            ("w", 512),
            ("height", 512),
            ("width", 512),
        ] {
            b = b.with_dimension_override(name, size).map_err(err)?;
        }
        if provider == "directml" {
            b = b
                .with_memory_pattern(false)
                .map_err(err)?
                .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
                .map_err(err)?;
        }
        if let Some(profile) = profile {
            b = b.with_profiling(profile).map_err(err)?;
        }
        let s = b.commit_from_file(model)?;
        let names = s.inputs().iter().map(|v| v.name().to_string()).collect();
        Ok(Self {
            session: Some(s),
            options: std::sync::Arc::new(ort::session::RunOptions::new()?),
            adapter: adapter.into(),
            names,
        })
    }
    pub fn infer(&mut self, rgb: &[u8], mask: &[u8]) -> Result<(Vec<u8>, f64, f64)> {
        let before = Instant::now();
        let n = 512 * 512;
        let adapter = self.adapter.as_str();
        if adapter == "migan" {
            let mut chw = vec![0u8; n * 3];
            for i in 0..n {
                for c in 0..3 {
                    chw[c * n + i] = rgb[i * 3 + c];
                }
            }
            let known: Vec<u8> = mask.iter().map(|&v| if v > 0 { 0 } else { 255 }).collect();
            let it = Tensor::from_array(([1usize, 3, 512, 512], chw))?;
            let mt = Tensor::from_array(([1usize, 1, 512, 512], known))?;
            let pre = before.elapsed().as_secs_f64() * 1000.;
            let now = Instant::now();
            let out = self.session.as_mut().unwrap().run_with_options(
                ort::inputs![self.names[0].as_str()=>it,self.names[1].as_str()=>mt],
                &self.options,
            )?;
            let (_, data) = out[0].try_extract_tensor::<u8>()?;
            ensure!(data.len() == n * 3, "Unexpected MI-GAN output");
            let mut result = vec![0u8; n * 3];
            for i in 0..n {
                for c in 0..3 {
                    result[i * 3 + c] = data[c * n + i];
                }
            }
            return Ok((result, pre, now.elapsed().as_secs_f64() * 1000.));
        }
        let mut im = vec![0f32; n * 3];
        let mut m = vec![0f32; n];
        for i in 0..n {
            m[i] = if mask[i] > 0 { 1. } else { 0. };
            for ch in 0..3 {
                let x = rgb[i * 3 + ch] as f32 / 255.;
                im[ch * n + i] = match adapter {
                    "aot" => (x * 2. - 1.) * (1. - m[i]) + m[i],
                    "manga_aot" => (x * 2. - 1.) * (1. - m[i]),
                    "migan" => (x * 2. - 1.) * (1. - m[i]),
                    _ => x,
                };
            }
        }
        let it = Tensor::from_array(([1usize, 3, 512, 512], im))?;
        let mt = Tensor::from_array(([1usize, 1, 512, 512], m))?;
        let pre = before.elapsed().as_secs_f64() * 1000.;
        let now = Instant::now();
        let s = self.session.as_mut().unwrap();
        let out = s.run_with_options(
            ort::inputs![self.names[0].as_str()=>it,self.names[1].as_str()=>mt],
            &self.options,
        )?;
        let (_, data) = out[0].try_extract_tensor::<f32>()?;
        ensure!(
            data.len() == n * 3,
            "Unexpected output length {}",
            data.len()
        );
        let mut result = vec![0u8; n * 3];
        for i in 0..n {
            for ch in 0..3 {
                let x = data[ch * n + i];
                ensure!(x.is_finite(), "Non-finite model output");
                let x = match adapter {
                    "aot" | "manga_aot" => (x + 1.) * 127.5,
                    "big_lama" => x,
                    _ => x * 255.,
                };
                result[i * 3 + ch] = x.round().clamp(0., 255.) as u8;
            }
        }
        Ok((result, pre, now.elapsed().as_secs_f64() * 1000.))
    }
}
