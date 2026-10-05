use image::{DynamicImage, Rgb, RgbImage};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;
use umanga_core::{
    documents, editing, pipeline::Engine, render::Renderer, safety, store, types::*,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn renderer() -> Renderer {
    Renderer::new(&root().join("assets/fonts")).unwrap()
}
fn region(text: &str, size: f32) -> Region {
    Region {
        target: text.into(),
        direction: "horizontal".into(),
        bbox: [0., 0., 320., 180.],
        kind: "free".into(),
        style: TextStyle {
            size: Some(size),
            color: "#111111".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn bounds(p: &tiny_skia::Pixmap) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..p.height() {
        for x in 0..p.width() {
            if p.data()[((y * p.width() + x) * 4 + 3) as usize] > 0 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    assert_ne!(b[0], u32::MAX);
    b
}

#[test]
fn detected_defaults_and_manual_defaults_are_distinct() {
    for (kind, bubble, enabled) in [
        ("free", false, true),
        ("free_text", false, true),
        ("free", true, false),
        ("text", true, false),
        ("text", false, false),
        ("manual", false, false),
    ] {
        let style = TextStyle::detected(kind, bubble);
        assert_eq!(style.outline_enabled, enabled, "{kind} {bubble}");
        assert_eq!(style.outline_width_percent, 8.);
        assert_eq!(style.outline_color, "#ffffff");
    }
    assert!(!Region::default().style.outline_enabled);
}

#[test]
fn disabled_pixels_match_reviewed_centered_samples() {
    let samples: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root().join("crates/core/tests/fixtures/outline-disabled.json")).unwrap(),
    )
    .unwrap();
    let renderer = renderer();
    for sample in samples {
        let mut r = region(sample["text"].as_str().unwrap(), 28.);
        r.bbox = [
            0.,
            0.,
            sample["width"].as_f64().unwrap() as f32,
            sample["height"].as_f64().unwrap() as f32,
        ];
        r.direction = sample["direction"].as_str().unwrap().into();
        r.style.size = sample["size"].as_f64().map(|v| v as f32);
        let p = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        assert_eq!(
            store::digest(p.data()),
            sample["rgbaSha256"].as_str().unwrap(),
            "{}",
            sample["name"]
        );
        r.style.outline_width_percent = 20.;
        r.style.outline_color = "#ff0033".into();
        assert_eq!(
            p,
            renderer.lettering(&r, "zh-Hans").unwrap().unwrap(),
            "Disabled values must not affect pixels"
        );
    }
}

#[test]
fn strokes_extend_outward_proportionally_and_never_cover_foreground() {
    let renderer = renderer();
    for (size, percent) in [(20., 10.), (40., 10.), (20., 20.)] {
        let mut r = region("田田AVAVA", size);
        let plain = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        r.style.outline_enabled = true;
        r.style.outline_width_percent = percent;
        r.style.outline_color = "#e04080".into();
        let outlined = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        let a = bounds(&plain);
        let b = bounds(&outlined);
        let extension = size * percent / 100.;
        let width_growth = (b[2] - b[0]) as f32 - (a[2] - a[0]) as f32;
        assert!(
            (width_growth - 2. * extension).abs() <= 2.,
            "{size}/{percent}: {width_growth}"
        );
        for y in 0..plain.height() {
            for x in 0..plain.width() {
                let old = &plain.data()[((y * plain.width() + x) * 4) as usize..][..4];
                if old[3] == 255 {
                    let new = &outlined.data()[((y * outlined.width() + x) * 4) as usize..][..4];
                    assert_eq!(new, old, "All glyph foregrounds must survive every outline");
                }
            }
        }
        assert!(
            outlined
                .data()
                .chunks_exact(4)
                .any(|p| p == [224, 64, 128, 255])
        );
    }
}

#[test]
fn outlined_mixed_scripts_and_vertical_punctuation_stay_inside_every_edge() {
    let renderer = renderer();
    for (text, direction) in [
        ("天地ABC…——終！", "vertical"),
        ("中文 العربية हिन्दी A\u{301}", "horizontal"),
        ("田田田田田田田田田田田田", "horizontal"),
    ] {
        let mut r = region(text, 28.);
        r.direction = direction.into();
        r.style.size = None;
        r.style.outline_enabled = true;
        r.style.outline_width_percent = 20.;
        r.bbox = [0., 0., 180., 220.];
        let p = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
        let b = bounds(&p);
        assert!(
            b[0] > 0 && b[1] > 0 && b[2] + 1 < p.width() && b[3] + 1 < p.height(),
            "{text}: {b:?}"
        );
        assert!(
            b[0].abs_diff(p.width() - 1 - b[2]) <= 1,
            "Centered horizontally: {b:?}"
        );
        assert!(
            b[1].abs_diff(p.height() - 1 - b[3]) <= 1,
            "Centered vertically: {b:?}"
        );
    }
    let mut r = region("国", 24.);
    r.bbox = [0., 0., 36., 40.];
    assert!(renderer.lettering(&r, "zh-Hans").is_ok());
    r.style.outline_enabled = true;
    r.style.outline_width_percent = 20.;
    assert!(
        renderer
            .lettering(&r, "zh-Hans")
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    r.style.size = None;
    assert!(renderer.lettering(&r, "zh-Hans").unwrap().is_some());
}

#[test]
fn outline_validation_and_independent_field_merging() {
    let mut r = region("国", 24.);
    for width in [f32::NAN, f32::INFINITY, 0., 20.1] {
        r.style.outline_width_percent = width;
        assert!(safety::region(&r).is_err());
    }
    r.style.outline_width_percent = 8.;
    for color in ["#fff", "ffffff", "#gg0000", "#ffffff00"] {
        r.style.outline_color = color.into();
        assert!(safety::region(&r).is_err());
    }
    r.style.outline_color = "#Ab09fF".into();
    assert!(safety::region(&r).is_ok());
    let folder = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let source = folder.path().join("original.png");
    RgbImage::new(320, 180).save(&source).unwrap();
    let mut base = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    base.regions = vec![r];
    let mut draft = base.clone();
    draft.regions[0].style.outline_enabled = true;
    draft.regions[0].style.outline_width_percent = 15.;
    let mut latest = base.clone();
    latest.regions[0].target = "concurrent translation".into();
    latest.regions[0].style.outline_color = "#0044ff".into();
    latest.regions[0].style.color = "#118822".into();
    let merged = editing::merge(&base, &draft, &latest).unwrap();
    let saved = &merged.regions[0];
    assert_eq!(saved.target, "concurrent translation");
    assert_eq!(saved.style.color, "#118822");
    assert_eq!(saved.style.outline_color, "#0044ff");
    assert!(saved.style.outline_enabled);
    assert_eq!(saved.style.outline_width_percent, 15.);
    let path = folder.path().join("book.umanga");
    store::create(&path, "Outline", std::slice::from_ref(&merged)).unwrap();
    assert_eq!(
        serde_json::to_value(store::page(&path, &merged.id).unwrap()).unwrap(),
        serde_json::to_value(&merged).unwrap()
    );
    {
        let db = store::connection(&path).unwrap();
        db.execute("UPDATE meta SET value='12' WHERE key='version'", [])
            .unwrap();
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    }
    let old_hash = store::file_hash(&path).unwrap();
    assert!(
        store::open(&path)
            .unwrap_err()
            .to_string()
            .contains(&format!("requires format {}", store::FORMAT_VERSION))
    );
    assert_eq!(store::file_hash(&path).unwrap(), old_hash);
    let mut malformed = serde_json::to_value(&merged.regions[0]).unwrap();
    malformed["style"]["outlineEnabled"] = json!("yes");
    assert!(serde_json::from_value::<Region>(malformed).is_err());
}

#[test]
fn production_foreground_is_authoritative_and_outline_edits_reuse_cleanup() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let engine = Engine::new(
        dir.path().join("no-models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Typography must not call a provider")),
    )
    .unwrap();
    let mut report = vec![];
    for (index, (rgb, color)) in [
        ([0, 0, 0], "#000000"),
        ([255, 255, 255], "#ffffff"),
        ([50, 120, 180], "#3278b4"),
    ]
    .into_iter()
    .enumerate()
    {
        for mode in ["solid", "neural-plain", "imported", "overlay"] {
            let source = dir.path().join(format!("{index}-{mode}.png"));
            let original = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 180, Rgb(rgb)));
            original.save(&source).unwrap();
            let mut p = documents::import(&[source.to_string_lossy().into()])
                .unwrap()
                .remove(0);
            let mut r = region("TEST 国", 28.);
            r.style.color = color.into();
            r.bbox = [20., 20., 300., 160.];
            r.overlay_only = mode == "overlay";
            r.bubble = Some(r.bbox);
            p.regions = vec![r];
            if mode == "imported" {
                p.background = Some(source.to_string_lossy().into());
            }
            let book = dir.path().join(format!("{index}-{mode}.umanga"));
            store::create(&book, "Typography", std::slice::from_ref(&p)).unwrap();
            let mut settings = TranslationSettings::default();
            settings.cleanup.method = if mode == "neural-plain" {
                CleanupMethod::MangaLama
            } else {
                CleanupMethod::Solid
            };
            let before_hash = store::file_hash(&source).unwrap();
            let render = |page: &mut Page| {
                engine
                    .render_page(
                        &original,
                        page,
                        &settings,
                        &book,
                        dir.path(),
                        false,
                        &CancellationToken::new(),
                        Arc::new(|_, _, _| {}),
                    )
                    .unwrap()
            };
            let plain = render(&mut p);
            assert!(
                p.regions[0].review.is_none(),
                "{mode}: {:?}",
                p.regions[0].review
            );
            // Premultiplied glyph channels and integer background compositing can
            // differ by one at antialiased edges even when both colors match.
            let max_delta = plain
                .to_rgb8()
                .as_raw()
                .iter()
                .zip(original.to_rgb8().as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                max_delta <= 1,
                "Foreground must not be substituted: {mode}, max channel delta {max_delta}"
            );
            let cleanup = serde_json::to_value(p.cleanup.as_ref().unwrap()).unwrap();
            p.regions[0].style.outline_enabled = true;
            p.regions[0].style.outline_color =
                if index == 1 { "#000000" } else { "#ffffff" }.into();
            let outlined = render(&mut p);
            assert!(p.regions[0].review.is_none());
            assert!(outlined.to_rgb8() != plain.to_rgb8());
            assert_eq!(
                serde_json::to_value(p.cleanup.as_ref().unwrap()).unwrap(),
                cleanup,
                "Outline-only edit reuses cleaned background"
            );
            assert_eq!(store::file_hash(&source).unwrap(), before_hash);
            let output = root().join("test-output/region-text-outlines/production");
            std::fs::create_dir_all(&output).unwrap();
            outlined
                .save(output.join(format!("{index}-{mode}.png")))
                .unwrap();
            report.push(json!({"case":format!("{index}-{mode}"),"foreground":color,"cleanupReused":true,"originalUnchanged":true}));
        }
    }
    std::fs::write(
        root().join("test-output/region-text-outlines/production/report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn production_mixed_artwork_export_and_overflow_preserve_originals() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let engine = Engine::new(
        dir.path().join("no-models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("No credentials for lettering")),
    )
    .unwrap();
    let original = DynamicImage::ImageRgb8(RgbImage::from_fn(600, 360, |x, y| {
        if x > 380 && x < 580 && y > 10 && y < 350 {
            Rgb([255, 255, 255])
        } else if (x + y) % 5 < 2 {
            Rgb([45, 45, 45])
        } else {
            Rgb([185, 185, 185])
        }
    }));
    let source = dir.path().join("original.png");
    original.save(&source).unwrap();
    let original_hash = store::file_hash(&source).unwrap();
    let mut p = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    p.regions = [
        ("free", None, [20., 20., 180., 330.], "在所有人的优越感之中"),
        ("text", None, [210., 20., 360., 330.], "对白没有匹配到气泡"),
        (
            "text",
            Some([380., 10., 580., 350.]),
            [390., 20., 570., 330.],
            "气泡中的对白保持没有描边",
        ),
    ]
    .into_iter()
    .map(|(kind, bubble, bbox, text)| Region {
        kind: kind.into(),
        bubble,
        bbox,
        target: text.into(),
        overlay_only: true,
        style: TextStyle {
            color: "#000000".into(),
            ..TextStyle::detected(kind, bubble.is_some())
        },
        ..Default::default()
    })
    .collect();
    let book = dir.path().join("sample.umanga");
    store::create(&book, "Outlines", std::slice::from_ref(&p)).unwrap();
    let settings = TranslationSettings::default();
    let render = |p: &mut Page| {
        engine
            .render_page(
                &original,
                p,
                &settings,
                &book,
                dir.path(),
                false,
                &CancellationToken::new(),
                Arc::new(|_, _, _| {}),
            )
            .unwrap()
    };
    let text_before: Vec<_> = p.regions.iter().map(|r| r.target.clone()).collect();
    let rendered = render(&mut p);
    assert!(p.regions.iter().all(|r| r.review.is_none()));
    assert_eq!(
        text_before,
        p.regions
            .iter()
            .map(|r| r.target.clone())
            .collect::<Vec<_>>()
    );
    let output = root().join("test-output/region-text-outlines/production");
    std::fs::create_dir_all(&output).unwrap();
    original.save(output.join("mixed-original.png")).unwrap();
    rendered.save(output.join("mixed-outlined.png")).unwrap();
    let rendered_path = dir.path().join("rendered.png");
    rendered.save(&rendered_path).unwrap();
    p.rendered = Some(rendered_path.to_string_lossy().into());
    let expected = p.revision;
    assert!(store::save_page(&book, &mut p, expected).unwrap());
    let reopened = store::open(&book).unwrap();
    assert!(reopened.pages[0].regions[0].style.outline_enabled);
    let export = dir.path().join("export");
    documents::export(&reopened, &export, "images").unwrap();
    assert_eq!(
        store::file_hash(&rendered_path).unwrap(),
        store::file_hash(&export.join("0001.png")).unwrap()
    );
    // Explicit oversized glyphs must preserve the affected area rather than clip.
    p.regions[0].bbox = [20., 20., 55., 60.];
    p.regions[0].style.size = Some(120.);
    p.regions[0].style.outline_width_percent = 20.;
    p.regions[0].overlay_only = false;
    let failed = render(&mut p).to_rgb8();
    assert!(p.regions[0].review.as_ref().unwrap().contains("overflow"));
    let rgb = original.to_rgb8();
    for y in 20..60 {
        for x in 20..55 {
            assert_eq!(failed.get_pixel(x, y), rgb.get_pixel(x, y));
        }
    }
    assert_eq!(store::file_hash(&source).unwrap(), original_hash);
}
