use crate::{store, types::*};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub fn catalog() -> Result<Vec<ModelPack>> {
    Ok(serde_json::from_str(include_str!(
        "../../../assets/models.json"
    ))?)
}

#[derive(Clone, Debug)]
pub struct ModelLocations {
    pub bundled: PathBuf,
    pub downloaded: PathBuf,
}
impl ModelLocations {
    pub fn root(&self, pack: &ModelPack) -> &Path {
        if pack.distribution == "bundled" {
            &self.bundled
        } else {
            &self.downloaded
        }
    }
}

/// Avoid hashing large packs on each readiness refresh or queued page. File metadata
/// and the pinned manifest invalidate successful and unsuccessful cached checks.
#[derive(Clone, Default)]
pub struct VerificationCache {
    entries: std::sync::Arc<parking_lot::Mutex<HashMap<PathBuf, (String, bool)>>>,
    #[cfg(test)]
    checks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl VerificationCache {
    fn stamp(root: &Path, pack: &ModelPack) -> String {
        let mut stamp = pack.revision.clone();
        for file in &pack.files {
            stamp.push_str(&file.sha256);
            stamp.push_str(&format!(
                "{:?}",
                std::fs::metadata(root.join(&pack.id).join(&file.name))
                    .ok()
                    .map(|m| (m.len(), m.modified().ok()))
            ));
        }
        stamp
    }
    pub fn cached(&self, root: &Path, pack: &ModelPack) -> Option<bool> {
        if pack
            .files
            .iter()
            .any(|f| crate::safety::managed_model_path(root, &pack.id, &f.name).is_err())
        {
            return Some(false);
        }
        let stamp = Self::stamp(root, pack);
        self.entries
            .lock()
            .get(&root.join(&pack.id))
            .filter(|(s, _)| s == &stamp)
            .map(|(_, valid)| *valid)
    }
    pub fn check(&mut self, root: &Path, pack: &ModelPack, force: bool) -> Result<bool> {
        self.check_cancellable(root, pack, force, &CancellationToken::new())
    }
    pub fn check_cancellable(
        &mut self,
        root: &Path,
        pack: &ModelPack,
        force: bool,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        if root.as_os_str().is_empty() {
            return Ok(false);
        }
        let key = crate::safety::resolved(&root.join(&pack.id))?;
        let flight = verification_flight(&key)?;
        let _flight = loop {
            anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
            if let Some(lock) = flight.try_lock_for(std::time::Duration::from_millis(20)) {
                break lock;
            }
        };
        if !force && let Some(valid) = self.cached(root, pack) {
            return Ok(valid);
        }
        let before = Self::stamp(root, pack);
        #[cfg(test)]
        self.checks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let valid = verify_cancellable(root, pack, cancel)?;
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        let after = Self::stamp(root, pack);
        anyhow::ensure!(
            before == after,
            "Model files changed during verification; verify again"
        );
        let mut entries = self.entries.lock();
        if entries.len() >= 256 && !entries.contains_key(&root.join(&pack.id)) {
            entries.clear();
        }
        entries.insert(root.join(&pack.id), (after, valid));
        Ok(valid)
    }
}

type FlightRegistry = HashMap<PathBuf, std::sync::Arc<parking_lot::Mutex<()>>>;
fn verification_flight(key: &Path) -> Result<std::sync::Arc<parking_lot::Mutex<()>>> {
    static FLIGHTS: std::sync::LazyLock<parking_lot::Mutex<FlightRegistry>> =
        std::sync::LazyLock::new(Default::default);
    let mut flights = FLIGHTS.lock();
    if !flights.contains_key(key) && flights.len() >= 512 {
        flights.retain(|_, flight| std::sync::Arc::strong_count(flight) > 1);
    }
    anyhow::ensure!(
        flights.len() < 512 || flights.contains_key(key),
        "Too many active model checks; retry shortly"
    );
    Ok(flights.entry(key.to_owned()).or_default().clone())
}
pub fn verify(root: &Path, pack: &ModelPack) -> Result<bool> {
    verify_cancellable(root, pack, &CancellationToken::new())
}
pub fn verify_cancellable(
    root: &Path,
    pack: &ModelPack,
    cancel: &CancellationToken,
) -> Result<bool> {
    for f in &pack.files {
        let p = crate::safety::managed_model_path(root, &pack.id, &f.name)?;
        if !p.is_file()
            || std::fs::metadata(&p)?.len() != f.bytes
            || store::file_hash_cancellable(&p, cancel)? != f.sha256
        {
            return Ok(false);
        }
    }
    Ok(true)
}
async fn hash_async(path: PathBuf, cancel: CancellationToken) -> Result<String> {
    tokio::task::spawn_blocking(move || store::file_hash_cancellable(&path, &cancel)).await?
}
pub async fn download(
    root: &Path,
    pack: &ModelPack,
    cancel: &CancellationToken,
    progress: impl Fn(u64, u64),
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    for f in &pack.files {
        crate::safety::managed_model_path(root, &pack.id, &f.name)?;
    }
    let dir = root.join(&pack.id);
    tokio::fs::create_dir_all(&dir).await?;
    let total = pack.files.iter().map(|f| f.bytes).sum();
    let mut completed = 0;
    let mut last_progress = std::time::Instant::now();
    for f in &pack.files {
        let target = crate::safety::managed_model_path(root, &pack.id, &f.name)?;
        if target.exists() && hash_async(target.clone(), cancel.clone()).await? == f.sha256 {
            completed += f.bytes;
            progress(completed, total);
            continue;
        }
        let part = target.with_extension("part");
        let mut last_error = None;
        for attempt in 0..3 {
            if cancel.is_cancelled() {
                bail!("Download cancelled; partial files retained")
            }
            crate::safety::managed_model_path(
                root,
                &pack.id,
                part.file_name()
                    .and_then(|s| s.to_str())
                    .context("Invalid partial path")?,
            )?;
            let mut offset = tokio::fs::metadata(&part)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            if offset > f.bytes {
                tokio::fs::remove_file(&part).await?;
                offset = 0;
            }
            let result:Result<()>=async{let response=tokio::select!{_ = cancel.cancelled()=>bail!("Cancelled"),r=client.get(&f.url).header("Range",format!("bytes={offset}-")).send()=>r?};
    if response.status()==reqwest::StatusCode::RANGE_NOT_SATISFIABLE{if offset==f.bytes{return Ok(())}bail!("Invalid partial download length")}
    let response=response.error_for_status()?;let append=response.status()==reqwest::StatusCode::PARTIAL_CONTENT&&offset>0;
    if append{let range=response.headers().get("content-range").context("Missing Content-Range")?.to_str()?;if !range.starts_with(&format!("bytes {offset}-")){bail!("Invalid resume range")}}
    crate::safety::managed_model_path(root, &pack.id, part.file_name().and_then(|s| s.to_str()).context("Invalid partial path")?)?;
    let mut file=tokio::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part).await?;let mut received=if append{offset}else{0};let mut stream=response.bytes_stream();
    loop{let chunk=tokio::select!{_ = cancel.cancelled()=>bail!("Cancelled"),v=stream.next()=>v};match chunk{Some(c)=>{let c=c?;received+=c.len()as u64;if received>f.bytes{bail!("Download exceeds declared size")};file.write_all(&c).await?;if last_progress.elapsed() >= std::time::Duration::from_millis(80) { progress(completed+received,total);last_progress=std::time::Instant::now(); }},None=>break}}
    file.flush().await?;file.sync_all().await?;Ok(())}.await;
            match result {
                Ok(()) => {
                    if tokio::fs::metadata(&part).await?.len() == f.bytes
                        && hash_async(part.clone(), cancel.clone()).await? == f.sha256
                    {
                        crate::safety::managed_model_path(root, &pack.id, &f.name)?;
                        crate::safety::managed_model_path(
                            root,
                            &pack.id,
                            part.file_name()
                                .and_then(|s| s.to_str())
                                .context("Invalid partial path")?,
                        )?;
                        let source = part.clone();
                        let destination = target.clone();
                        tokio::task::spawn_blocking(move || {
                            store::replace_file(&source, &destination)
                        })
                        .await??;
                        last_error = None;
                        break;
                    } else {
                        tokio::fs::remove_file(&part).await?;
                        last_error = Some(anyhow::anyhow!("Checkpoint checksum mismatch"))
                    }
                }
                Err(e) => last_error = Some(e),
            }
            if cancel.is_cancelled() {
                bail!("Download cancelled; partial files retained")
            };
            if attempt < 2 {
                tokio::select! {_ = cancel.cancelled()=>bail!("Cancelled"),_ = tokio::time::sleep(std::time::Duration::from_secs(1<<attempt))=>{}}
            }
        }
        if let Some(e) = last_error {
            return Err(e);
        }
        completed += f.bytes;
        progress(completed, total);
    }
    Ok(())
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    #[test]
    fn matching_checks_share_one_hash_and_cancelled_checks_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let mut pack = catalog().unwrap().remove(0);
        pack.id = "test-pack".into();
        pack.files.truncate(1);
        pack.files[0].name = "test.bin".into();
        let file = dir.path().join(&pack.id).join(&pack.files[0].name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let bytes = b"bounded model verification fixture";
        std::fs::write(&file, bytes).unwrap();
        pack.files[0].bytes = bytes.len() as u64;
        pack.files[0].sha256 = store::digest(bytes);
        let cache = VerificationCache::default();
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    let mut c = cache.clone();
                    let p = &pack;
                    let root = dir.path();
                    let start = &start;
                    scope.spawn(move || {
                        start.wait();
                        assert!(c.check(root, p, false).unwrap());
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
        });
        assert_eq!(cache.checks.load(std::sync::atomic::Ordering::SeqCst), 1);
        let token = CancellationToken::new();
        token.cancel();
        let mut other = VerificationCache::default();
        assert!(
            other
                .check_cancellable(dir.path(), &pack, false, &token)
                .is_err()
        );
        assert!(other.cached(dir.path(), &pack).is_none());
        std::fs::write(&file, b"different length").unwrap();
        assert!(cache.cached(dir.path(), &pack).is_none());
        assert!(!other.check(dir.path(), &pack, false).unwrap());
    }
}
