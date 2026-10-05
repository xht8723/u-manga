//! Managed library index and chapter membership. Processing still uses the derived page order.
use crate::{documents, store, types::*};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub title: String,
    pub creator: String,
    pub description: String,
    pub language: String,
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overrides {
    pub translation: Option<TranslationSettings>,
    pub reader: Option<ReaderSettings>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub page_ids: Vec<String>,
    pub read: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    /// IPC-only commit warnings; never trusted from imported book metadata.
    #[serde(default, skip_deserializing, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<crate::ui_message::UiMessage>,
    pub path: String,
    pub id: String,
    pub metadata: Metadata,
    pub chapters: Vec<Chapter>,
    pub omitted_page_ids: Vec<String>,
    pub cover: Option<String>,
    pub overrides: Overrides,
    pub glossary: crate::glossary::BookGlossary,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub path: String,
    pub id: String,
    pub metadata: Metadata,
    pub chapters: usize,
    pub completed: usize,
    pub pages: usize,
    pub translated: usize,
    pub review: usize,
    pub cover: Option<String>,
    pub cover_page: Option<String>,
    pub updated: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub chapters: Vec<Chapter>,
    pub omitted_page_ids: Vec<String>,
    pub pages: Vec<Page>,
    #[serde(with = "crate::ui_message::list")]
    pub warnings: Vec<String>,
}

fn index(root: &Path) -> Result<Connection> {
    if root.as_os_str().is_empty() {
        bail!("Choose a library folder in Settings")
    }
    fs::create_dir_all(root)?;
    let file = root.join("library.sqlite");
    let creating = !file.exists();
    let c = Connection::open(file)?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    if creating {
        c.execute_batch("CREATE TABLE books(id TEXT PRIMARY KEY, data TEXT NOT NULL)")?;
        c.pragma_update(None, "user_version", store::FORMAT_VERSION)?;
    } else {
        let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != store::FORMAT_VERSION {
            bail!(
                "Unsupported library format. Choose an empty library folder for this release; existing files are unchanged."
            )
        }
    }
    c.execute_batch("PRAGMA journal_mode=WAL")?;
    Ok(c)
}
pub fn list(root: &Path) -> Result<Vec<BookSummary>> {
    let c = index(root)?;
    let mut q = c.prepare("SELECT data FROM books")?;
    q.query_map([], |r| r.get::<_, String>(0))?
        .map(|r| {
            let mut b: BookSummary = serde_json::from_str(&r?)?;
            let name = Path::new(&b.path)
                .file_name()
                .context("Invalid library entry")?;
            let path = root.join(name);
            if let Some(cover) = &b.cover {
                b.cover = Some(
                    store::assets(&path)
                        .join(Path::new(cover).file_name().context("Invalid cover")?)
                        .to_string_lossy()
                        .into(),
                );
            }
            b.path = path.to_string_lossy().into();
            Ok(b)
        })
        .collect()
}
/// Explicit setup/apply operation: confirm the selected library can accept writes.
pub fn validate_location(root: &Path) -> Result<()> {
    let c = index(root)?;
    c.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")?;
    Ok(())
}
pub(crate) fn get(c: &Connection) -> Result<Book> {
    let json: String = c.query_row("SELECT value FROM meta WHERE key='book'", [], |r| r.get(0))?;
    let b: Book = serde_json::from_str(&json)?;
    crate::glossary::validate(&mut b.glossary.clone())?;
    crate::safety::identity(&b.id)?;
    anyhow::ensure!(
        b.revision < i64::MAX as u64,
        "Book revision exceeds storage limits"
    );
    let mut chapters = HashSet::new();
    let mut pages = HashSet::new();
    for ch in &b.chapters {
        crate::safety::identity(&ch.id)?;
        anyhow::ensure!(chapters.insert(&ch.id), "Duplicate chapter ID");
        for id in &ch.page_ids {
            crate::safety::identity(id)?;
            anyhow::ensure!(pages.insert(id), "Duplicate page membership");
        }
    }
    let mut omitted = HashSet::new();
    for id in &b.omitted_page_ids {
        crate::safety::identity(id)?;
        anyhow::ensure!(pages.contains(id), "Omitted page is not in this book");
        anyhow::ensure!(omitted.insert(id), "Duplicate omitted page ID");
    }
    Ok(b)
}
pub fn open(path: &Path) -> Result<Book> {
    if !path.is_file() {
        bail!("Book file is missing; restore it or remove its library entry")
    }
    let c = store::connection(path)?;
    let mut b = get(&c)?;
    b.path = path.to_string_lossy().into();
    if let Some(cover) = &b.cover {
        b.cover = Some(
            store::assets(path)
                .join(Path::new(cover).file_name().context("Invalid cover")?)
                .to_string_lossy()
                .into(),
        );
    }
    Ok(b)
}
pub(crate) fn put(c: &Connection, b: &Book) -> Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO meta VALUES('book',?1)",
        [serde_json::to_string(b)?],
    )?;
    c.execute(
        "UPDATE meta SET value=?1 WHERE key='title'",
        [&b.metadata.title],
    )?;
    Ok(())
}
pub fn refresh(root: &Path, path: &Path) -> Result<BookSummary> {
    // Serialize publication, including the read, so an older refresh cannot publish last.
    static PUBLICATION: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _publication = PUBLICATION.lock();
    let mut c = store::connection(path)?;
    let tx = c.transaction()?;
    let mut b = get(&tx)?;
    b.path = path.to_string_lossy().into();
    if let Some(cover) = &b.cover {
        b.cover = Some(
            crate::safety::asset(
                path,
                Path::new(cover)
                    .file_name()
                    .context("Invalid cover")?
                    .to_str()
                    .context("Invalid cover")?,
            )?
            .to_string_lossy()
            .into(),
        );
    }
    let eligible = store::eligible_page_ids(&tx, false)?;
    let (mut pages, mut translated, mut review) = (0, 0, 0);
    {
        let mut q = tx.prepare("SELECT id,json_extract(data,'$.rendered') IS NOT NULL,json_extract(data,'$.status')='review' FROM pages")?;
        for row in q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })? {
            let (id, rendered, needs_review) = row?;
            pages += 1;
            translated += usize::from(rendered && eligible.contains(&id));
            review += usize::from(needs_review);
        }
    }
    let summary = BookSummary {
        path: b.path,
        id: b.id,
        metadata: b.metadata,
        chapters: b.chapters.len(),
        completed: b.chapters.iter().filter(|c| c.read).count(),
        pages,
        translated,
        review,
        cover: b.cover,
        cover_page: b.chapters.iter().find_map(|c| c.page_ids.first().cloned()),
        updated: now(),
    };
    tx.commit()?;
    index(root)?.execute(
        "INSERT OR REPLACE INTO books VALUES(?1,?2)",
        params![summary.id, serde_json::to_string(&summary)?],
    )?;
    Ok(summary)
}
pub fn create(
    root: &Path,
    metadata: Metadata,
    preview: ImportPreview,
    cover: Option<String>,
) -> Result<Book> {
    if metadata.title.trim().is_empty() {
        bail!("Enter a book title")
    }
    index(root)?;
    let path = root.join(format!("{}.umanga", uid()));
    let p = store::create(&path, &metadata.title, &[])?;
    let b = Book {
        warnings: vec![],
        path: p.path,
        id: p.id,
        metadata,
        chapters: vec![],
        omitted_page_ids: vec![],
        cover: None,
        overrides: Overrides::default(),
        glossary: Default::default(),
        revision: 0,
    };
    put(&store::connection(&path)?, &b)?;
    let result = (|| {
        organize(
            root,
            &path,
            0,
            preview.chapters,
            preview.pages,
            preview.omitted_page_ids,
        )?;
        if let Some(cover) = cover {
            set_cover(root, &path, Some(&cover))?;
        }
        open(&path)
    })();
    if result.is_err() {
        let _ = delete(root, &path);
    }
    result
}
pub fn update(
    root: &Path,
    path: &Path,
    metadata: Metadata,
    overrides: Overrides,
    expected: u64,
) -> Result<Book> {
    update_with_cover(root, path, metadata, overrides, expected, None)
}
/// Glossary has its own revision: chapter completion and metadata edits do not
/// conflict, but another glossary editor must never be overwritten silently.
pub fn refresh_committed(root: &Path, path: &Path, book: Book) -> Book {
    committed(root, path, book)
}
fn committed(root: &Path, path: &Path, mut book: Book) -> Book {
    book.path = path.to_string_lossy().into();
    if let Err(error) = refresh(root, path) {
        let mut warning = crate::ui_message::UiMessage::from_text(
            "Changes saved; the Library summary could not refresh. Reopen the book to refresh it.",
        );
        warning.detail = Some(error.to_string());
        book.warnings.push(warning);
    }
    book
}
pub fn save_glossary(
    root: &Path,
    path: &Path,
    expected: u64,
    mut glossary: crate::glossary::BookGlossary,
) -> Result<Book> {
    crate::glossary::validate(&mut glossary)?;
    let mut c = store::connection(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut b = get(&tx)?;
    anyhow::ensure!(
        b.glossary.revision == expected,
        "Glossary changed elsewhere. Reopen it before saving; your draft is still available."
    );
    let automatic: HashSet<_> = b.glossary.automatic_sources.iter().collect();
    let before: HashMap<_, _> = b
        .glossary
        .entries
        .iter()
        .map(|e| (&e.source, &e.target))
        .collect();
    let after: HashMap<_, _> = glossary
        .entries
        .iter()
        .map(|e| (&e.source, &e.target))
        .collect();
    glossary
        .automatic_sources
        .retain(|source| automatic.contains(source) && before.get(source) == after.get(source));
    glossary.revision = expected
        .checked_add(1)
        .context("Glossary revision exceeds storage limits")?;
    b.glossary = glossary;
    b.revision += 1;
    put(&tx, &b)?;
    tx.commit()?;
    Ok(committed(root, path, b))
}
pub fn update_with_cover(
    root: &Path,
    path: &Path,
    metadata: Metadata,
    overrides: Overrides,
    expected: u64,
    cover: Option<Option<&str>>,
) -> Result<Book> {
    if metadata.title.trim().is_empty() {
        bail!("Enter a book title")
    }
    let mut cover_output = None;
    let mut c = store::connection(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut b = get(&tx)?;
    if b.revision != expected {
        bail!("Book changed; reopen it before applying")
    }
    b.metadata = metadata;
    b.overrides = overrides;
    if let Some(source) = cover {
        let (value, output) = prepare_cover(path, source)?;
        b.cover = value;
        cover_output = output;
    }
    b.revision += 1;
    put(&tx, &b)?;
    tx.commit()?;
    if let Some(output) = &mut cover_output {
        output.commit();
    }
    let _ = crate::assets::collect(path, None);
    Ok(committed(root, path, b))
}
pub fn complete(root: &Path, path: &Path, id: &str, read: bool) -> Result<Book> {
    let mut c = store::connection(path)?;
    let tx = c.transaction()?;
    let mut b = get(&tx)?;
    let ch = b
        .chapters
        .iter_mut()
        .find(|c| c.id == id)
        .context("Chapter not found")?;
    ch.read = read;
    b.revision += 1;
    put(&tx, &b)?;
    tx.commit()?;
    Ok(committed(root, path, b))
}
pub fn chapter_pages(path: &Path, id: &str) -> Result<Vec<Page>> {
    let mut c = store::connection(path)?;
    let tx = c.transaction()?;
    let b = get(&tx)?;
    let ch = b
        .chapters
        .iter()
        .find(|c| c.id == id)
        .context("Chapter not found")?;
    let mut query = tx.prepare(
        "SELECT data FROM pages WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY number",
    )?;
    query
        .query_map([serde_json::to_string(&ch.page_ids)?], |r| {
            r.get::<_, String>(0)
        })?
        .map(|s| {
            let mut p: Page = serde_json::from_str(&s?)?;
            crate::safety::page(&p)?;
            store::resolve_assets(path, &mut p);
            Ok(p)
        })
        .collect()
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceIdentity {
    pub id: String,
    pub source: Source,
}
pub fn source_identities(path: &Path) -> Result<Vec<SourceIdentity>> {
    let c = store::connection(path)?;
    let mut q = c.prepare("SELECT id,json_extract(data,'$.source') FROM pages")?;
    q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|row| {
            let (id, source) = row?;
            Ok(SourceIdentity {
                id,
                source: serde_json::from_str(&source)?,
            })
        })
        .collect()
}
pub fn export_protection(book: &Path) -> Result<crate::safety::ProtectedPaths> {
    let root = book.parent().context("Book has no parent")?;
    let mut protected = crate::safety::ProtectedPaths::default();
    let books = if root.join("library.sqlite").is_file() {
        list(root)?
            .into_iter()
            .map(|b| PathBuf::from(b.path))
            .collect()
    } else {
        vec![book.to_owned()]
    };
    for p in books {
        for suffix in ["", "-wal", "-shm"] {
            protected
                .files
                .insert(crate::safety::resolved(Path::new(&format!(
                    "{}{suffix}",
                    p.display()
                )))?);
        }
        protected
            .directories
            .push(crate::safety::resolved(&store::assets(&p))?);
        if p.is_file() {
            for s in source_identities(&p)? {
                protected
                    .files
                    .insert(crate::safety::resolved(Path::new(&s.source.path))?);
            }
        }
    }
    for suffix in ["", "-wal", "-shm"] {
        protected.files.insert(crate::safety::resolved(
            &root.join(format!("library.sqlite{suffix}")),
        )?);
    }
    protected
        .directories
        .push(crate::safety::resolved(&root.join("models"))?);
    Ok(protected)
}
pub fn effective(b: &Book, defaults: &AppSettings) -> TranslationSettings {
    let mut s = b
        .overrides
        .translation
        .clone()
        .unwrap_or_else(|| defaults.translation.clone());
    if !b.metadata.language.is_empty() {
        s.source_language = b.metadata.language.clone();
    }
    s.auto_glossary = b.glossary.auto_detect;
    s.glossary_enabled = b.glossary.enabled;
    // Disabled terms stay on the book, not duplicated into every queued page snapshot.
    s.glossary = if s.glossary_enabled {
        b.glossary.entries.clone()
    } else {
        Vec::new()
    };
    s.deepl_glossary_id = b.glossary.deepl_glossary_id.clone();
    s
}
/// Commit membership and omission; existing page payloads come from SQLite, never stale UI snapshots.
pub fn organize(
    root: &Path,
    path: &Path,
    expected: u64,
    mut chapters: Vec<Chapter>,
    added: Vec<Page>,
    omitted_page_ids: Vec<String>,
) -> Result<Book> {
    let mut c = store::connection(path)?;
    let tx = c.transaction()?;
    let mut b = get(&tx)?;
    if b.revision != expected {
        bail!("Book changed; reopen the organizer")
    }
    let mut existing: HashMap<String, Page> = {
        let mut q = tx.prepare("SELECT data FROM pages")?;
        q.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| {
                let p: Page = serde_json::from_str(&r?)?;
                Ok((p.id.clone(), p))
            })
            .collect::<Result<_>>()?
    };
    let original_ids: HashSet<_> = existing.keys().cloned().collect();
    let requested_ids: HashSet<_> = chapters.iter().flat_map(|c| c.page_ids.iter()).collect();
    let mut protected = if original_ids.iter().any(|id| !requested_ids.contains(id)) {
        linked_originals(root)?
    } else {
        HashSet::new()
    };
    for p in added {
        crate::safety::page(&p)?;
        if !existing.contains_key(&p.id) {
            documents::source_health(&p)?;
            protected.insert(crate::safety::resolved(Path::new(&p.source.path))?);
            existing.insert(p.id.clone(), p);
        }
    }
    let mut seen = HashSet::new();
    let mut chapter_ids = HashSet::new();
    let mut n = 0;
    let retained_chapters: HashSet<_> = chapters.iter().map(|c| c.id.clone()).collect();
    for ch in &mut chapters {
        crate::safety::identity(&ch.id)?;
        if ch.title.trim().is_empty() || !chapter_ids.insert(ch.id.clone()) {
            bail!("Chapter names must be nonempty and IDs unique")
        }
        // Completion follows source membership, including split and merge. New pages make recipients unread.
        if !ch.page_ids.is_empty() {
            ch.read = ch.read
                && ch.page_ids.iter().all(|id| {
                    b.chapters
                        .iter()
                        .find(|old| old.page_ids.contains(id))
                        .is_some_and(|old| old.read)
                });
            if let Some(original) = b.chapters.iter().find(|c| c.id == ch.id) {
                let receiving = ch
                    .page_ids
                    .iter()
                    .filter(|id| !original.page_ids.contains(id));
                for id in receiving {
                    let merged = b
                        .chapters
                        .iter()
                        .find(|c| c.page_ids.contains(id))
                        .is_some_and(|source| {
                            !retained_chapters.contains(&source.id)
                                && source.page_ids.iter().all(|id| ch.page_ids.contains(id))
                        });
                    if !merged {
                        ch.read = false;
                    }
                }
            }
        } else {
            ch.read = b
                .chapters
                .iter()
                .find(|old| old.id == ch.id)
                .is_some_and(|old| old.read);
        }
        for id in &ch.page_ids {
            if !seen.insert(id.clone()) {
                bail!("A page may belong to only one chapter")
            }
            let p = existing.get_mut(id).context("Unknown page in chapter")?;
            p.number = n;
            n += 1;
            // Bump revisions so late editor/worker responses cannot replace the new order.
            p.revision += 1;
            tx.execute(
                "INSERT OR REPLACE INTO pages VALUES(?1,?2,?3,?4)",
                params![p.id, p.number, p.revision, serde_json::to_string(p)?],
            )?;
        }
    }
    for id in original_ids.difference(&seen) {
        tx.execute("DELETE FROM pages WHERE id=?1", [id])?;
        tx.execute(
            "DELETE FROM jobs WHERE json_extract(data,'$.pageId')=?1",
            [id],
        )?;
    }
    let mut omitted = HashSet::new();
    for id in &omitted_page_ids {
        crate::safety::identity(id)?;
        anyhow::ensure!(
            seen.contains(id),
            "Omitted page is not in the retained chapters"
        );
        anyhow::ensure!(omitted.insert(id), "Duplicate omitted page ID");
    }
    b.omitted_page_ids = chapters
        .iter()
        .flat_map(|c| &c.page_ids)
        .filter(|id| omitted.contains(id))
        .cloned()
        .collect();
    b.chapters = chapters;
    b.revision += 1;
    put(&tx, &b)?;
    tx.commit()?;
    // The transaction is durable. Retirement failures must not report membership as unsaved.
    for id in original_ids.difference(&seen) {
        if let Err(error) = crate::assets::collect_protected(path, Some(id), &protected) {
            let mut warning = crate::ui_message::UiMessage::from_text(
                "Changes saved; unused generated files could not be removed. They remain available.",
            );
            warning.detail = Some(error.to_string());
            b.warnings.push(warning);
            break;
        }
    }
    Ok(committed(root, path, b))
}
fn prepare_cover(
    path: &Path,
    source: Option<&str>,
) -> Result<(Option<String>, Option<crate::assets::ProvisionalOutput>)> {
    Ok(if let Some(s) = source {
        if !documents::image_ext(s) {
            bail!("Choose a PNG, JPEG or WebP cover")
        }
        let im = crate::image_input::open(Path::new(s))?;
        let dest = crate::safety::asset(path, &format!("cover-{}.png", uid()))?;
        let output = crate::assets::ProvisionalOutput::image(path, &dest, &im.thumbnail(600, 900))?;
        (Some(dest.to_string_lossy().into()), Some(output))
    } else {
        (None, None)
    })
}
pub fn set_cover(root: &Path, path: &Path, source: Option<&str>) -> Result<Book> {
    let (cover, mut cover_output) = prepare_cover(path, source)?;
    let mut c = store::connection(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut b = get(&tx)?;
    b.cover = cover;
    b.revision += 1;
    put(&tx, &b)?;
    tx.commit()?;
    if let Some(output) = &mut cover_output {
        output.commit();
    }
    let _ = crate::assets::collect(path, None);
    Ok(committed(root, path, b))
}
pub fn delete(root: &Path, path: &Path) -> Result<()> {
    delete_protected(root, path, &HashSet::new())
}
pub fn delete_protected(root: &Path, path: &Path, extra: &HashSet<PathBuf>) -> Result<()> {
    if !path.exists() {
        let wanted = crate::safety::resolved(path)?;
        anyhow::ensure!(
            wanted.parent() == Some(root.canonicalize()?.as_path()),
            "Only managed books can be removed"
        );
        let entry = list(root)?
            .into_iter()
            .find(|b| crate::safety::resolved(Path::new(&b.path)).ok().as_ref() == Some(&wanted))
            .context("Book is not indexed")?;
        index(root)?.execute("DELETE FROM books WHERE id=?1", [entry.id])?;
        return Ok(());
    }
    let canonical = path.canonicalize()?;
    let root = root.canonicalize()?;
    if canonical.parent() != Some(root.as_path())
        || canonical.extension().and_then(|s| s.to_str()) != Some("umanga")
    {
        bail!("Only managed books can be removed")
    }
    let b = open(&canonical)?;
    // A linked original may itself have been selected from an owned asset directory.
    // Preserve linked files, including those referenced by another managed book.
    let mut linked = linked_originals(&root)?;
    linked.extend(extra.iter().cloned());
    let c = store::connection(&canonical)?;
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(c);
    fs::remove_file(&canonical)?;
    let owned = store::assets(&canonical);
    if owned.exists() && owned.canonicalize()?.parent() == Some(root.as_path()) {
        prune_owned(&owned, &linked)?;
    }
    index(&root)?.execute("DELETE FROM books WHERE id=?1", [b.id])?;
    Ok(())
}
pub fn linked_originals(root: &Path) -> Result<HashSet<PathBuf>> {
    let mut linked = HashSet::new();
    for summary in list(root)? {
        if !Path::new(&summary.path).is_file() {
            continue;
        }
        let c = store::connection(Path::new(&summary.path))?;
        let mut q = c.prepare("SELECT json_extract(data,'$.source.path') FROM pages")?;
        for source in q.query_map([], |r| r.get::<_, String>(0))? {
            if let Ok(p) = Path::new(&source?).canonicalize() {
                linked.insert(p);
            }
        }
    }
    Ok(linked)
}
/// Finish retirement of a journal-proven relocated copy, including a prior partial deletion.
/// Ordinary missing-book removal intentionally does not have this asset-cleanup authority.
pub fn retire_copy(root: &Path, path: &Path, id: &str, protected: &HashSet<PathBuf>) -> Result<()> {
    crate::safety::identity(id)?;
    if path.exists() {
        crate::safety::no_link(path)?;
    }
    let root = root.canonicalize()?;
    let path = crate::safety::resolved(path)?;
    anyhow::ensure!(
        path.parent() == Some(root.as_path())
            && path.extension().and_then(|s| s.to_str()) == Some("umanga"),
        "Only managed books can be removed"
    );
    if path.exists() {
        anyhow::ensure!(
            open(&path)?.id == id,
            "Stored book identity is inconsistent"
        );
        return delete_protected(&root, &path, protected);
    }
    let indexed = list(&root)?.into_iter().find(|b| b.id == id);
    let Some(indexed) = indexed else {
        return Ok(());
    };
    anyhow::ensure!(
        crate::safety::same_path(Path::new(&indexed.path), &path)?,
        "Stored book identity is inconsistent"
    );
    let mut linked = linked_originals(&root)?;
    linked.extend(protected.iter().cloned());
    let owned = store::assets(&path);
    if owned.exists() {
        prune_owned(&owned, &linked)?;
    }
    index(&root)?.execute("DELETE FROM books WHERE id=?1", [id])?;
    Ok(())
}
fn prune_owned(folder: &Path, linked: &HashSet<PathBuf>) -> Result<()> {
    crate::safety::no_link(folder)?;
    for entry in fs::read_dir(folder)? {
        let e = entry?;
        crate::safety::no_link(&e.path())?;
        let kind = e.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            prune_owned(&e.path(), linked)?;
        } else if !linked.contains(&e.path().canonicalize()?) {
            fs::remove_file(e.path())?;
        }
    }
    if fs::read_dir(folder)?.next().is_none() {
        fs::remove_dir(folder)?;
    }
    Ok(())
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    crate::safety::separate(from, to)?;
    crate::safety::no_link(from)?;
    if to.exists() {
        crate::safety::no_link(to)?;
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let e = entry?;
        crate::safety::no_link(&e.path())?;
        if e.file_type()?.is_symlink() {
            continue;
        }
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        } else {
            copy_verified(&e.path(), &to.join(e.file_name()))?;
        }
    }
    Ok(())
}
/// Copy the single library-owned model directory without following links outside it.
fn copy_verified(from: &Path, to: &Path) -> Result<()> {
    crate::safety::no_link(from)?;
    if to.exists() {
        crate::safety::no_link(to)?;
    }
    fs::copy(from, to)?;
    anyhow::ensure!(
        store::file_hash(from)? == store::file_hash(to)?,
        "Copied file differs from its source: {}",
        from.display()
    );
    Ok(())
}
/// Originals are retained until the caller has durably switched to the copied library.
pub fn copy_model_storage(from: &Path, to: &Path) -> Result<()> {
    fn copy(from: &Path, to: &Path, boundary: &Path) -> Result<()> {
        if to.exists() {
            crate::safety::no_link(to)?;
        }
        let metadata = fs::symlink_metadata(from)?;
        if metadata.file_type().is_symlink() || !from.canonicalize()?.starts_with(boundary) {
            bail!(
                "Model directory contains an external link: {}",
                from.display()
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                bail!(
                    "Model directory contains a linked folder or file: {}",
                    from.display()
                );
            }
        }
        if metadata.is_dir() {
            fs::create_dir_all(to)?;
            for entry in fs::read_dir(from)? {
                let entry = entry?;
                copy(&entry.path(), &to.join(entry.file_name()), boundary)?;
            }
        } else {
            copy_verified(from, to)?;
        }
        Ok(())
    }
    let source = from.join("models");
    if !source.exists() {
        return Ok(());
    }
    if to.exists() {
        crate::safety::no_link(to)?;
    }
    fs::create_dir_all(to)?;
    let boundary = source.canonicalize()?;
    let target = to.canonicalize()?;
    if target.starts_with(&boundary) || boundary.starts_with(&target) {
        bail!("Choose a separate destination for the library");
    }
    copy(&source, &to.join("models"), &boundary)
}

/// Only library relocation rewrites captured storage paths; ordinary settings edits do not.
pub fn relocate_job_storage(book: &Path, old_library: &Path, new_library: &Path) -> Result<()> {
    let mut jobs = store::jobs(book)?;
    for job in &mut jobs {
        job.project = book.to_string_lossy().into();
        if crate::safety::same_path(Path::new(&job.model_directory), &old_library.join("models"))? {
            job.model_directory = new_library.join("models").to_string_lossy().into();
        }
    }
    store::save_jobs(book, &jobs)
}
// A failed import owns only its freshly allocated destination. Unknown or linked
// paths are never removed, even when this leaves a recoverable partial copy.
struct StagedImport {
    path: PathBuf,
    committed: bool,
}
impl Drop for StagedImport {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let cleanup = || -> Result<()> {
            let assets = store::assets(&self.path);
            if assets.exists() {
                prune_owned(&assets, &HashSet::new())?;
            }
            for suffix in ["", "-wal", "-shm"] {
                let file = PathBuf::from(format!("{}{suffix}", self.path.display()));
                if file.exists() {
                    crate::safety::no_link(&file)?;
                    fs::remove_file(file)?;
                }
            }
            Ok(())
        };
        if let Err(e) = cleanup() {
            eprintln!("Partial import preserved for recovery: {e}");
        }
    }
}
/// Validate the actual current-format snapshot before import or retiring another copy.
pub fn validate_book(source: &Path) -> Result<Book> {
    let mut c = Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = c.transaction()?;
    let book = validate_book_connection(&tx)?;
    tx.commit()?;
    Ok(book)
}
fn validate_book_connection(c: &Connection) -> Result<Book> {
    store::check_version(c)?;
    let source_book = get(c)?;
    let _: String = c.query_row("SELECT value FROM meta WHERE key='title'", [], |r| r.get(0))?;
    let settings: String = c.query_row("SELECT value FROM meta WHERE key='settings'", [], |r| {
        r.get(0)
    })?;
    let _: TranslationSettings = serde_json::from_str(&settings)?;
    let stored_id: String =
        c.query_row("SELECT value FROM meta WHERE key='id'", [], |r| r.get(0))?;
    anyhow::ensure!(
        stored_id == source_book.id,
        "Stored book identity is inconsistent"
    );
    let expected: HashSet<_> = source_book
        .chapters
        .iter()
        .flat_map(|ch| ch.page_ids.iter().cloned())
        .collect();
    let ordered: Vec<_> = source_book
        .chapters
        .iter()
        .flat_map(|ch| ch.page_ids.iter())
        .collect();
    anyhow::ensure!(
        ordered.len() == expected.len(),
        "Duplicate chapter membership"
    );
    let mut actual = HashSet::new();
    let mut query = c.prepare("SELECT id,number,revision,data FROM pages ORDER BY number")?;
    for (position, row) in query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, usize>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .enumerate()
    {
        let (id, number, revision, json) = row?;
        let page: Page = serde_json::from_str(&json)?;
        crate::safety::page(&page)?;
        anyhow::ensure!(
            number == position
                && page.number == number
                && page.revision == revision
                && ordered
                    .get(position)
                    .is_some_and(|expected| **expected == id),
            "Stored page order or revision is inconsistent"
        );
        anyhow::ensure!(id == page.id && actual.insert(id), "Invalid page identity");
    }
    drop(query);
    anyhow::ensure!(
        actual == expected,
        "Chapter membership does not match stored pages"
    );
    let jobs = store::job_rows(c)?;
    anyhow::ensure!(
        jobs.invalid == 0,
        "Cannot import invalid Jobs: {}",
        jobs.errors.join("\n")
    );
    Ok(source_book)
}
pub fn import_book(root: &Path, source: &Path) -> Result<Book> {
    crate::safety::separate(&store::assets(source), root)?;
    index(root)?;
    let source_book = validate_book(source)?;
    let c = Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Copying a current-format book is idempotent; it never upgrades older files.
    if let Some(existing) = list(root)?.into_iter().find(|s| s.id == source_book.id) {
        validate_book(Path::new(&existing.path))?;
        return open(Path::new(&existing.path));
    }
    let path = root.join(format!("{}.umanga", uid()));
    let mut staged = StagedImport {
        path: path.clone(),
        committed: false,
    };
    c.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    drop(c);
    let copied = validate_book(&path)?;
    anyhow::ensure!(copied.id == source_book.id, "Book changed during import");
    if store::assets(source).is_dir() {
        copy_tree(&store::assets(source), &store::assets(&path))?;
    } else {
        fs::create_dir_all(store::assets(&path))?;
    }
    let p = store::open(&path)?;
    let mut b = open(&path)?;
    b.path = path.to_string_lossy().into();
    let mut c = store::connection(&path)?;
    let tx = c.transaction()?;
    for mut page in p.pages {
        if let Some(c) = &mut page.cleanup
            && let Some(name) = Path::new(&c.path).file_name()
        {
            c.path = store::assets(&path).join(name).to_string_lossy().into();
        }
        for asset in [&mut page.rendered, &mut page.background] {
            if let Some(old) = asset.as_ref()
                && let Some(name) = Path::new(old).file_name()
            {
                *asset = Some(store::assets(&path).join(name).to_string_lossy().into());
            }
        }
        tx.execute(
            "UPDATE pages SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&page)?, page.id],
        )?;
    }
    put(&tx, &b)?;
    tx.commit()?;
    refresh(root, &path)?;
    staged.committed = true;
    open(&path)
}
fn supported(p: &Path) -> bool {
    documents::image_ext(&p.to_string_lossy())
        || matches!(
            p.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "cbz" | "zip" | "pdf"
        )
}
fn children(p: &Path) -> Result<Vec<PathBuf>> {
    crate::safety::no_link(p)?;
    let mut entries = fs::read_dir(p)?
        .filter_map(|e| e.ok())
        .filter(|e| crate::safety::no_link(&e.path()).is_ok())
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.sort_by_key(|p| documents::natural(&p.to_string_lossy()));
    Ok(entries)
}
fn collect(p: &Path, files: &mut Vec<PathBuf>, cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        bail!("Scan cancelled")
    }
    crate::safety::no_link(p)?;
    if p.is_dir() {
        for ch in children(p)? {
            collect(&ch, files, cancel)?;
        }
    } else if supported(p) {
        files.push(p.to_owned());
    }
    Ok(())
}
pub fn scan(
    paths: &[String],
    automatic: bool,
    cancel: &CancellationToken,
) -> Result<ImportPreview> {
    let mut out = ImportPreview::default();
    let mut seen = HashSet::new();
    let mut groups: Vec<(String, Vec<PathBuf>)> = vec![];
    let mut selected_images: Vec<PathBuf> = vec![];
    for s in paths {
        let p = Path::new(s);
        if automatic && p.is_file() && documents::image_ext(s) {
            selected_images.push(p.to_owned());
            continue;
        }
        let name = (if p.is_dir() {
            p.file_name()
        } else {
            p.file_stem()
        })
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
        if p.is_dir() && automatic {
            let entries = children(p)?;
            let mut loose = vec![];
            for e in entries {
                if documents::image_ext(&e.to_string_lossy()) && e.is_file() {
                    loose.push(e);
                } else if e.is_dir() || supported(&e) {
                    let mut f = vec![];
                    collect(&e, &mut f, cancel)?;
                    groups.push((
                        (if e.is_dir() {
                            e.file_name()
                        } else {
                            e.file_stem()
                        })
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                        f,
                    ));
                }
            }
            if !loose.is_empty() {
                groups.push((name, loose));
            }
        } else {
            let mut f = vec![];
            collect(p, &mut f, cancel)?;
            groups.push((name, f));
        }
    }
    if !selected_images.is_empty() {
        selected_images.sort_by_key(|p| documents::natural(&p.to_string_lossy()));
        let parent = selected_images[0].parent();
        let title = if selected_images.iter().all(|p| p.parent() == parent) {
            parent
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Pages".into())
        } else {
            "Pages".into()
        };
        groups.push((title, selected_images));
    }
    if automatic {
        groups.sort_by_key(|(name, _)| documents::natural(name));
    } else if groups.len() > 1 {
        let title = groups[0].0.clone();
        let files = groups.into_iter().flat_map(|(_, files)| files).collect();
        groups = vec![(title, files)];
    }
    for (title, files) in groups {
        let mut chapter = Chapter {
            id: uid(),
            title,
            page_ids: vec![],
            read: false,
        };
        for file in files {
            if cancel.is_cancelled() {
                bail!("Scan cancelled")
            }
            match documents::import_cancellable(&[file.to_string_lossy().into()], Some(cancel)) {
                Ok(pages) => {
                    for mut p in pages {
                        let key = format!(
                            "{}|{:?}|{}",
                            p.source.path.to_lowercase(),
                            p.source.entry,
                            p.source.index
                        );
                        if !seen.insert(key) {
                            out.warnings.push(format!("Duplicate skipped: {}", p.name));
                            continue;
                        }
                        p.number = out.pages.len();
                        chapter.page_ids.push(p.id.clone());
                        out.pages.push(p);
                    }
                }
                Err(e) => out.warnings.push(format!("{}: {e}", file.display())),
            }
        }
        out.chapters.push(chapter);
    }
    if cancel.is_cancelled() {
        bail!("Scan cancelled")
    }
    Ok(out)
}
