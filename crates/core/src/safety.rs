//! Boundaries for untrusted book data and application-owned filesystem operations.
use crate::types::*;
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

pub fn identity(id: &str) -> Result<()> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id),
        "Invalid book/page identity"
    );
    Ok(())
}
pub fn page(p: &Page) -> Result<()> {
    identity(&p.id)?;
    ensure!(
        p.revision < i64::MAX as u64,
        "Page revision exceeds storage limits"
    );
    ensure!(
        p.width > 0 && p.height > 0 && p.width as u64 * p.height as u64 <= 100_000_000,
        "Invalid page dimensions"
    );
    ensure!(p.regions.len() <= 4096, "Too many page regions");
    ensure!(
        p.pdf_points.is_none_or(|points| points
            .iter()
            .all(|v| v.is_finite() && *v > 0. && *v <= 100_000.)),
        "Invalid PDF page size"
    );
    let mut ids = HashSet::new();
    for r in &p.regions {
        ensure!(
            !r.id.is_empty() && r.id.len() <= 256 && ids.insert(&r.id),
            "Invalid or duplicate region identity"
        );
        region(r)?;
    }
    Ok(())
}
pub const MAX_SOURCE_CHARS: usize = 16384;
pub const MAX_TARGET_CHARS: usize = 4096;
/// SQLite trim removes ASCII spaces only; all native eligibility uses Unicode whitespace.
pub fn has_text(text: &str) -> bool {
    !text.trim().is_empty()
}
pub fn response_text(source: &str, target: &str) -> Result<()> {
    ensure!(
        source.chars().count() <= MAX_SOURCE_CHARS && target.chars().count() <= MAX_TARGET_CHARS,
        "Region text exceeds render budget"
    );
    Ok(())
}
pub fn region(r: &Region) -> Result<()> {
    ensure!(r.score.is_finite(), "Invalid region confidence");
    ensure!(
        r.style.font.as_ref().is_none_or(|name| name.len() <= 4096),
        "Font selection exceeds input limits"
    );
    ensure!(
        r.bubble.is_none_or(
            |bounds| bounds.iter().all(|v| v.is_finite() && v.abs() <= 100_000.)
                && bounds[2] >= bounds[0]
                && bounds[3] >= bounds[1]
        ),
        "Invalid balloon bounds"
    );
    ensure!(
        r.bbox.iter().all(|v| v.is_finite() && v.abs() <= 100_000.)
            && r.bbox[2] >= r.bbox[0]
            && r.bbox[3] >= r.bbox[1],
        "Invalid region bounds"
    );
    ensure!(
        (r.bbox[2] - r.bbox[0]) as f64 * (r.bbox[3] - r.bbox[1]) as f64 <= 25_000_000.,
        "Region exceeds render budget"
    );
    ensure!(
        r.target.chars().count() <= MAX_TARGET_CHARS
            && r.source.chars().count() <= MAX_SOURCE_CHARS,
        "Region text exceeds render budget"
    );
    ensure!(
        r.style
            .size
            .is_none_or(|s| s.is_finite() && (1.0..=1000.0).contains(&s))
            && r.style.line_gap.is_finite()
            && (0.0..=10.0).contains(&r.style.line_gap)
            && r.style.outline_width_percent.is_finite()
            && (1.0..=20.0).contains(&r.style.outline_width_percent)
            && r.style.outline_color.len() == 7
            && r.style.outline_color.starts_with('#')
            && r.style.outline_color.as_bytes()[1..]
                .iter()
                .all(u8::is_ascii_hexdigit),
        "Invalid typography"
    );
    Ok(())
}
pub fn resolved(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(part),
        }
    }
    if normalized.exists() {
        return Ok(normalized.canonicalize()?);
    }
    let name = normalized.file_name().context("Invalid path")?;
    Ok(resolved(normalized.parent().context("Invalid parent")?)?.join(name))
}
pub fn no_link(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        !m.file_type().is_symlink(),
        "Linked paths are not allowed in owned storage: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            m.file_attributes() & 0x400 == 0,
            "Reparse points are not allowed in owned storage: {}",
            path.display()
        );
    }
    Ok(())
}
pub fn separate(from: &Path, to: &Path) -> Result<()> {
    let a = resolved(from)?;
    let b = resolved(to)?;
    ensure!(
        !a.starts_with(&b) && !b.starts_with(&a),
        "Choose a destination outside the source library and its assets"
    );
    Ok(())
}
pub fn asset(book: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        Path::new(name).components().count() == 1
            && matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            )
            && !name.contains(['/', '\\', ':']),
        "Invalid asset filename"
    );
    let root = crate::store::assets(book);
    if root.exists() {
        no_link(&root)?;
    }
    let path = root.join(name);
    if path.exists() {
        no_link(&path)?;
    }
    ensure!(
        resolved(&path)?.parent() == Some(resolved(&root)?.as_path()),
        "Asset escapes owned directory"
    );
    Ok(path)
}
pub fn page_asset(book: &Path, id: &str, suffix: &str) -> Result<PathBuf> {
    identity(id)?;
    asset(book, &format!("{id}-{suffix}"))
}

#[derive(Default)]
pub struct ProtectedPaths {
    pub files: HashSet<PathBuf>,
    pub directories: Vec<PathBuf>,
}
impl ProtectedPaths {
    pub fn check(&self, path: &Path) -> Result<()> {
        let path = resolved(path)?;
        ensure!(
            !self.files.contains(&path) && !self.directories.iter().any(|d| path.starts_with(d)),
            "Export cannot overwrite an original, book database, or managed working asset"
        );
        Ok(())
    }
}

/// Compare canonical aliases without treating Windows spelling/case as ownership.
pub fn same_path(a: &Path, b: &Path) -> Result<bool> {
    let a = resolved(a)?;
    let b = resolved(b)?;
    #[cfg(windows)]
    {
        Ok(a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()))
    }
    #[cfg(not(windows))]
    {
        Ok(a == b)
    }
}

/// Conservative aggregate CPU scratch bound, excluding model session weights.
/// Cleanup may inspect an entire connected balloon, hence full-page mask bounds.
pub fn render_budget(p: &Page) -> Result<()> {
    page(p)?;
    let pixels = u64::from(p.width)
        .checked_mul(u64::from(p.height))
        .context("Render size overflow")?;
    let masks = pixels
        .checked_mul(p.regions.len() as u64)
        .and_then(|v| v.checked_mul(2))
        .context("Mask size overflow")?;
    let largest = p
        .regions
        .iter()
        .map(|r| {
            ((r.bbox[2] - r.bbox[0]).ceil().max(1.) as u64)
                .saturating_mul((r.bbox[3] - r.bbox[1]).ceil().max(1.) as u64)
        })
        .max()
        .unwrap_or(0);
    let bytes = pixels
        .checked_mul(96)
        .and_then(|v| v.checked_add(masks))
        .and_then(|v| v.checked_add(largest.saturating_mul(4)))
        .context("Render size overflow")?;
    ensure!(
        bytes <= 1024 * 1024 * 1024,
        "Page exceeds aggregate render memory budget; split the page or reduce overlapping regions"
    );
    let spool = p
        .regions
        .iter()
        .try_fold(0u64, |sum, r| {
            sum.checked_add(
                ((r.bbox[2] - r.bbox[0]).ceil().max(1.) as u64)
                    .saturating_mul((r.bbox[3] - r.bbox[1]).ceil().max(1.) as u64)
                    .saturating_mul(4),
            )
        })
        .context("Lettering size overflow")?;
    ensure!(
        spool <= 512 * 1024 * 1024,
        "Page exceeds aggregate lettering budget"
    );
    Ok(())
}
/// Validate every existing ancestor before managed model I/O, including aliases.
pub fn managed_model_path(root: &Path, pack: &str, name: &str) -> Result<PathBuf> {
    for part in [pack, name] {
        ensure!(
            !part.is_empty()
                && !part.contains(['/', '\\', ':'])
                && matches!(
                    Path::new(part).components().next(),
                    Some(Component::Normal(_))
                )
                && Path::new(part).components().count() == 1,
            "Invalid model path"
        );
    }
    let path = root.join(pack).join(name);
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor).is_ok() {
            no_link(ancestor)?;
        }
    }
    ensure!(
        resolved(&path)?.starts_with(resolved(root)?),
        "Model path escapes managed storage"
    );
    Ok(path)
}
