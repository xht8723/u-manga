//! Transient book/chapter submission snapshots. No page is reset when previewing or queuing.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchMode {
    SkipTranslated,
    Replace,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageVersion {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPage {
    #[serde(flatten)]
    pub version: PageVersion,
    pub translated: bool,
    pub omitted: bool,
    pub has_work: bool,
    pub busy: bool,
    pub number: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPreview {
    pub book_id: String,
    pub book_title: String,
    pub chapter_title: Option<String>,
    pub pages: Vec<BatchPage>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOutcome {
    pub skipped: usize,
    pub omitted: usize,
    pub busy: usize,
    pub changed: usize,
    pub held: bool,
}
#[derive(Debug)]
pub struct BatchSubmission {
    pub jobs: Vec<Job>,
    pub outcome: BatchOutcome,
}

/// Read membership and lightweight page facts from one consistent snapshot. Only targets,
/// revisions and presence flags cross this query; no originals, renders or full pages load.
fn snapshot(path: &Path, chapter_id: Option<&str>) -> Result<BatchPreview> {
    if let Some(id) = chapter_id {
        crate::safety::identity(id)?;
    }
    let mut connection = store::connection(path)?;
    let tx = connection.transaction()?;
    let book = crate::library::get(&tx)?;
    let omitted: HashSet<_> = book.omitted_page_ids.iter().collect();
    let chapter = chapter_id
        .map(|id| {
            book.chapters
                .iter()
                .find(|c| c.id == id)
                .context("Chapter no longer exists")
        })
        .transpose()?;
    let wanted: HashSet<&String> = chapter.map_or_else(
        || book.chapters.iter().flat_map(|c| &c.page_ids).collect(),
        |c| c.page_ids.iter().collect(),
    );
    let mut query = tx.prepare(
        "SELECT id,revision,number,
          (SELECT json_group_array(json_extract(value,'$.target')) FROM json_each(pages.data,'$.regions')),
          coalesce(json_array_length(data,'$.regions')>0 OR json_extract(data,'$.rendered') IS NOT NULL
            OR json_extract(data,'$.cleanup') IS NOT NULL OR json_extract(data,'$.background') IS NOT NULL,0)
         FROM pages WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY number",
    )?;
    let mut pages = Vec::with_capacity(wanted.len());
    for row in query.query_map([serde_json::to_string(&wanted)?], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, usize>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
        ))
    })? {
        let (id, revision, number, targets, has_work) = row?;
        if !wanted.contains(&id) {
            continue;
        }
        crate::safety::identity(&id)?;
        let targets: Vec<Option<String>> = serde_json::from_str(&targets)?;
        pages.push(BatchPage {
            omitted: omitted.contains(&id),
            version: PageVersion { id, revision },
            translated: targets.iter().flatten().any(|t| !t.trim().is_empty()),
            has_work,
            busy: false,
            number,
        });
    }
    drop(query);
    let preview = BatchPreview {
        book_id: book.id,
        book_title: book.metadata.title,
        chapter_title: chapter.map(|c| c.title.clone()),
        pages,
    };
    tx.commit()?;
    Ok(preview)
}

impl Engine {
    pub fn preview_batch(&self, path: &str, chapter_id: Option<&str>) -> Result<BatchPreview> {
        self.recover(path)?;
        let _op = self.operations.lock();
        let mut preview = snapshot(Path::new(path), chapter_id)?;
        let state = self.state.lock();
        anyhow::ensure!(
            !state.blocked.contains(path),
            "Book organization is being applied"
        );
        for page in &mut preview.pages {
            let key = (path.to_owned(), page.version.id.clone());
            page.busy = state.ongoing.contains_key(&key) || state.editing.contains(&key);
        }
        Ok(preview)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_batch(
        &self,
        path: &str,
        book_id: &str,
        chapter_id: Option<&str>,
        pages: &[PageVersion],
        mode: BatchMode,
        provider: &ProviderProfile,
        settings: &TranslationSettings,
        directory: &Path,
    ) -> Result<BatchSubmission> {
        let provider = crate::instructions::capture(provider, settings)?;
        crate::safety::identity(book_id)?;
        let mut requested = HashMap::with_capacity(pages.len());
        for page in pages {
            crate::safety::identity(&page.id)?;
            anyhow::ensure!(
                page.revision < i64::MAX as u64,
                "Page revision exceeds storage limits"
            );
            anyhow::ensure!(
                requested.insert(&page.id, page.revision).is_none(),
                "Duplicate page in translation submission"
            );
        }
        self.recover(path)?;
        let _op = self.operations.lock();
        let current = snapshot(Path::new(path), chapter_id)?;
        anyhow::ensure!(
            current.book_id == book_id,
            "Book changed; reopen translation options"
        );
        let state = self.state.lock();
        anyhow::ensure!(
            !state.blocked.contains(path),
            "Book organization is being applied"
        );
        let mut outcome = BatchOutcome::default();
        let mut jobs = vec![];
        let mut positions = HashMap::new();
        let submission = super::scheduler::CapturedSubmission {
            path,
            provider: &provider,
            settings,
            kind: JobKind::Translation,
            directory,
        };
        for page in current.pages {
            let Some(expected) = requested.remove(&page.version.id) else {
                continue;
            };
            if expected != page.version.revision {
                outcome.changed += 1;
            } else if page.omitted {
                outcome.omitted += 1;
            } else if mode == BatchMode::SkipTranslated && page.translated {
                outcome.skipped += 1;
            } else if state
                .ongoing
                .contains_key(&(path.into(), page.version.id.clone()))
                || state
                    .editing
                    .contains(&(path.into(), page.version.id.clone()))
            {
                outcome.busy += 1;
            } else {
                positions.insert(page.version.id.clone(), page.number);
                jobs.push(self.captured_job(
                    &submission,
                    &page.version.id,
                    (mode == BatchMode::Replace).then_some(expected),
                ));
            }
        }
        // Deleted pages or pages moved outside the captured chapter are never replaced.
        outcome.changed += requested.len();
        drop(state);
        let jobs = self.publish_enqueued(path, jobs, &positions)?;
        outcome.held = self.is_held();
        Ok(BatchSubmission { jobs, outcome })
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn cancelling_worker_remains_busy_until_exit_and_is_not_replaced() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let image = tmp.path().join("original.png");
        image::RgbImage::new(32, 48).save(&image).unwrap();
        let pages = documents::import(&[image.to_string_lossy().into()]).unwrap();
        let project = store::create(&tmp.path().join("book.umanga"), "Book", &pages).unwrap();
        let engine = Engine::new(
            tmp.path().join("models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider access")),
        )
        .unwrap();
        engine
            .enqueue(
                &project.path,
                &[pages[0].id.clone()],
                &ProviderProfile::default(),
            )
            .unwrap();
        let (job, cancel) = engine.pick().unwrap();
        engine.control(&job.id, "cancel").unwrap();
        assert!(cancel.is_cancelled());
        let preview = engine.preview_batch(&project.path, None).unwrap();
        assert!(preview.pages[0].busy);
        let versions = preview
            .pages
            .iter()
            .map(|p| p.version.clone())
            .collect::<Vec<_>>();
        let result = engine
            .enqueue_batch(
                &project.path,
                &preview.book_id,
                None,
                &versions,
                BatchMode::Replace,
                &ProviderProfile::default(),
                &TranslationSettings::default(),
                &tmp.path().join("models"),
            )
            .unwrap();
        assert!(result.jobs.is_empty());
        assert_eq!(result.outcome.busy, 1);
        engine.finish(&job.id, &cancel, Err(anyhow::anyhow!("Cancelled")), 1);
        assert!(!engine.preview_batch(&project.path, None).unwrap().pages[0].busy);
    }
}
