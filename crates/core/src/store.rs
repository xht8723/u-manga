use crate::types::*;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
pub fn file_hash(path: &Path) -> Result<String> {
    file_hash_cancellable(path, &tokio_util::sync::CancellationToken::new())
}
pub fn file_hash_cancellable(
    path: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<String> {
    let _permit = crate::hash_work::acquire(cancel)?;
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?
    }
    let temp = path.with_extension(format!("{}.tmp", uid()));
    use std::io::Write;
    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = Temporary(temp.clone());
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace_file(&temp, path)?;
    Ok(())
}
pub fn replace_file(source: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    fs::rename(source, target)?;
    Ok(())
}
pub fn assets(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}-data", path.display()))
}
pub const FORMAT_VERSION: u32 = 19;

pub fn check_version(c: &Connection) -> Result<()> {
    let version: String = c
        .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
            r.get(0)
        })
        .context("Unsupported book file; create a new book with this version of U-Manga")?;
    if version != FORMAT_VERSION.to_string() {
        bail!(
            "Unsupported book format {version}; this release requires format {FORMAT_VERSION}. Create a new book from its original sources."
        )
    }
    Ok(())
}
pub fn connection(path: &Path) -> Result<Connection> {
    open_connection(path, false)
}
fn open_connection(path: &Path, create: bool) -> Result<Connection> {
    let mut flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE;
    if create {
        flags |= rusqlite::OpenFlags::SQLITE_OPEN_CREATE;
    }
    let c =
        Connection::open_with_flags(path, flags).context("Book file is missing or inaccessible")?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    if create {
        c.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE pages(id TEXT PRIMARY KEY,number INTEGER NOT NULL,revision INTEGER NOT NULL,data TEXT NOT NULL); CREATE TABLE cache(key TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE jobs(id TEXT PRIMARY KEY,data TEXT NOT NULL);")?;
        c.execute(
            "INSERT INTO meta VALUES('version',?1)",
            [FORMAT_VERSION.to_string()],
        )?;
    } else {
        check_version(&c)?;
    }
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(c)
}
pub fn create(path: &Path, title: &str, pages: &[Page]) -> Result<Project> {
    if path.exists() {
        bail!("Book already exists; choose a new book filename")
    };
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?
    };
    let mut c = open_connection(path, true)?;
    let t = c.transaction()?;
    let id = uid();
    let book = crate::library::Book {
        warnings: vec![],
        path: path.to_string_lossy().into(),
        id: id.clone(),
        metadata: crate::library::Metadata {
            title: title.into(),
            ..Default::default()
        },
        chapters: if pages.is_empty() {
            vec![]
        } else {
            vec![crate::library::Chapter {
                id: uid(),
                title: title.into(),
                page_ids: pages.iter().map(|p| p.id.clone()).collect(),
                read: false,
            }]
        },
        cover: None,
        omitted_page_ids: vec![],
        overrides: Default::default(),
        glossary: Default::default(),
        revision: 0,
    };
    for (k, v) in [
        ("id", id),
        ("title", title.into()),
        ("book", serde_json::to_string(&book)?),
        (
            "settings",
            serde_json::to_string(&TranslationSettings::default())?,
        ),
    ] {
        t.execute("INSERT INTO meta VALUES(?1,?2)", params![k, v])?;
    }
    for p in pages {
        t.execute(
            "INSERT INTO pages VALUES(?1,?2,?3,?4)",
            params![p.id, p.number, p.revision, serde_json::to_string(p)?],
        )?;
    }
    t.commit()?;
    fs::create_dir_all(assets(path))?;
    open(path)
}
pub fn open(path: &Path) -> Result<Project> {
    if !path.is_file() {
        bail!("Book does not exist")
    };
    let c = connection(path)?;
    let get = |k: &str| -> Result<String> {
        Ok(c.query_row("SELECT value FROM meta WHERE key=?1", [k], |r| r.get(0))?)
    };
    let mut q = c.prepare("SELECT data FROM pages ORDER BY number")?;
    let mut pages = q
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|s| Ok(serde_json::from_str(&s?)?))
        .collect::<Result<Vec<Page>>>()?;
    for p in &mut pages {
        crate::safety::page(p)?;
        resolve_assets(path, p);
    }
    Ok(Project {
        path: path.to_string_lossy().into(),
        id: get("id")?,
        title: get("title")?,
        settings: serde_json::from_str(&get("settings")?)?,
        pages,
        omitted_page_ids: crate::library::get(&c)?.omitted_page_ids,
    })
}
pub fn page(path: &Path, id: &str) -> Result<Page> {
    let c = connection(path)?;
    let s: String = c
        .query_row("SELECT data FROM pages WHERE id=?1", [id], |r| r.get(0))
        .context("Page not found")?;
    let mut p = serde_json::from_str(&s)?;
    crate::safety::page(&p)?;
    resolve_assets(path, &mut p);
    Ok(p)
}
pub(crate) fn resolve_assets(project: &Path, page: &mut Page) {
    if let Some(c) = &mut page.cleanup
        && !Path::new(&c.path).exists()
        && let Some(name) = Path::new(&c.path).file_name()
    {
        let moved = assets(project).join(name);
        if moved.is_file() {
            c.path = moved.to_string_lossy().into();
        }
    }
    for value in [&mut page.rendered, &mut page.background] {
        if let Some(old) = value.as_ref()
            && !Path::new(old).exists()
            && let Some(name) = Path::new(old).file_name()
        {
            let moved = assets(project).join(name);
            if moved.is_file() {
                *value = Some(moved.to_string_lossy().into());
            }
        }
    }
}
pub fn save_page(path: &Path, p: &mut Page, expected: u64) -> Result<bool> {
    crate::safety::page(p)?;
    if !path.is_file() {
        bail!("Book no longer exists")
    }
    let c = connection(path)?;
    let mut next = p.clone();
    next.revision = expected.checked_add(1).context("Page revision overflow")?;
    let n = c.execute(
        "UPDATE pages SET revision=?1,data=?2,number=?5 WHERE id=?3 AND revision=?4",
        params![
            next.revision,
            serde_json::to_string(&next)?,
            p.id,
            expected,
            p.number
        ],
    )?;
    if n == 1 {
        *p = next;
    }
    Ok(n == 1)
}
pub fn settings(path: &Path, s: &TranslationSettings) -> Result<()> {
    connection(path)?.execute(
        "UPDATE meta SET value=?1 WHERE key='settings'",
        [serde_json::to_string(s)?],
    )?;
    Ok(())
}
/// A preparation reset and its resume marker must survive (or roll back) together.
pub fn save_checkpoint(path: &Path, page: &mut Page, expected: u64, job: &Job) -> Result<bool> {
    crate::safety::page(page)?;
    let mut c = connection(path)?;
    let tx = c.transaction()?;
    let mut next = page.clone();
    next.revision = expected.checked_add(1).context("Page revision overflow")?;
    anyhow::ensure!(
        job.page_revision == Some(next.revision),
        "Checkpoint revision mismatch"
    );
    let n = tx.execute(
        "UPDATE pages SET revision=?1,data=?2,number=?5 WHERE id=?3 AND revision=?4",
        params![
            next.revision,
            serde_json::to_string(&next)?,
            next.id,
            expected,
            next.number
        ],
    )?;
    if n != 1 {
        return Ok(false);
    }
    tx.execute(
        "INSERT OR REPLACE INTO jobs VALUES(?1,?2)",
        params![job.id, crate::job_validation::serialize(job)?],
    )?;
    tx.commit()?;
    *page = next;
    Ok(true)
}
/// Submission needs settings and page membership, not serialized page images/regions.
pub fn translation_settings(path: &Path) -> Result<TranslationSettings> {
    let json: String =
        connection(path)?.query_row("SELECT value FROM meta WHERE key='settings'", [], |r| {
            r.get(0)
        })?;
    Ok(serde_json::from_str(&json)?)
}
pub fn page_order(path: &Path) -> Result<Vec<(String, usize)>> {
    let c = connection(path)?;
    let mut q = c.prepare("SELECT id,number FROM pages ORDER BY number")?;
    Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}
pub fn preceding_context(
    path: &Path,
    number: usize,
    count: usize,
) -> Result<crate::prompts::SourceContext> {
    if count == 0 {
        return Ok(Default::default());
    }
    let c = connection(path)?;
    let mut q =
        c.prepare("SELECT number,data FROM pages WHERE number>=?1 AND number<?2 ORDER BY number")?;
    let pages = q.query_map(params![number.saturating_sub(count), number], |r| {
        Ok((r.get::<_, usize>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut context = crate::prompts::SourceContext::default();
    for row in pages {
        let (page_number, json) = row?;
        let page: Page = serde_json::from_str(&json)?;
        let sources: Vec<_> = page
            .regions
            .into_iter()
            .filter(|r| crate::safety::has_text(&r.source))
            .map(|r| r.source)
            .collect();
        if !sources.is_empty() {
            context.pages.push(crate::prompts::SourceContextPage {
                page_number,
                sources,
            });
        }
    }
    Ok(context)
}
pub fn cache_get(path: &Path, key: &str) -> Result<Option<Vec<TranslationItem>>> {
    let c = connection(path)?;
    let s: Option<String> = c
        .query_row("SELECT data FROM cache WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    s.map(|v| Ok(serde_json::from_str(&v)?)).transpose()
}
pub fn cache_validated(
    path: &Path,
    key: &str,
    expected: &[Region],
    text_only: bool,
) -> Result<Option<Vec<TranslationItem>>> {
    let c = connection(path)?;
    for lookup in [format!("validated-v1:{key}"), key.to_owned()] {
        let json: Option<String> = c
            .query_row("SELECT data FROM cache WHERE key=?1", [&lookup], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(json) = json else {
            continue;
        };
        let Ok(mut items) = serde_json::from_str::<Vec<TranslationItem>>(&json) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        if !items.iter().all(|i| {
            expected.iter().any(|r| r.id == i.id)
                && seen.insert(&i.id)
                && crate::safety::response_text(&i.source, &i.target).is_ok()
        }) {
            continue;
        }
        if text_only {
            items.retain(|i| {
                expected
                    .iter()
                    .any(|r| r.id == i.id && crate::safety::has_text(&r.source))
            });
            for i in &mut items {
                i.source = expected
                    .iter()
                    .find(|r| r.id == i.id)
                    .unwrap()
                    .source
                    .clone();
            }
        }
        items.retain(|i| !i.target.trim().is_empty());
        return Ok(Some(items));
    }
    Ok(None)
}
pub fn cache_put_validated(path: &Path, key: &str, items: &[TranslationItem]) -> Result<()> {
    cache_put(path, &format!("validated-v1:{key}"), items)
}
pub fn cache_put(path: &Path, key: &str, items: &[TranslationItem]) -> Result<()> {
    for item in items {
        crate::safety::response_text(&item.source, &item.target)?;
    }
    if !path.is_file() {
        bail!("Book no longer exists")
    }
    connection(path)?.execute(
        "INSERT OR REPLACE INTO cache VALUES(?1,?2)",
        params![key, serde_json::to_string(items)?],
    )?;
    Ok(())
}
pub fn save_job(path: &Path, j: &Job) -> Result<()> {
    save_jobs(path, std::slice::from_ref(j))
}
pub fn save_jobs(path: &Path, jobs: &[Job]) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    if !path.is_file() {
        bail!("Book no longer exists")
    }
    let mut c = connection(path)?;
    let tx = c.transaction()?;
    {
        let mut insert = tx.prepare_cached("INSERT OR REPLACE INTO jobs VALUES(?1,?2)")?;
        for j in jobs {
            let json = crate::job_validation::serialize(j)?;
            insert.execute(params![j.id, json])?;
        }
    }
    tx.commit()?;
    Ok(())
}
pub fn jobs(path: &Path) -> Result<Vec<Job>> {
    let c = connection(path)?;
    let rows = job_rows(&c)?;
    anyhow::ensure!(rows.invalid == 0, "{}", rows.errors.join("\n"));
    Ok(rows.jobs)
}
#[derive(Default)]
pub struct JobRows {
    pub jobs: Vec<Job>,
    pub errors: Vec<String>,
    pub invalid: usize,
}
pub fn job_rows(c: &Connection) -> Result<JobRows> {
    use std::collections::{HashMap, HashSet};
    let mut pages = HashMap::new();
    let mut query = c.prepare("SELECT id,revision,(SELECT json_group_array(json_extract(value,'$.id')) FROM json_each(pages.data,'$.regions')) FROM pages")?;
    for row in query.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, revision, regions) = row?;
        pages.insert(
            id,
            (revision, serde_json::from_str::<HashSet<String>>(&regions)?),
        );
    }
    let mut report = JobRows::default();
    let mut q = c.prepare("SELECT id,CASE WHEN length(CAST(data AS BLOB)) <= ?1 THEN data ELSE NULL END FROM jobs ORDER BY rowid DESC")?;
    for row in q.query_map([crate::job_validation::MAX_ROW_BYTES], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (id, json) = row?;
        let decoded = (|| -> Result<Job> {
            let json = json.as_deref().context("Job row exceeds size limit")?;
            let job: Job = serde_json::from_str(json)?;
            crate::job_validation::serialized_size(json, &job)?;
            let (revision, regions) = pages.get(&job.page_id).context("Job page is missing")?;
            crate::job_validation::validate(&id, &job, *revision, regions)?;
            Ok(job)
        })();
        match decoded {
            Ok(job) => report.jobs.push(job),
            Err(e) => {
                report.invalid += 1;
                if report.errors.len() < 100 {
                    report.errors.push(format!(
                        "Job {} was not loaded: {e}",
                        id.chars().take(64).collect::<String>()
                    ));
                }
            }
        }
    }
    if report.invalid > report.errors.len() {
        report.errors.push(format!(
            "{} additional invalid Job rows remain unchanged",
            report.invalid - report.errors.len()
        ));
    }
    Ok(report)
}
pub fn translated_page_ids(path: &Path) -> Result<std::collections::HashSet<String>> {
    let c = connection(path)?;
    eligible_page_ids(&c, false)
}
pub fn cleanup_page_ids(path: &Path) -> Result<std::collections::HashSet<String>> {
    let c = connection(path)?;
    eligible_page_ids(&c, true)
}
pub(crate) fn eligible_page_ids(
    c: &Connection,
    prepared: bool,
) -> Result<std::collections::HashSet<String>> {
    let mut q = c.prepare("SELECT pages.id,coalesce(json_extract(value,'$.target'),''),coalesce(json_extract(value,'$.prepared'),0) FROM pages,json_each(pages.data,'$.regions')")?;
    let mut ids = std::collections::HashSet::new();
    for row in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })? {
        let (id, target, ready) = row?;
        if crate::safety::has_text(&target) || (prepared && ready) {
            ids.insert(id);
        }
    }
    Ok(ids)
}
pub fn cache_key(
    p: &Page,
    s: &TranslationSettings,
    provider: &ProviderProfile,
    context: &str,
) -> Result<String> {
    let regions: Vec<_> = p
        .regions
        .iter()
        .map(|r| (&r.id, r.bbox, &r.source))
        .collect();
    let task = crate::instructions::translation_task(provider, s);
    let key = digest(&serde_json::to_vec(&(
        (
            crate::prompts::PROMPT_VERSION,
            crate::prompts::CONTEXT_VERSION,
        ),
        if provider.service == "llm" {
            Some(crate::instructions::effective(
                &provider.instructions,
                s,
                task,
            )?)
        } else {
            None
        },
        p.fingerprint.as_str(),
        regions,
        &s.source_language,
        &s.target_language,
        (&s.mode, s.auto_glossary),
        &s.ocr,
        s.glossary_enabled,
        if s.glossary_enabled {
            s.glossary.as_slice()
        } else {
            &[]
        },
        context,
        (
            &provider.service,
            &provider.protocol,
            &provider.endpoint,
            &provider.model,
            &provider.app_id,
            &provider.region,
            &s.deepl_glossary_id,
            provider.thinking,
            &provider.thinking_policy,
        ),
    ))?);
    Ok(
        if task == crate::instructions::Task::SimpleTextTranslation {
            format!(
                "simple-text-v{}:{key}",
                crate::instructions::SIMPLE_PROMPT_VERSION
            )
        } else if provider.service == "llm" && task == crate::instructions::Task::TextTranslation {
            format!(
                "llm-text-batch-v{}:{key}",
                crate::text_batches::STRUCTURED_CACHE_VERSION
            )
        } else {
            key
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revision_conflicts_preserve_manual_edits() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("日本語.umanga");
        let mut p = Page {
            id: uid(),
            number: 0,
            name: "test".into(),
            source: Source {
                path: "source.png".into(),
                kind: "image".into(),
                entry: None,
                index: 0,
            },
            width: 1,
            height: 1,
            pdf_points: None,
            source_stamp: String::new(),
            fingerprint: "abc".into(),
            revision: 0,
            regions: vec![],
            rendered: None,
            background: None,
            cleanup: None,
            status: "new".into(),
            error: None,
        };
        create(&path, "test", &[p.clone()]).unwrap();
        let mut late = p.clone();
        p.name = "manual".into();
        assert!(save_page(&path, &mut p, 0).unwrap());
        late.name = "old API result".into();
        assert!(!save_page(&path, &mut late, 0).unwrap());
        assert_eq!(page(&path, &p.id).unwrap().name, "manual");
    }
    #[test]
    fn night_filters_are_neutral() {
        let s = AppearanceSettings::default();
        assert_eq!(
            serde_json::to_value(s.day.filters).unwrap(),
            serde_json::to_value(s.night.filters).unwrap()
        );
    }
}
