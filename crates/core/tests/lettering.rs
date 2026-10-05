use image::{DynamicImage, Rgb, RgbImage};
use std::path::PathBuf;
use tiny_skia::Pixmap;
use umanga_core::{render::Renderer, types::*};

fn renderer() -> Renderer {
    Renderer::new(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")).unwrap()
}

fn ink_bounds(p: &Pixmap, left: u32, right: u32) -> (u32, u32, u32, u32) {
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..p.height() {
        for x in left..right {
            if p.data()[((y * p.width() + x) * 4 + 3) as usize] > 0 {
                bounds = (
                    bounds.0.min(x),
                    bounds.1.min(y),
                    bounds.2.max(x),
                    bounds.3.max(y),
                );
            }
        }
    }
    assert_ne!(bounds.0, u32::MAX, "Expected visible lettering");
    bounds
}

#[test]
fn wide_auto_is_horizontal_and_tall_auto_is_vertical_with_manual_overrides() {
    let renderer = renderer();
    for text in ["魔界", "魔王城", "勇者", "魔王"] {
        for (bbox, expected) in [
            ([0., 0., 160., 80.], "horizontal"),
            ([0., 0., 80., 160.], "vertical"),
            ([0., 0., 120., 120.], "horizontal"),
            ([0., 0., 134., 143.], "vertical"),
            ([0., 0., 120.1, 120.2], "vertical"),
            ([0., 0., 120.2, 120.1], "horizontal"),
        ] {
            let mut region = Region {
                bbox,
                target: text.into(),
                ..Default::default()
            };
            let auto = renderer.lettering(&region, "zh-Hans").unwrap().unwrap();
            region.direction = expected.into();
            let explicit = renderer.lettering(&region, "zh-Hans").unwrap().unwrap();
            assert_eq!(auto, explicit, "{text} {expected}");
            region.direction = if expected == "vertical" {
                "horizontal"
            } else {
                "vertical"
            }
            .into();
            assert_ne!(
                auto,
                renderer.lettering(&region, "zh-Hans").unwrap().unwrap()
            );
        }
    }
    let mut english = Region {
        bbox: [0., 0., 90., 200.],
        target: "Hello".into(),
        ..Default::default()
    };
    let auto = renderer.lettering(&english, "en").unwrap().unwrap();
    english.direction = "horizontal".into();
    assert_eq!(auto, renderer.lettering(&english, "en").unwrap().unwrap());
}

#[test]
fn dialogue_columns_share_the_top_and_keep_right_to_left_order() {
    let renderer = renderer();
    let mut region = Region {
        bbox: [0., 0., 140., 260.],
        target: "你好世界，嗨。".into(),
        style: TextStyle {
            size: Some(28.),
            ..Default::default()
        },
        ..Default::default()
    };
    let punctuation = renderer.lettering(&region, "zh-Hans").unwrap().unwrap();
    assert_eq!(
        region.target, "你好世界，嗨。",
        "Formatting must not rewrite saved text"
    );
    region.target = "你好世界\n嗨".into();
    let manual = renderer.lettering(&region, "zh-Hans").unwrap().unwrap();
    assert_ne!(
        punctuation, manual,
        "Manual punctuation must survive rendering"
    );
    let left = ink_bounds(&manual, 0, 70);
    let right = ink_bounds(&manual, 70, 140);
    assert!(left.1.abs_diff(right.1) <= 1);
    assert!(right.1.abs_diff(manual.height() - 1 - right.3) <= 1);
    assert!(
        right.3 > left.3 + 60,
        "First (longer) column must be on the right"
    );
}

#[test]
fn horizontal_dialogue_and_free_captions_are_centered() {
    let renderer = renderer();
    let mut region = Region {
        bbox: [0., 0., 300., 180.],
        target: "Hello, world.".into(),
        direction: "horizontal".into(),
        style: TextStyle {
            size: Some(28.),
            ..Default::default()
        },
        ..Default::default()
    };
    let rendered = renderer.lettering(&region, "en").unwrap().unwrap();
    region.target = "Hello\nworld".into();
    assert_ne!(
        rendered,
        renderer.lettering(&region, "en").unwrap().unwrap()
    );
    let rendered = renderer.lettering(&region, "en").unwrap().unwrap();
    let dialogue = ink_bounds(&rendered, 0, rendered.width());
    assert!(dialogue.1.abs_diff(rendered.height() - 1 - dialogue.3) <= 1);
    region.kind = "free_text".into();
    let caption = renderer.lettering(&region, "en").unwrap().unwrap();
    assert_eq!(rendered, caption);
    region.bubble = Some([0., 0., 300., 180.]);
    assert_eq!(
        rendered,
        renderer.lettering(&region, "en").unwrap().unwrap()
    );
}

#[test]
fn generated_punctuation_only_translation_never_erases_the_original() {
    let renderer = renderer();
    let mut original = RgbImage::from_pixel(120, 120, Rgb([255, 255, 255]));
    for y in 35..85 {
        for x in 58..63 {
            original.put_pixel(x, y, Rgb([0, 0, 0]));
        }
    }
    let image = DynamicImage::ImageRgb8(original.clone());
    let mut regions = vec![Region {
        bbox: [25., 25., 95., 95.],
        target: "，。".into(),
        allow_fill: true,
        ..Default::default()
    }];
    regions[0].target = umanga_core::render::format_translation(&regions[0], &regions[0].target);
    assert!(
        renderer
            .lettering(&regions[0], "zh-Hans")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        renderer
            .render_benchmark(&image, &mut regions, "zh-Hans", None)
            .unwrap()
            .to_rgb8(),
        original
    );
    assert_eq!(regions[0].target, "");
}
