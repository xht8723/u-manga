use std::path::PathBuf;
use tiny_skia::Pixmap;
use umanga_core::{render::Renderer, types::*};

fn renderer() -> Renderer {
    Renderer::new(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")).unwrap()
}

fn sample(text: &str, direction: &str, width: f32, height: f32) -> Region {
    Region {
        target: text.into(),
        direction: direction.into(),
        bbox: [0., 0., width, height],
        style: TextStyle {
            size: Some(28.),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn bounds(p: &Pixmap, left: u32, right: u32) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..p.height() {
        for x in left..right {
            if p.data()[((y * p.width() + x) * 4 + 3) as usize] > 0 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    assert_ne!(b[0], u32::MAX);
    b
}

fn centered(p: &Pixmap) {
    let b = bounds(p, 0, p.width());
    assert!(
        (i64::from(b[0]) - i64::from(p.width() - 1 - b[2])).abs() <= 1,
        "Horizontal margins: {b:?}"
    );
    assert!(
        (i64::from(b[1]) - i64::from(p.height() - 1 - b[3])).abs() <= 1,
        "Vertical margins: {b:?}"
    );
    assert!(b[0] > 0 && b[1] > 0 && b[2] + 1 < p.width() && b[3] + 1 < p.height());
}

#[test]
fn unequal_columns_keep_common_top_and_center_as_one_block() {
    let renderer = renderer();
    let r = sample("田田田田\n田", "vertical", 140., 260.);
    let p = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
    centered(&p);
    let left = bounds(&p, 0, 70);
    let right = bounds(&p, 70, 140);
    assert!(left[1].abs_diff(right[1]) <= 1);
    assert!(right[3] > left[3] + 60);
}

#[test]
fn all_region_kinds_share_placement_independently_of_bubble_position() {
    let renderer = renderer();
    for direction in ["vertical", "horizontal"] {
        let mut r = sample("田田\n田", direction, 180., 260.);
        let saved = r.target.clone();
        let dialogue = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        centered(&dialogue);
        for kind in ["text", "free", "free_text", "manual"] {
            for bubble in [None, Some([-10., -20., 240., 300.])] {
                r.kind = kind.into();
                r.bubble = bubble;
                let p = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
                centered(&p);
                assert_eq!(dialogue, p, "{kind}/{bubble:?}/{direction}");
            }
        }
        assert_eq!(r.target, saved);
        assert_eq!(r.bbox, [0., 0., 180., 260.]);
    }
}

#[test]
fn center_varied_scripts_outlines_and_fitting_without_clipping() {
    let renderer = renderer();
    for (text, direction, width, height) in [
        ("田", "vertical", 90., 240.),
        ("Hello 世界 A\u{301}", "horizontal", 320., 180.),
        ("中文 العربية हिन्दी\n田", "horizontal", 420., 210.),
        ("田田\n田", "auto", 160.1, 160.2),
        ("田田\n田", "auto", 160.2, 160.1),
        ("田田\n\n田", "horizontal", 280., 220.),
        ("田田\n\n田", "vertical", 240., 260.),
        ("天地ABC……——終！", "vertical", 220., 380.),
        (
            "这是很长的对白需要自动换行保持所有列的顶部对齐",
            "vertical",
            160.,
            190.,
        ),
    ] {
        for kind in ["text", "free"] {
            for percent in [None, Some(1.), Some(8.), Some(20.)] {
                let mut r = sample(text, direction, width, height);
                r.kind = kind.into();
                r.style.outline_enabled = percent.is_some();
                r.style.outline_width_percent = percent.unwrap_or(8.);
                for size in [Some(28.), None] {
                    r.style.size = size;
                    match renderer.lettering(&r, "zh-Hans") {
                        Ok(Some(p)) => centered(&p),
                        Err(e) if size.is_some() => assert!(e.to_string().contains("overflow")),
                        other => panic!("{text}: {other:?}"),
                    }
                }
            }
        }
    }
    let mut r = sample("田田田田", "vertical", 24., 24.);
    assert!(
        renderer
            .lettering(&r, "zh-Hans")
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    r.target = "，。".into();
    r.bbox = [0., 0., 100., 140.];
    centered(&renderer.lettering(&r, "zh-Hans").unwrap().unwrap());
}

#[test]
fn internal_blank_lines_and_columns_keep_their_spacing() {
    let renderer = renderer();
    for kind in ["text", "free", "free_text", "manual"] {
        for direction in ["vertical", "horizontal"] {
            let mut r = sample("田田\n田", direction, 220., 260.);
            r.kind = kind.into();
            let a = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
            r.target = "田田\n\n田".into();
            let b = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
            centered(&a);
            centered(&b);
            let a = bounds(&a, 0, a.width());
            let b = bounds(&b, 0, b.width());
            let axis = if direction == "vertical" { 0 } else { 1 };
            let growth = (b[axis + 2] - b[axis]) as f32 - (a[axis + 2] - a[axis]) as f32;
            let pitch = 28. * (1. + r.style.line_gap);
            // Comparing two rasterized spans accumulates up to one pixel of edge
            // rounding from each image; each image's centering still allows only one.
            assert!(
                (growth - pitch).abs() <= 2.,
                "{direction}: {a:?} -> {b:?}, growth {growth}, pitch {pitch}"
            );
        }
    }
}

#[test]
fn capture_native_samples_when_requested() {
    let Some(output) = std::env::var_os("U_MANGA_CENTERING_SAMPLES") else {
        return;
    };
    let output = PathBuf::from(output);
    std::fs::create_dir_all(&output).unwrap();
    let renderer = renderer();
    let mut report = vec![];
    for (name, text, direction, width, height, free, outline) in [
        (
            "unequal-columns",
            "你好世界\n嗨",
            "vertical",
            140.,
            260.,
            false,
            false,
        ),
        (
            "short-dialogue",
            "你好",
            "vertical",
            100.,
            300.,
            false,
            false,
        ),
        (
            "horizontal",
            "Hello\nworld",
            "horizontal",
            300.,
            180.,
            false,
            false,
        ),
        (
            "outlined",
            "天地ABC……\n田",
            "vertical",
            180.,
            300.,
            false,
            true,
        ),
        (
            "caption",
            "拥有外交\n豁免权",
            "vertical",
            140.,
            260.,
            true,
            true,
        ),
        (
            "horizontal-caption",
            "Hello\nworld",
            "horizontal",
            300.,
            180.,
            true,
            false,
        ),
        (
            "free-text",
            "你好世界\n嗨",
            "vertical",
            140.,
            260.,
            true,
            false,
        ),
    ] {
        let mut r = sample(text, direction, width, height);
        if free {
            r.kind = "free".into();
        }
        r.style.outline_enabled = outline;
        let p = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        p.save_png(output.join(format!("{name}.png"))).unwrap();
        report.push(serde_json::json!({"name": name, "region": r, "inkBounds": bounds(&p, 0, p.width()), "rgbaSha256": umanga_core::store::digest(p.data())}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
