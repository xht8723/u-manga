//! Journaled library activation and retirement, isolated from book commands.
use super::library_api::root;
use super::*;
use umanga_core::library;
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Relocation {
    version: u32,
    source: PathBuf,
    destination: PathBuf,
    books: Vec<String>,
    protected: std::collections::HashSet<PathBuf>,
    fingerprints: HashMap<String, String>,
    copies: HashMap<String, (String, String)>,
    committed: bool,
}
impl Relocation {
    async fn verify_for_activation(self) -> anyhow::Result<Self> {
        tauri::async_runtime::spawn_blocking(move || {
            self.verify_retirement()?;
            Ok(self)
        })
        .await?
    }
    fn resume(&mut self, current: &Path, target: &Path) -> Api<()> {
        if self.version != 1 || self.destination != target {
            return Err("An interrupted library move exists. Select its destination again to finish it before starting another move.".into());
        }
        if current == self.destination {
            self.committed = true;
        }
        if current
            != if self.committed {
                &self.destination
            } else {
                &self.source
            }
        {
            return Err("The interrupted move belongs to another library. Reopen that library before retrying.".into());
        }
        Ok(())
    }
    fn verify_retirement(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.copies.len() == self.books.len(),
            "Incomplete relocation inventory; old copies were preserved"
        );
        for source in &self.books {
            let (path, id) = self
                .copies
                .get(source)
                .ok_or_else(|| anyhow::anyhow!("Missing relocation copy"))?;
            let path = umanga_core::safety::resolved(Path::new(path))?;
            anyhow::ensure!(
                path.parent() == Some(self.destination.as_path()),
                "Invalid copied book location"
            );
            let b = library::validate_book(&path)?;
            anyhow::ensure!(
                &b.id == id,
                "Copied book identity changed; old copies were preserved"
            );
            // A valid database is insufficient if the only generated pixels still live
            // beside the source. Check current references; post-activation edits are valid.
            let project = store::open(&path)?;
            let referenced = project
                .pages
                .iter()
                .flat_map(|page| {
                    [
                        page.rendered.as_ref(),
                        page.background.as_ref(),
                        page.cleanup.as_ref().map(|c| &c.path),
                    ]
                    .into_iter()
                    .flatten()
                })
                .chain(b.cover.iter());
            for reference in referenced {
                let name = Path::new(reference)
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("Invalid copied asset reference"))?;
                let copied = store::assets(&path).join(name);
                umanga_core::safety::no_link(&copied)?;
                anyhow::ensure!(
                    copied.is_file(),
                    "A copied book asset is missing; old copies were preserved"
                );
            }
        }
        Ok(())
    }
    fn protect_retirement(&mut self) -> anyhow::Result<()> {
        self.verify_retirement()?;
        self.protected
            .extend(library::linked_originals(&self.source)?);
        self.protected
            .extend(library::linked_originals(&self.destination)?);
        Ok(())
    }
}
fn relocation_fingerprint(path: &Path) -> anyhow::Result<String> {
    Ok(store::digest(&serde_json::to_vec(&(
        library::open(path)?,
        store::open(path)?,
        store::jobs(path)?,
    ))?))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn journal() -> Relocation {
        Relocation {
            version: 1,
            source: "old".into(),
            destination: "new".into(),
            books: vec![],
            protected: Default::default(),
            fingerprints: Default::default(),
            copies: Default::default(),
            committed: false,
        }
    }
    #[test]
    fn crash_after_preferences_commit_resumes_retirement() {
        let mut j = journal();
        j.resume(Path::new("new"), Path::new("new")).unwrap();
        assert!(j.committed);
        assert!(j.resume(Path::new("old"), Path::new("new")).is_err());
    }
    #[test]
    fn incomplete_copy_and_unrelated_activation_cannot_retire_originals() {
        let mut j = journal();
        assert!(j.resume(Path::new("unrelated"), Path::new("new")).is_err());
        assert!(!j.committed);
        j.books.push("old/book.umanga".into());
        assert!(j.verify_retirement().is_err());
    }
    #[tokio::test]
    async fn retirement_rejects_incomplete_copies_but_accepts_consistent_edits() {
        let t = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test-output")
            .join(format!("retirement-{}", uid()));
        let old = t.join("old");
        let new = t.join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let original = t.join("source.png");
        image::RgbImage::new(20, 30).save(&original).unwrap();
        let page = documents::import(&[original.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0);
        let source = store::create(&old.join("source.umanga"), "fixture", &[page]).unwrap();
        let copy = library::import_book(&new, Path::new(&source.path)).unwrap();
        let mut j = journal();
        j.source = old.canonicalize().unwrap();
        j.destination = new.canonicalize().unwrap();
        j.books = vec![source.path.clone()];
        j.copies
            .insert(source.path.clone(), (copy.path.clone(), copy.id.clone()));
        assert!(j.verify_retirement().is_ok());
        let copy_path = Path::new(&copy.path);
        let original_settings: String = store::connection(copy_path)
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .unwrap();
        store::connection(copy_path)
            .unwrap()
            .execute("UPDATE meta SET value='broken' WHERE key='settings'", [])
            .unwrap();
        assert!(j.verify_retirement().is_err());
        store::connection(copy_path)
            .unwrap()
            .execute(
                "UPDATE meta SET value=?1 WHERE key='settings'",
                [original_settings],
            )
            .unwrap();
        let mut copied_page = store::open(copy_path).unwrap().pages.remove(0);
        copied_page.rendered = Some(
            store::assets(copy_path)
                .join("missing.png")
                .to_string_lossy()
                .into(),
        );
        store::save_page(copy_path, &mut copied_page, 0).unwrap();
        assert!(
            j.verify_retirement().is_err(),
            "Missing generated pixels must not retire their source copy"
        );
        assert!(
            j.clone().verify_for_activation().await.is_err(),
            "A resumed copy missing generated pixels must fail before preference activation"
        );
        assert!(!j.committed);
        j.resume(&old.canonicalize().unwrap(), &new.canonicalize().unwrap())
            .unwrap();
        assert!(
            !j.committed,
            "The source remains the recoverable active library"
        );
        image::RgbImage::new(20, 30)
            .save(copied_page.rendered.as_ref().unwrap())
            .unwrap();
        assert!(j.verify_retirement().is_ok());
        assert!(j.clone().verify_for_activation().await.is_ok());
        store::connection(Path::new(&copy.path))
            .unwrap()
            .execute("DELETE FROM pages", [])
            .unwrap();
        assert!(j.verify_retirement().is_err());
        let mut corrected = library::open(Path::new(&copy.path)).unwrap();
        corrected.chapters.clear();
        store::connection(Path::new(&copy.path))
            .unwrap()
            .execute(
                "UPDATE meta SET value=?1 WHERE key='book'",
                [serde_json::to_string(&corrected).unwrap()],
            )
            .unwrap();
        assert!(
            j.verify_retirement().is_ok(),
            "A consistent edit after activation is valid"
        );
        assert_eq!(store::open(Path::new(&source.path)).unwrap().pages.len(), 1);
        // New destination books may link old owned images after an interrupted activation.
        let late = store::assets(Path::new(&source.path)).join("late-original.png");
        image::RgbImage::new(10, 10).save(&late).unwrap();
        let hash = store::file_hash(&late).unwrap();
        let preview = library::scan(
            &[late.to_string_lossy().into_owned()],
            true,
            &CancellationToken::new(),
        )
        .unwrap();
        library::create(
            &new,
            library::Metadata {
                title: "Late link".into(),
                ..Default::default()
            },
            preview,
            None,
        )
        .unwrap();
        j.protect_retirement().unwrap();
        assert!(j.protected.contains(&late.canonicalize().unwrap()));
        library::delete_protected(&old, Path::new(&source.path), &j.protected).unwrap();
        assert_eq!(store::file_hash(&late).unwrap(), hash);
    }
}
#[tauri::command]
pub async fn library_relocate(state: State<'_, AppState>, destination: String) -> Api<AppSettings> {
    let _storage = state.storage_change.lock().await;
    if !state.downloads.lock().is_empty() {
        return Err("Finish or cancel downloads and scans before moving the library.".into());
    }
    state.jobs_hub.wait_recovery().await;
    let current = root(&state);
    let target = umanga_core::safety::resolved(Path::new(&destination)).map_err(error)?;
    let journal_path = state.folder.join("relocation.json");
    let mut existing = match std::fs::read(&journal_path) {
        Ok(bytes) => Some(serde_json::from_slice::<Relocation>(&bytes).map_err(|e| {
            format!("Cannot read relocation journal; preserve it for recovery: {e}")
        })?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(error(e)),
    };
    if existing.as_ref().is_some_and(|j| {
        !j.committed
            && j.destination != target
            && umanga_core::safety::resolved(&current).ok().as_ref() == Some(&j.source)
    }) {
        // Keep abandoned copies and their journal; never infer that user files may be removed.
        std::fs::rename(
            &journal_path,
            state
                .folder
                .join(format!("relocation-abandoned-{}.json", uid())),
        )
        .map_err(error)?;
        existing = None;
    }
    let mut journal = if let Some(mut j) = existing {
        // Preferences are the durable activation commit. A crash immediately after that
        // write must resume retirement, not copy over the now-active library.
        j.resume(
            &umanga_core::safety::resolved(&current).map_err(error)?,
            &target,
        )?;
        j
    } else {
        umanga_core::safety::separate(&current, &target).map_err(error)?;
        if target.is_dir() && std::fs::read_dir(&target).map_err(error)?.next().is_some() {
            return Err("Choose an empty destination folder".into());
        }
        let source = current.canonicalize().map_err(error)?;
        let old = source.clone();
        let (books, protected) =
            tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
                Ok((library::list(&old)?, library::linked_originals(&old)?))
            })
            .await
            .map_err(error)?
            .map_err(error)?;
        Relocation {
            version: 1,
            source,
            destination: target.clone(),
            books: books.into_iter().map(|b| b.path).collect(),
            protected,
            fingerprints: HashMap::new(),
            copies: HashMap::new(),
            committed: false,
        }
    };
    umanga_core::safety::separate(&journal.source, &journal.destination).map_err(error)?;
    let mut guards = Vec::new();
    if !journal.committed {
        for path in &journal.books {
            guards.push(state.engine.reserve_book(path).await.map_err(error)?);
        }
        let files = journal.books.clone();
        let fingerprints = tauri::async_runtime::spawn_blocking(
            move || -> anyhow::Result<HashMap<String, String>> {
                files
                    .into_iter()
                    .map(|p| Ok((p.clone(), relocation_fingerprint(Path::new(&p))?)))
                    .collect()
            },
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        if !journal.fingerprints.is_empty() && journal.fingerprints != fingerprints {
            return Err("Source books changed after the interrupted move. Copies were preserved; select a new empty destination to restart the move.".into());
        }
        journal.fingerprints = fingerprints;
        store::atomic_write(
            &journal_path,
            &serde_json::to_vec_pretty(&journal).map_err(error)?,
        )
        .map_err(error)?;
        let sources = journal.books.clone();
        let old = journal.source.clone();
        let new = target.clone();
        journal.copies = tauri::async_runtime::spawn_blocking(
            move || -> anyhow::Result<HashMap<String, (String, String)>> {
                library::copy_model_storage(&old, &new)?;
                let mut copies = HashMap::new();
                for source in sources {
                    let copied = library::import_book(&new, Path::new(&source))?;
                    library::relocate_job_storage(Path::new(&copied.path), &old, &new)?;
                    library::validate_book(Path::new(&copied.path))?;
                    copies.insert(source, (copied.path, copied.id));
                }
                Ok(copies)
            },
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        store::atomic_write(
            &journal_path,
            &serde_json::to_vec_pretty(&journal).map_err(error)?,
        )
        .map_err(error)?;
        // Reused copies can have valid databases but missing generated assets. Verify
        // before the durable preference commit, while the source is still active.
        journal = journal.verify_for_activation().await.map_err(error)?;
        let mut next = state.settings.lock().clone();
        next.library_directory = target.to_string_lossy().into();
        persist_preferences(&state, &next).await?;
        state.host.stop();
        journal.committed = true;
        // The pre-commit journal plus committed preferences are sufficient for recovery.
        // Do not strand the active library if updating this redundant marker fails.
        if let Err(e) = store::atomic_write(
            &journal_path,
            &serde_json::to_vec_pretty(&journal).map_err(error)?,
        ) {
            eprintln!("Relocation marker deferred: {e}");
        }
    }
    state.host.stop();
    state
        .engine
        .inference
        .lock()
        .set_root(target.join("models"));
    *state.engine.model_directory.lock() = target.join("models");
    for path in &journal.books {
        state.engine.release_book(path, None);
    }
    drop(guards);
    // Activate the usable copy before best-effort retirement; a locked old file cannot strand Jobs.
    state
        .jobs_hub
        .activate(target.clone(), state.engine.clone());
    let old = journal.source.clone();
    let books = journal.books.clone();
    let retirement_journal = journal_path.clone();
    let errors = tauri::async_runtime::spawn_blocking(move || {
        let prepare = (|| -> anyhow::Result<_> {
            journal.protect_retirement()?;
            // Freeze the union before the first deletion; retries retain protection too.
            store::atomic_write(&retirement_journal, &serde_json::to_vec_pretty(&journal)?)?;
            Ok(())
        })();
        match prepare {
            Ok(()) => (),
            Err(e) => {
                return vec![format!(
                    "Destination verification failed; originals and old copies preserved: {e}"
                )];
            }
        };
        let mut errors = Vec::new();
        for path in books {
            let (_, id) = &journal.copies[&path]; // validated complete inventory before deletion
            if let Err(e) = library::retire_copy(&old, Path::new(&path), id, &journal.protected) {
                errors.push(format!("{path}: {e}"));
            }
        }
        errors
    })
    .await
    .map_err(error)?;
    if !errors.is_empty() {
        return Err(format!(
            "Library moved. Some old copies could not be retired; select the same destination in Move library to retry. {}",
            errors.join("; ")
        ).into());
    }
    std::fs::remove_file(journal_path).map_err(error)?;
    Ok(state.settings.lock().clone())
}
