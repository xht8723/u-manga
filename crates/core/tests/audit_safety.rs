use std::{fs, path::Path};
use umanga_core::{
    documents,
    library::{self, ImportPreview, Metadata},
    store,
    types::*,
};

fn book(root: &Path, source: &Path) -> library::Book {
    let pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
    library::create(
        root,
        Metadata {
            title: "Audit".into(),
            ..Default::default()
        },
        ImportPreview {
            omitted_page_ids: vec![],
            chapters: vec![library::Chapter {
                id: uid(),
                title: "One".into(),
                page_ids: pages.iter().map(|p| p.id.clone()).collect(),
                read: false,
            }],
            pages,
            warnings: vec![],
        },
        None,
    )
    .unwrap()
}
fn picture(path: &Path) {
    image::RgbImage::new(24, 32).save(path).unwrap();
}

#[test]
fn export_selection_cannot_overwrite_another_books_original() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a.png");
    let b = t.path().join("b.png");
    picture(&a);
    picture(&b);
    let root = t.path().join("library");
    let first = book(&root, &a);
    let _second = book(&root, &b);
    let before = fs::read(&b).unwrap();
    let project = store::open(Path::new(&first.path)).unwrap();
    assert!(documents::export(&project, &b, "cbz").is_err());
    assert_eq!(fs::read(&b).unwrap(), before);
}

#[test]
fn malformed_page_id_cannot_materialize_outside_assets() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let mut page = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    page.id = "../escaped".into();
    assert!(documents::materialize(&t.path().join("book.umanga"), &page).is_err());
    assert!(!t.path().join("escaped-original.png").exists());
}

#[test]
fn missing_indexed_book_can_be_removed_without_touching_originals() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let root = t.path().join("library");
    let b = book(&root, &source);
    fs::remove_file(&b.path).unwrap();
    library::delete(&root, Path::new(&b.path)).unwrap();
    assert!(library::list(&root).unwrap().is_empty());
    assert!(source.exists());
}

#[test]
fn overlapping_import_and_hostile_membership_are_rejected_without_files() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let root = t.path().join("library");
    let b = book(&root, &source);
    let nested = store::assets(Path::new(&b.path)).join("nested");
    assert!(library::import_book(&nested, Path::new(&b.path)).is_err());
    assert!(!nested.exists());
    let connection = store::connection(Path::new(&b.path)).unwrap();
    let mut broken = b.clone();
    broken.chapters[0].page_ids[0] = "../escape".into();
    connection
        .execute(
            "UPDATE meta SET value=?1 WHERE key='book'",
            [serde_json::to_string(&broken).unwrap()],
        )
        .unwrap();
    let destination = t.path().join("destination");
    assert!(library::import_book(&destination, Path::new(&b.path)).is_err());
    assert!(library::list(&destination).unwrap().is_empty());
    assert!(source.exists());
}

#[test]
fn metadata_and_cover_commit_together_and_stale_dialog_keeps_existing_cover() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let root = t.path().join("library");
    let b = book(&root, &source);
    let mut metadata = b.metadata.clone();
    metadata.title = "New title".into();
    let changed = library::update_with_cover(
        &root,
        Path::new(&b.path),
        metadata.clone(),
        b.overrides.clone(),
        b.revision,
        Some(Some(source.to_str().unwrap())),
    )
    .unwrap();
    assert_eq!(changed.metadata.title, "New title");
    assert!(changed.cover.is_some());
    assert!(
        library::update_with_cover(
            &root,
            Path::new(&b.path),
            metadata,
            b.overrides.clone(),
            b.revision,
            Some(None)
        )
        .is_err()
    );
    assert_eq!(
        library::open(Path::new(&b.path)).unwrap().cover,
        changed.cover
    );
    let completed = library::complete(&root, Path::new(&b.path), &b.chapters[0].id, true).unwrap();
    let covered =
        library::set_cover(&root, Path::new(&b.path), Some(source.to_str().unwrap())).unwrap();
    assert!(covered.chapters[0].read);
    assert!(covered.revision > completed.revision);
    let count = fs::read_dir(store::assets(Path::new(&b.path)))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("cover-")
        })
        .count();
    assert_eq!(count, 1);
}

#[test]
fn shared_originals_survive_retiring_relocated_books_in_any_order() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let root = t.path().join("library");
    let b = book(&root, &source);
    let linked = store::assets(Path::new(&b.path)).join("linked.png");
    picture(&linked);
    let a = book(&root, &linked);
    let protected = library::linked_originals(&root).unwrap();
    let dest = t.path().join("moved");
    library::import_book(&dest, Path::new(&a.path)).unwrap();
    library::import_book(&dest, Path::new(&b.path)).unwrap();
    library::delete_protected(&root, Path::new(&a.path), &protected).unwrap();
    library::delete_protected(&root, Path::new(&b.path), &protected).unwrap();
    assert!(linked.exists());
    assert!(source.exists());
}

#[test]
fn rejected_geometry_and_conflicting_save_do_not_change_page_or_revision() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let b = book(&t.path().join("library"), &source);
    let mut p = library::chapter_pages(Path::new(&b.path), &b.chapters[0].id)
        .unwrap()
        .remove(0);
    let original = serde_json::to_value(&p).unwrap();
    assert!(!store::save_page(Path::new(&b.path), &mut p, 100).unwrap());
    assert_eq!(serde_json::to_value(&p).unwrap(), original);
    p.regions.push(Region {
        bbox: [0., 0., f32::NAN, 10.],
        ..Default::default()
    });
    assert!(store::save_page(Path::new(&b.path), &mut p, 0).is_err());
    assert!(
        store::page(Path::new(&b.path), &p.id)
            .unwrap()
            .regions
            .is_empty()
    );
}

#[test]
fn repeated_generated_outputs_are_bounded_and_unknown_assets_are_retained() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let b = book(&t.path().join("library"), &source);
    let path = Path::new(&b.path);
    let mut p = library::chapter_pages(path, &b.chapters[0].id)
        .unwrap()
        .remove(0);
    let unknown = store::assets(path).join("keep-me.png");
    picture(&unknown);
    for _ in 0..40 {
        let _outputs = umanga_core::assets::PageOutputs::new(path, &p.id).unwrap();
        let output =
            umanga_core::safety::page_asset(path, &p.id, &format!("{}.png", uid())).unwrap();
        picture(&output);
        p.rendered = Some(output.to_string_lossy().into());
        let revision = p.revision;
        assert!(store::save_page(path, &mut p, revision).unwrap());
    }
    let generated = fs::read_dir(store::assets(path))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&p.id)
        })
        .count();
    assert_eq!(generated, 1);
    assert!(unknown.exists());
    assert!(source.exists());
}

#[cfg(windows)]
#[test]
fn junction_traversal_is_rejected_and_failed_import_leaves_no_partial_book() {
    use std::os::windows::process::CommandExt;
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.png");
    picture(&source);
    let b = book(&t.path().join("library"), &source);
    let external = t.path().join("external");
    fs::create_dir(&external).unwrap();
    let sentinel = external.join("keep.png");
    picture(&sentinel);
    let junction = store::assets(Path::new(&b.path)).join("linked");
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(umanga_core::safety::no_link(&junction).is_err());
    let destination = t.path().join("destination");
    assert!(library::import_book(&destination, Path::new(&b.path)).is_err());
    assert!(library::list(&destination).unwrap().is_empty());
    assert!(
        !fs::read_dir(&destination).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "umanga"))
    );
    assert!(sentinel.exists());
    // Remove only the junction itself; never recurse into its target.
    fs::remove_dir(junction).unwrap();
}
