//! Offline visual fixtures for the reported mixed-script and punctuation cases.
use anyhow::Result;
use image::{Rgb, RgbImage};
use umanga_core::{render::Renderer, types::*};
fn main() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("test-output/lettering-thinking-navigation");
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut sheet = RgbImage::from_pixel(1040, 540, Rgb([235, 237, 236]));
    let cases = [
        (
            "enum",
            "按照 A、B、C、D、E 的五级分级，将怪物运送到与其等级相符的地点。",
            "vertical",
            180,
            450,
            None,
        ),
        (
            "latin",
            "根据调查结果，女性将 A 列为最高位，E 列为最低位。",
            "vertical",
            180,
            450,
            None,
        ),
        ("ellipsis", "那真是……咕咕……", "vertical", 180, 450, None),
        (
            "dash",
            "等——一下……É e\u{301} 12",
            "vertical",
            180,
            450,
            Some("Arial"),
        ),
        (
            "horizontal",
            "A、B、C……等——一下",
            "horizontal",
            180,
            450,
            None,
        ),
    ];
    for (i, (name, text, direction, width, height, font)) in cases.iter().enumerate() {
        let region = Region {
            bbox: [0., 0., *width as f32, *height as f32],
            direction: (*direction).into(),
            target: (*text).into(),
            style: TextStyle {
                size: Some(25.),
                font: font.map(str::to_owned),
                ..Default::default()
            },
            ..Default::default()
        };
        let letters = renderer.lettering(&region, "zh-Hans")?.unwrap();
        letters.save_png(out.join(format!("{name}.png")))?;
        for y in 0..*height {
            for x in 0..*width {
                let offset = ((y * width + x) * 4) as usize;
                let p = &letters.data()[offset..offset + 4];
                sheet.put_pixel(
                    20 + i as u32 * 205 + x,
                    20 + y,
                    Rgb([
                        p[0].saturating_add(255 - p[3]),
                        p[1].saturating_add(255 - p[3]),
                        p[2].saturating_add(255 - p[3]),
                    ]),
                );
            }
        }
    }
    sheet.save(out.join("lettering-sheet.png"))?;
    println!("Saved native lettering examples to {}", out.display());
    Ok(())
}
