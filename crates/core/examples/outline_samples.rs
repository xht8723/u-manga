//! Isolated native glyph samples; baseline captures disabled pixels before edits.
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use umanga_core::{render::Renderer, store, types::*};

fn main() -> Result<()> {
    let output = PathBuf::from(std::env::args().nth(1).expect("Output directory required"));
    let outlined = std::env::args().nth(2).as_deref() == Some("outlined");
    std::fs::create_dir_all(&output)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut reports = vec![];
    for (name, text, direction, width, height, size) in [
        (
            "horizontal",
            "Hello 世界 café A\u{301}",
            "horizontal",
            320,
            140,
            Some(28.),
        ),
        (
            "vertical",
            "天地ABC…——終！",
            "vertical",
            180,
            380,
            Some(28.),
        ),
        (
            "fallback",
            "中文 العربية हिन्दी",
            "horizontal",
            420,
            180,
            Some(28.),
        ),
        (
            "fitted",
            "这是很长的一段对白需要自动换行并保持顶部对齐",
            "horizontal",
            180,
            150,
            None,
        ),
        ("small", "ABC天地", "horizontal", 220, 100, Some(12.)),
        ("large", "ABC天地", "horizontal", 300, 140, Some(48.)),
    ] {
        let mut region = Region {
            id: name.into(),
            target: text.into(),
            direction: direction.into(),
            bbox: [0., 0., width as f32, height as f32],
            kind: "free".into(),
            overlay_only: true,
            style: TextStyle {
                color: "#111111".into(),
                size,
                ..Default::default()
            },
            ..Default::default()
        };
        if outlined {
            let mut value = serde_json::to_value(&region)?;
            value["style"]["outlineEnabled"] = json!(true);
            value["style"]["outlineWidthPercent"] = json!(8.);
            value["style"]["outlineColor"] = json!("#ffffff");
            region = serde_json::from_value(value)?;
        }
        let pix = renderer
            .lettering(&region, "zh-Hans")?
            .expect("Visible lettering");
        let mut background = image::RgbImage::from_fn(width, height, |x, y| {
            if (x / 28 + y / 28) % 2 == 0 {
                image::Rgb([60, 70, 80])
            } else {
                image::Rgb([210, 200, 180])
            }
        });
        for (i, pixel) in background.pixels_mut().enumerate() {
            let rgba = &pix.data()[i * 4..i * 4 + 4];
            for channel in 0..3 {
                pixel[channel] = (u32::from(rgba[channel])
                    + u32::from(pixel[channel]) * (255 - u32::from(rgba[3])) / 255)
                    .min(255) as u8;
            }
        }
        background.save(output.join(format!("{name}.png")))?;
        reports.push(json!({"name":name,"text":text,"direction":direction,"width":width,"height":height,"size":size,"rgbaSha256":store::digest(pix.data())}));
    }
    std::fs::write(
        output.join("samples.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!(
        "Saved {} native samples to {}",
        reports.len(),
        output.display()
    );
    Ok(())
}
