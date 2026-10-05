use umanga_core::{
    render::{Renderer, dialogue_lettering_text, format_translation, region_lettering_text},
    types::*,
};

#[test]
fn dialogue_punctuation_keeps_symbols_and_respects_protected_tokens() {
    for (input, expected) in [
        ("甲，乙。", "甲\n乙"),
        ("阿·布=朋友", "阿·布=朋友"),
        ("·甲=乙·", "·甲=乙·"),
        ("甲、乙､丙", "甲、乙､丙"),
        ("甲！？乙", "甲！？\n乙"),
        ("等等……再说", "等等……\n再说"),
        ("等等...再说．．．嗯", "等等...\n再说．．．\n嗯"),
        ("等等——别走", "等等——\n别走"),
        ("好♥再见", "好♥\n再见"),
        ("好❤️再见👍🏽继续👩‍👩‍👧‍👦结束", "好❤️\n再见👍🏽\n继续👩‍👩‍👧‍👦\n结束"),
        ("甲♪乙→丙＋丁＝戊", "甲♪\n乙→\n丙＋\n丁＝\n戊"),
        ("「你好」再见", "「你好」再见"),
        ("「你好！」再见", "「你好！」\n再见"),
        ("（『你好！？』）再见", "（『你好！？』）\n再见"),
        ("\"Hello!\" Next.", "\"Hello!\"\nNext"),
        ("He said, 'Hello!' Next.", "He said\n'Hello!'\nNext"),
        ("3.14，don't go。", "3.14\ndon't go"),
        ("1,000.50，well-known。", "1,000.50\nwell-known"),
        (
            "Éric’s e\u{301}-mail，foo_bar。",
            "Éric’s e\u{301}-mail\nfoo_bar",
        ),
        (
            "１２．３４，５６。12:30；3/4",
            "１２．３４，５６\n12:30；\n3/4",
        ),
        ("甲，。 乙！\r\n丙\n\n丁", "甲\n乙！\n丙\n\n丁"),
        ("甲：乙；丙？丁！戊", "甲：\n乙；\n丙？\n丁！\n戊"),
        ("甲.乙-丙", "甲\n乙-\n丙"),
        (" ，。 ", ""),
    ] {
        assert_eq!(dialogue_lettering_text(input), expected, "{input:?}");
    }
}

#[test]
fn formatting_is_region_aware_and_never_changes_saved_translation() {
    let mut r = Region {
        target: "甲！乙……丙，丁。".into(),
        ..Default::default()
    };
    let saved = r.target.clone();
    assert_eq!(format_translation(&r, &r.target), "甲！\n乙……\n丙\n丁");
    r.kind = "free".into();
    assert_eq!(format_translation(&r, &r.target), "甲！乙……丙\n丁");
    r.kind = "free_text".into();
    assert_eq!(format_translation(&r, &r.target), "甲！乙……丙\n丁");
    r.bubble = Some([0., 0., 100., 100.]);
    assert_eq!(format_translation(&r, &r.target), "甲！\n乙……\n丙\n丁");
    assert_eq!(r.target, saved);
    assert_eq!(region_lettering_text(&r), saved);
}

#[test]
fn free_vertical_columns_share_dialogue_centering_and_common_tops() {
    let renderer =
        Renderer::new(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"))
            .unwrap();
    let mut r = Region {
        bbox: [0., 0., 140., 260.],
        target: "拥有外交\n豁免权".into(),
        style: TextStyle {
            size: Some(28.),
            ..Default::default()
        },
        ..Default::default()
    };
    let dialogue = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
    r.kind = "free".into();
    let caption = renderer.lettering(&r, "zh-Hans").unwrap().unwrap();
    assert_eq!(dialogue, caption);
    let mut tops = Vec::new();
    for (left, right) in [(0, 70), (70, 140)] {
        let top = (0..caption.height())
            .find(|y| {
                (left..right)
                    .any(|x| caption.data()[((y * caption.width() + x) * 4 + 3) as usize] > 0)
            })
            .unwrap();
        tops.push(top);
    }
    assert!(tops[0].abs_diff(tops[1]) <= 1);
}

#[test]
fn manual_punctuation_and_explicit_breaks_reach_both_native_layouts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let renderer = Renderer::new(&root.join("assets/fonts")).unwrap();
    let out = root.join("test-output/full-prompts/lettering");
    std::fs::create_dir_all(&out).unwrap();
    for direction in ["horizontal", "vertical"] {
        let region = Region {
            bbox: [0., 0., 320., 320.],
            target: "甲，乙。·=！\n丙？丁\n\n手工换行".into(),
            direction: direction.into(),
            style: TextStyle {
                size: Some(28.),
                ..Default::default()
            },
            ..Default::default()
        };
        let manual = renderer.lettering(&region, "zh-Hans").unwrap().unwrap();
        manual
            .save_png(out.join(format!("{direction}-manual.png")))
            .unwrap();
        let formatted = Region {
            target: format_translation(&region, &region.target),
            ..region.clone()
        };
        let generated = renderer.lettering(&formatted, "zh-Hans").unwrap().unwrap();
        generated
            .save_png(out.join(format!("{direction}-generated.png")))
            .unwrap();
        assert_ne!(
            manual, generated,
            "Renderer must preserve manual punctuation/breaks"
        );
        assert_eq!(region_lettering_text(&region), region.target);
    }
}
