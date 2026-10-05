use image::{DynamicImage, GenericImageView, Rgb, RgbImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_hollow_rect_mut},
    rect::Rect,
};
use umanga_core::{cleanup, render::Renderer, types::*};

fn fixture(fill: [u8; 3], ink: [u8; 3]) -> (RgbImage, Region) {
    let mut im = RgbImage::from_pixel(220, 240, Rgb([150, 150, 150]));
    draw_filled_rect_mut(&mut im, Rect::at(40, 35).of_size(140, 175), Rgb(fill));
    draw_hollow_rect_mut(&mut im, Rect::at(40, 35).of_size(140, 175), Rgb(ink));
    for y in [75, 105, 135, 165] {
        for x in [80, 110, 135] {
            draw_filled_rect_mut(&mut im, Rect::at(x, y).of_size(3, 12), Rgb(ink));
            draw_filled_rect_mut(&mut im, Rect::at(x, y + 5).of_size(10, 3), Rgb(ink));
        }
    }
    (
        im,
        Region {
            id: "test".into(),
            bbox: [76., 71., 148., 181.],
            bubble: Some([40., 35., 180., 210.]),
            source: "原文".into(),
            target: "你好。".into(),
            ..Default::default()
        },
    )
}
#[test]
fn masks_dark_light_colored_ink_and_preserves_boundaries() {
    for (fill, ink) in [
        ([247, 243, 231], [20, 20, 20]),
        ([18, 25, 35], [238, 243, 249]),
        ([253, 245, 226], [192, 35, 65]),
    ] {
        let (im, r) = fixture(fill, ink);
        let before = im.clone();
        let result = cleanup::analyze(&im, &r);
        assert_eq!(result.fill, fill);
        assert!(result.mask.as_raw().iter().filter(|&&v| v > 0).count() > 300);
        for (x, y, p) in result.mask.enumerate_pixels() {
            if p[0] > 0 {
                assert!(
                    x + result.origin[0] > 40
                        && x + result.origin[0] < 179
                        && y + result.origin[1] > 35
                        && y + result.origin[1] < 209
                );
                assert!(result.interior.as_ref().unwrap().get_pixel(x, y)[0] > 0);
            }
        }
        assert_eq!(before, im);
    }
}
#[test]
fn recovers_missing_bubble_and_tolerates_grain_and_a_contaminated_text_margin() {
    let (mut im, mut r) = fixture([244, 240, 233], [15, 15, 15]);
    r.bubble = None;
    // Widen the recovery search enough to include this balloon and its outline.
    r.bbox = [62., 60., 159., 190.];
    for y in 37..208 {
        for x in 42..178 {
            let p = im.get_pixel_mut(x, y);
            if p[0] > 200 {
                let d = ((x * 17 + y * 23) % 13) as u8;
                for c in &mut p.0 {
                    *c = c.saturating_sub(d);
                }
            }
        }
    }
    assert!(cleanup::analyze(&im, &r).interior.is_some());
    r.bbox = [44., 60., 159., 190.]; // The background strip would sample the outline.
    r.bubble = Some([40., 35., 180., 210.]);
    assert!(cleanup::analyze(&im, &r).interior.is_some());
}
#[test]
fn open_background_artwork_and_screentones_use_bounded_fallback_masks() {
    let (mut im, r) = fixture([255, 255, 255], [0, 0, 0]);
    draw_filled_rect_mut(
        &mut im,
        Rect::at(95, 34).of_size(30, 6),
        Rgb([255, 255, 255]),
    );
    // The opening continues into the outside background.
    draw_filled_rect_mut(
        &mut im,
        Rect::at(95, 0).of_size(30, 36),
        Rgb([255, 255, 255]),
    );
    assert_fallback(&im, &r);
    let (mut textured, r) = fixture([255, 255, 255], [0, 0, 0]);
    for y in 40..204 {
        for x in 44..175 {
            if (x / 3 + y / 3) % 2 == 0 {
                textured.put_pixel(x, y, Rgb([40, 40, 40]));
            }
        }
    }
    assert_fallback(&textured, &r);
    let (mut dotted, r) = fixture([255, 255, 255], [0, 0, 0]);
    for y in (45..200).step_by(12) {
        for x in (48..174).step_by(12) {
            draw_filled_rect_mut(&mut dotted, Rect::at(x, y).of_size(3, 3), Rgb([0, 0, 0]));
        }
    }
    assert_fallback(&dotted, &r);
}

fn assert_fallback(im: &RgbImage, r: &Region) {
    let result = cleanup::analyze(im, r);
    assert!(result.interior.is_none());
    assert!(result.mask.as_raw().iter().any(|&v| v > 0));
    for (x, y, p) in result.mask.enumerate_pixels() {
        if p[0] > 0 {
            let (x, y) = ((x + result.origin[0]) as f32, (y + result.origin[1]) as f32);
            assert!(x >= r.bbox[0] - 3. && x < r.bbox[2] + 3.);
            assert!(y >= r.bbox[1] - 3. && y < r.bbox[3] + 3.);
        }
    }
    assert!(!r.allow_fill);
}

#[test]
fn connected_balloon_can_end_at_a_physical_page_edge() {
    let (mut im, mut r) = fixture([255, 255, 255], [0, 0, 0]);
    draw_filled_rect_mut(
        &mut im,
        Rect::at(41, 0).of_size(138, 74),
        Rgb([255, 255, 255]),
    );
    for y in 0..75 {
        im.put_pixel(40, y, Rgb([0, 0, 0]));
        im.put_pixel(179, y, Rgb([0, 0, 0]));
    }
    r.bubble = Some([40., 0., 180., 210.]);
    assert!(cleanup::analyze(&im, &r).interior.is_some());
    r.bubble = Some([40., 55., 180., 210.]);
    assert!(
        cleanup::analyze_with_bubbles(&im, &r, &[[40., 0., 180., 100.]])
            .interior
            .is_some()
    );
    r.bubble = None;
    assert_fallback(&im, &r);
}
#[test]
fn touching_text_uses_fallback_but_unrelated_border_components_are_excluded() {
    let (mut im, mut r) = fixture([255, 255, 255], [0, 0, 0]);
    r.bbox = [41., 60., 151., 185.];
    assert!(cleanup::analyze(&im, &r).interior.is_some());
    draw_filled_rect_mut(&mut im, Rect::at(40, 110).of_size(82, 3), Rgb([0, 0, 0]));
    assert_fallback(&im, &r);
}
#[test]
fn solid_override_uses_color_and_region_bounds_without_brushes() {
    let (im, mut r) = fixture([255, 255, 255], [0, 0, 0]);
    r.allow_fill = true;
    r.style.fill = "#ffffff".into();
    let result = cleanup::analyze(&im, &r);
    assert_eq!(result.fill, [255, 255, 255]);
    assert!(result.interior.is_none());
    assert!(result.mask.as_raw().contains(&255));
    assert!(result.mask.as_raw().contains(&0));
    for (x, y, m) in result.mask.enumerate_pixels() {
        if m[0] > 0 {
            let x = x as f32 + result.origin[0] as f32;
            let y = y as f32 + result.origin[1] as f32;
            assert!(x >= r.bbox[0] - 3. && x < r.bbox[2] + 3.);
            assert!(y >= r.bbox[1] - 3. && y < r.bbox[3] + 3.);
        }
    }
}
#[test]
fn rerender_is_deterministic_preserves_text_and_has_no_translation_dependency() {
    let (im, r) = fixture([250, 245, 236], [0, 0, 0]);
    let original = DynamicImage::ImageRgb8(im.clone());
    let renderer = Renderer::new(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    )
    .unwrap();
    let mut regions = vec![r.clone()];
    regions[0].review = Some("old cleanup failure".into());
    let rendered = renderer
        .render_benchmark(&original, &mut regions, "zh-Hans", None)
        .unwrap();
    assert!(regions[0].review.is_none());
    assert_eq!(regions[0].target, r.target);
    assert_eq!(regions[0].source, r.source);
    assert_eq!(regions[0].id, r.id);
    assert_eq!(
        rendered.to_rgb8(),
        renderer
            .render_benchmark(&original, &mut regions, "zh-Hans", None)
            .unwrap()
            .to_rgb8()
    );
    assert_eq!(original.to_rgb8(), im);
}

#[test]
fn reader_defaults_and_explicit_paged_setting_round_trip() {
    assert_eq!(ReaderSettings::default().layout, "continuous");
    let mut settings = AppSettings::default();
    settings.reader.layout = "paged".into();
    let restored: AppSettings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.reader.layout, "paged");
}

#[test]
fn dark_background_gets_contrasting_lettering_and_cleaned_background_bypasses_cleanup() {
    let (im, mut r) = fixture([10, 10, 10], [245, 245, 245]);
    let renderer = Renderer::new(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    )
    .unwrap();
    let original = DynamicImage::ImageRgb8(im);
    let mut regions = vec![r.clone()];
    let output = renderer
        .render_benchmark(&original, &mut regions, "zh-Hans", None)
        .unwrap();
    assert_ne!(output.to_rgb8(), original.to_rgb8());
    assert!(regions[0].review.is_none());
    assert_eq!(regions[0].style.color, r.style.color);
    r.style.color = "#ffffff".into();
    regions[0] = r;
    let white = renderer
        .render_benchmark(&original, &mut regions, "zh-Hans", None)
        .unwrap();
    assert_eq!(output.to_rgb8(), white.to_rgb8());
    assert!(regions[0].review.is_none());
    // Imported clean art is authoritative even when there is no enclosed original balloon.
    let open = DynamicImage::ImageRgb8(RgbImage::from_pixel(220, 240, Rgb([180, 180, 180])));
    let clean = DynamicImage::ImageRgb8(RgbImage::from_pixel(220, 240, Rgb([25, 25, 25])));
    let output = renderer
        .render_benchmark(&open, &mut regions, "zh-Hans", Some(&clean))
        .unwrap();
    assert!(regions[0].review.is_none());
    assert_ne!(output.to_rgb8(), clean.to_rgb8());
    assert_eq!(output.get_pixel(0, 0), clean.get_pixel(0, 0));
}

#[test]
fn cleanup_retry_keeps_cache_keys_export_pixels_and_rejects_a_stale_save() {
    use umanga_core::{documents, store};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("原稿.png");
    let (image, region) = fixture([250, 245, 236], [0, 0, 0]);
    image.save(&source).unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut page = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    page.regions = vec![region];
    page.regions[0].review = Some("Old cleanup failure".into());
    let path = temp.path().join("book.umanga");
    store::create(&path, "Test", &[page.clone()]).unwrap();
    let settings = TranslationSettings::default();
    let provider = ProviderProfile::default();
    let key = store::cache_key(&page, &settings, &provider, "").unwrap();
    let renderer = Renderer::new(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    )
    .unwrap();
    let output = renderer
        .render_benchmark(
            &documents::load(&page.source).unwrap(),
            &mut page.regions,
            "zh-Hans",
            None,
        )
        .unwrap();
    assert_eq!(
        key,
        store::cache_key(&page, &settings, &provider, "").unwrap()
    );
    assert!(!page.regions[0].allow_fill);
    let rendered = store::assets(&path).join("rendered.png");
    output.save(&rendered).unwrap();
    page.rendered = Some(rendered.to_string_lossy().into());
    assert!(store::save_page(&path, &mut page, 0).unwrap());
    let export = temp.path().join("export");
    documents::export(&store::open(&path).unwrap(), &export, "images").unwrap();
    assert_eq!(
        std::fs::read(&rendered).unwrap(),
        std::fs::read(export.join("0001.png")).unwrap()
    );
    let mut newer = page.clone();
    newer.regions[0].target = "新的手动编辑".into();
    assert!(store::save_page(&path, &mut newer, 1).unwrap());
    assert!(!store::save_page(&path, &mut page, 1).unwrap());
    assert_eq!(
        store::page(&path, &page.id).unwrap().regions[0].target,
        "新的手动编辑"
    );
    assert_eq!(original_bytes, std::fs::read(&source).unwrap());
}

#[test]
fn fallback_renders_saved_translation_without_solid_override() {
    let (mut im, mut r) = fixture([255, 255, 255], [0, 0, 0]);
    draw_filled_rect_mut(&mut im, Rect::at(40, 110).of_size(82, 3), Rgb([0, 0, 0]));
    r.review = Some("Old cleanup rejection".into());
    let mask = cleanup::analyze(&im, &r);
    assert!(mask.interior.is_none());
    let renderer = Renderer::new(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    )
    .unwrap();
    let original = DynamicImage::ImageRgb8(im);
    let mut regions = vec![r.clone()];
    let rendered = renderer
        .render_benchmark(&original, &mut regions, "zh-Hans", None)
        .unwrap();
    assert_ne!(rendered.to_rgb8(), original.to_rgb8());
    assert!(regions[0].review.is_none());
    assert_eq!(regions[0].target, r.target);
    assert_eq!(regions[0].id, r.id);
    assert!(!regions[0].allow_fill);
    // The attached artwork outside the text-area fallback stays unchanged.
    assert_eq!(rendered.get_pixel(40, 111), original.get_pixel(40, 111));
}

#[test]
fn fallback_handles_clipped_and_empty_text_rectangles_without_out_of_bounds_masks() {
    let im = RgbImage::from_pixel(24, 24, Rgb([255, 255, 255]));
    for bbox in [
        [-5., -5., 10., 10.],
        [12., 12., 40., 40.],
        [40., 40., 45., 45.],
        [5., 5., 5., 5.],
    ] {
        let result = cleanup::analyze(
            &im,
            &Region {
                bbox,
                ..Default::default()
            },
        );
        assert!(result.origin[0] + result.mask.width() <= im.width());
        assert!(result.origin[1] + result.mask.height() <= im.height());
    }
}
