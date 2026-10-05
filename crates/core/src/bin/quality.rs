use anyhow::Result;
use image::{DynamicImage, Rgb, RgbImage};
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use umanga_core::{
    inference::{self, Inference},
    render::Renderer,
    store,
    types::*,
};
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join("test-output/quality");
    std::fs::create_dir_all(&output)?;
    inference::init(&root.join("assets/runtime"))?;
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut infer = Inference::new(root.join("assets/models"));
    let mut results = vec![];
    let pages: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("test-output/native/report.json"))?)?;
    for page in pages {
        let name = page["page"].as_str().unwrap();
        let original = image::open(
            root.parent()
                .unwrap()
                .join(format!("sample_raw_manga/sample_page_styles/{name}.jpg")),
        )?
        .to_rgb8();
        let rendered = image::open(root.join(format!("test-output/native/{name}.png")))?.to_rgb8();
        original.save(root.join(format!("test-output/native/{name}-original.png")))?;
        let regions: Vec<Region> = serde_json::from_value(page["regions"].clone())?;
        let mut outside = 0usize;
        for (x, y, pixel) in original.enumerate_pixels() {
            let allowed = regions.iter().any(|r| {
                r.review.is_none()
                    && x as f32 >= r.bbox[0] - 10.
                    && x as f32 <= r.bbox[2] + 10.
                    && y as f32 >= r.bbox[1] - 10.
                    && y as f32 <= r.bbox[3] + 10.
            });
            if !allowed && pixel != rendered.get_pixel(x, y) {
                outside += 1;
            }
        }
        assert_eq!(outside, 0, "Unexpected artwork changes on {name}");
    }
    results.push(
        json!({"name":"Artwork preservation","pages":19,"outsideApprovedRegionsChangedPixels":0}),
    );
    for (name, lang, text, vertical) in [
        ("latin", "en", "The moon is beautiful tonight.", false),
        ("japanese", "ja", "今日はとてもいい天気ですね。", false),
        ("chinese", "zh", "今天的月亮真漂亮。", false),
        ("vertical", "zh", "「你好！」今天是第12话……OpenAI。", true),
        ("arabic", "ar", "مرحبا بالعالم 123 — Hello", false),
        ("indic", "hi", "नमस्ते दुनिया — Hello 123", false),
        ("mixed", "zh", "你好 Hello مرحبا नमस्ते", false),
    ] {
        let (w, h) = if vertical { (320, 900) } else { (1000, 140) };
        let background = DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([255, 255, 255])));
        let mut regions = vec![Region {
            bbox: [12., 12., w as f32 - 12., h as f32 - 12.],
            target: text.into(),
            direction: if vertical { "vertical" } else { "horizontal" }.into(),
            overlay_only: true,
            style: TextStyle {
                size: Some(40.),
                ..Default::default()
            },
            ..Default::default()
        }];
        let rendered = renderer.render_benchmark(&background, &mut regions, lang, None)?;
        assert!(
            regions[0].review.is_none(),
            "{name}: {:?}",
            regions[0].review
        );
        assert_ne!(rendered.to_rgb8(), background.to_rgb8());
        assert_eq!(
            rendered.to_rgb8(),
            renderer
                .render_benchmark(&background, &mut regions, lang, None)?
                .to_rgb8()
        );
        rendered.save(output.join(format!("{name}.png")))?;
        let start = Instant::now();
        let ocr = if ["en", "ja", "zh"].contains(&lang) && !vertical && name != "mixed" {
            Some(infer.pp(&rendered, lang)?)
        } else {
            None
        };
        let row = json!({"name":name,"language":lang,"expected":text,"ocr":ocr,"milliseconds":start.elapsed().as_millis(),"exact":ocr.as_deref()==Some(text)});
        println!("{row}");
        results.push(row);
    }
    if std::env::args().any(|a| a == "--directml") {
        let page = image::open(
            root.parent()
                .unwrap()
                .join("sample_raw_manga/sample_page_styles/0005.jpg"),
        )?;
        let regions = infer.detect(&page)?;
        let crops = regions
            .iter()
            .take(1)
            .map(|r| inference::crop(&page, r.bbox))
            .collect::<Vec<_>>();
        let settings = TranslationSettings {
            device: "directml".into(),
            ..Default::default()
        };
        let start = Instant::now();
        let text = infer.recognize(&crops, &settings)?;
        let row = json!({"name":"DirectML validation","result":text,"device":infer.device_message,"milliseconds":start.elapsed().as_millis()});
        println!("{row}");
        results.push(row);
    }
    store::atomic_write(
        &output.join("report.json"),
        &serde_json::to_vec_pretty(&results)?,
    )?;
    Ok(())
}
