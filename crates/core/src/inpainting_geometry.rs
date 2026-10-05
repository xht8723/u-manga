use super::*;
pub fn reflect(v: i32, n: u32) -> u32 {
    let n = n as i32;
    if n <= 1 {
        return 0;
    }
    let mut x = v;
    while x < 0 || x >= n {
        x = if x < 0 { -x } else { 2 * n - 2 - x };
    }
    x as u32
}

pub fn tile_touches(mask: &GrayImage, ox: i32, oy: i32) -> bool {
    let x0 = ox.clamp(0, mask.width() as i32) as u32;
    let x1 = (ox + 512).clamp(0, mask.width() as i32) as u32;
    let y0 = oy.clamp(0, mask.height() as i32) as u32;
    let y1 = (oy + 512).clamp(0, mask.height() as i32) as u32;
    (y0..y1).any(|y| (x0..x1).any(|x| mask.get_pixel(x, y)[0] != 0))
}

#[allow(
    clippy::too_many_arguments,
    reason = "Explicit processing inputs preserve the shared production/benchmark and captured-job contracts"
)]
pub fn compose(
    image: &RgbImage,
    input_mask: &GrayImage,
    mask: &GrayImage,
    tiles: &[[i32; 2]],
    mut output: RgbImage,
    session: &mut ModelSession,
    cancel: &CancellationToken,
    progress: &dyn Fn(usize, usize),
) -> Result<(RgbImage, Vec<serde_json::Value>)> {
    ensure!(
        image.dimensions() == mask.dimensions()
            && image.dimensions() == input_mask.dimensions()
            && image.dimensions() == output.dimensions(),
        "Mask dimensions do not match original"
    );
    let w = image.width() as usize;
    let h = image.height() as usize;
    let pixels = w
        .checked_mul(h)
        .filter(|n| *n <= 32_000_000)
        .ok_or_else(|| anyhow::anyhow!("Inpainting exceeds buffer budget"))?;
    let mut sums = vec![[0f32; 3]; pixels];
    let mut weights = vec![0f32; pixels];
    let mut times = Vec::new();
    let total = tiles
        .iter()
        .filter(|[x, y]| tile_touches(mask, *x, *y))
        .count();
    for &[ox, oy] in tiles {
        // Preserve all-inpainting input tensors and blending geometry. Only
        // skip a tile if it cannot contribute to any output-mask pixel.
        if !tile_touches(mask, ox, oy) {
            continue;
        }
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        let t = Instant::now();
        let mut pixels = vec![0u8; 512 * 512 * 3];
        let mut hole = vec![0u8; 512 * 512];
        for y in 0..512i32 {
            for x in 0..512i32 {
                let i = y as usize * 512 + x as usize;
                let px = reflect(ox + x, w as u32);
                let py = reflect(oy + y, h as u32);
                pixels[i * 3..i * 3 + 3].copy_from_slice(&image.get_pixel(px, py).0);
                // Reflect the mask together with the image. Leaving reflected text
                // unmasked leaks the answer into small-crop context.
                hole[i] = input_mask.get_pixel(px, py)[0];
            }
        }
        let (out, pre, infer) = session.infer(&pixels, &hole)?;
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        for y in 0..512i32 {
            for x in 0..512i32 {
                let i = y as usize * 512 + x as usize;
                if hole[i] == 0
                    || ox + x < 0
                    || oy + y < 0
                    || ox + x >= w as i32
                    || oy + y >= h as i32
                {
                    continue;
                }
                if mask.get_pixel((ox + x) as u32, (oy + y) as u32)[0] == 0 {
                    continue;
                }
                let idx = (oy + y) as usize * w + (ox + x) as usize;
                let weight = ((x + 1).min(512 - x).min(64) * (y + 1).min(512 - y).min(64)) as f32;
                weights[idx] += weight;
                for ch in 0..3 {
                    sums[idx][ch] += out[i * 3 + ch] as f32 * weight;
                }
            }
        }
        times.push(json!({"x":ox,"y":oy,"inference_readback_ms":infer,"tensor_ms":pre,"total_ms":t.elapsed().as_secs_f64()*1000.}));
        progress(times.len(), total);
    }
    for (x, y, m) in mask.enumerate_pixels() {
        if m[0] > 0 {
            let i = y as usize * w + x as usize;
            ensure!(weights[i] > 0., "Mask coverage gap");
            output.put_pixel(
                x,
                y,
                Rgb(sums[i].map(|v| (v / weights[i]).round().clamp(0., 255.) as u8)),
            );
        }
    }
    Ok((output, times))
}
