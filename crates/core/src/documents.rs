use crate::{store, types::*};
use anyhow::{Context, Result, bail};
use image::DynamicImage;
use pdfium_render::prelude::*;
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};
type PdfWork = Box<dyn FnOnce(&Pdfium) + Send>;
static PDFIUM: OnceLock<std::sync::mpsc::Sender<PdfWork>> = OnceLock::new();
pub fn init_pdfium(directory: &Path) -> Result<()> {
    if PDFIUM.get().is_some() {
        return Ok(());
    }
    let directory = directory.to_owned();
    let (tx, rx) = std::sync::mpsc::channel::<PdfWork>();
    let (ready, wait) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("umanga-pdf".into())
        .spawn(move || {
            let binding =
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&directory));
            match binding {
                Ok(b) => {
                    let pdf = Pdfium::new(b);
                    let _ = ready.send(Ok(()));
                    for work in rx {
                        work(&pdf)
                    }
                }
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                }
            }
        })?;
    wait.recv()?.map_err(anyhow::Error::msg)?;
    let _ = PDFIUM.set(tx);
    Ok(())
}
fn with_pdf<T: Send + 'static>(
    work: impl FnOnce(&Pdfium) -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    PDFIUM
        .get()
        .context("PDF runtime not initialized")?
        .send(Box::new(move |pdf| {
            let _ = tx.send(work(pdf));
        }))
        .map_err(|_| anyhow::anyhow!("PDF worker stopped"))?;
    rx.recv()?
}
pub fn image_ext(s: &str) -> bool {
    matches!(
        s.rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "webp"
    )
}
pub fn natural(s: &str) -> String {
    let mut out = String::new();
    let mut num = String::new();
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_digit() {
            num.push(ch)
        } else {
            if !num.is_empty() {
                out += &format!("{:0>16}", num);
                num.clear();
            }
            out.push(ch)
        }
    }
    if !num.is_empty() {
        out += &format!("{:0>16}", num)
    }
    out
}
pub fn import(paths: &[String]) -> Result<Vec<Page>> {
    import_cancellable(paths, None)
}
pub fn import_cancellable(
    paths: &[String],
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Vec<Page>> {
    let check = || -> Result<()> {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            bail!("Scan cancelled")
        }
        Ok(())
    };
    let mut sources = Vec::new();
    for value in paths {
        check()?;
        let p = Path::new(value)
            .canonicalize()
            .context("Source is missing")?;
        if p.is_dir() {
            let mut children = fs::read_dir(p)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && image_ext(&p.to_string_lossy()))
                .collect::<Vec<_>>();
            children.sort_by_key(|p| natural(&p.to_string_lossy()));
            for p in children {
                sources.push(Source {
                    path: p.to_string_lossy().into(),
                    kind: "image".into(),
                    entry: None,
                    index: 0,
                })
            }
        } else if image_ext(value) {
            sources.push(Source {
                path: p.to_string_lossy().into(),
                kind: "image".into(),
                entry: None,
                index: 0,
            })
        } else {
            match p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "cbz" | "zip" => {
                    let mut zip = zip::ZipArchive::new(fs::File::open(&p)?)?;
                    let mut names = Vec::new();
                    for i in 0..zip.len() {
                        check()?;
                        let f = zip.by_index(i)?;
                        if !f.is_dir() && image_ext(f.name()) && f.enclosed_name().is_some() {
                            if f.size() > 256 * 1024 * 1024 {
                                bail!("Archive image is too large")
                            };
                            names.push(f.name().to_owned());
                        }
                    }
                    names.sort_by_key(|n| natural(n));
                    for n in names {
                        sources.push(Source {
                            path: p.to_string_lossy().into(),
                            kind: "archive".into(),
                            entry: Some(n),
                            index: 0,
                        })
                    }
                }
                "pdf" => {
                    let input = p.clone();
                    let count = with_pdf(move |pdf| {
                        Ok(pdf
                            .load_pdf_from_file(&input, None)
                            .context(
                                "Cannot open PDF; encrypted PDFs must be unlocked before import",
                            )?
                            .pages()
                            .len())
                    })?;
                    for i in 0..count {
                        sources.push(Source {
                            path: p.to_string_lossy().into(),
                            kind: "pdf".into(),
                            entry: None,
                            index: i,
                        })
                    }
                }
                _ => bail!("Unsupported source: {}", p.display()),
            }
        }
    }
    if sources.is_empty() {
        bail!("No supported pages found")
    };
    sources
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            check()?;
            let (width, height) = dimensions(&s)?;
            let pdf_points = if s.kind == "pdf" {
                Some(pdf_points(&s)?)
            } else {
                None
            };
            let source_stamp = source_stamp(&s)?;
            let name = s.entry.clone().unwrap_or_else(|| {
                Path::new(&s.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            });
            Ok(Page {
                id: uid(),
                number: i,
                name,
                source: s,
                width,
                height,
                pdf_points,
                source_stamp,
                fingerprint: String::new(),
                revision: 0,
                regions: vec![],
                rendered: None,
                background: None,
                cleanup: None,
                status: "new".into(),
                error: None,
            })
        })
        .collect()
}
pub fn source_bytes(s: &Source) -> Result<Vec<u8>> {
    if s.kind == "archive" {
        let mut z = zip::ZipArchive::new(fs::File::open(&s.path)?)?;
        let f = z.by_name(s.entry.as_deref().context("Missing archive entry")?)?;
        let size = f.size();
        crate::image_input::encoded(f, size)
    } else {
        let f = fs::File::open(&s.path)
            .context("Original source missing; restore it to its linked location")?;
        let size = f.metadata()?.len();
        crate::image_input::encoded(f, size)
    }
}
pub fn source_stamp(s: &Source) -> Result<String> {
    let m = fs::metadata(&s.path).context("Original source missing; restore it to its linked location or add a replacement through Manage chapters")?;
    Ok(format!(
        "{}:{}",
        m.len(),
        m.modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ))
}
pub fn source_health(p: &Page) -> Result<()> {
    let stamp = source_stamp(&p.source)?;
    if stamp != p.source_stamp {
        bail!("Original source changed; restore the matching original or import it as a new book");
    }
    Ok(())
}
pub fn dimensions(s: &Source) -> Result<(u32, u32)> {
    if s.kind == "pdf" {
        let s = s.clone();
        with_pdf(move |pdf| {
            let d = pdf.load_pdf_from_file(&s.path, None)?;
            let p = d.pages().get(s.index)?;
            let dimensions = pdf_dimensions(p.width().value, p.height().value)?;
            Ok(dimensions)
        })
    } else {
        if s.kind == "archive" {
            crate::image_input::bytes_dimensions(source_bytes(s)?)
        } else {
            crate::image_input::dimensions(Path::new(&s.path))
        }
    }
}
pub fn pdf_points(s: &Source) -> Result<[f32; 2]> {
    let s = s.clone();
    with_pdf(move |pdf| {
        let d = pdf.load_pdf_from_file(&s.path, None)?;
        let p = d.pages().get(s.index)?;
        Ok([p.width().value, p.height().value])
    })
}
pub fn load(s: &Source) -> Result<DynamicImage> {
    let image = if s.kind == "pdf" {
        let s = s.clone();
        with_pdf(move |pdf| {
            let d = pdf.load_pdf_from_file(&s.path, None)?;
            let p = d.pages().get(s.index)?;
            let _ = pdf_dimensions(p.width().value, p.height().value)?;
            Ok(p.render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width((p.width().value * 200. / 72.).ceil() as i32),
            )?
            .as_image())
        })?
    } else if s.kind == "archive" {
        crate::image_input::decode(Cursor::new(source_bytes(s)?))?
    } else {
        crate::image_input::open(Path::new(&s.path))?
    };
    if image.width() as u64 * image.height() as u64 > 100_000_000 {
        bail!("Page exceeds 100 megapixels")
    };
    Ok(image)
}
fn pdf_dimensions(width: f32, height: f32) -> Result<(u32, u32)> {
    anyhow::ensure!(
        width.is_finite() && height.is_finite() && width > 0. && height > 0.,
        "Invalid PDF page size"
    );
    // Mirror the pinned Pdfium width-driven scaling and rounding, preserving pixels.
    let target = (width * 200. / 72.).ceil();
    let scale = target / width;
    let w = (width * scale).round() as u32;
    let h = (height * scale).round() as u32;
    crate::image_input::geometry(
        w,
        h,
        u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4),
    )?;
    Ok((w, h))
}
pub(crate) fn matches_dimensions(page: &Page, dimensions: (u32, u32)) -> bool {
    dimensions == (page.width, page.height)
        || (page.source.kind == "pdf"
            && page.width.abs_diff(dimensions.0) <= 1
            && page.height.abs_diff(dimensions.1) <= 1)
}
pub fn materialize(project: &Path, p: &Page) -> Result<PathBuf> {
    crate::materialization::get(project, p)
}
pub fn export(project: &Project, destination: &Path, format: &str) -> Result<usize> {
    export_named(
        project,
        destination,
        format,
        &std::collections::HashMap::new(),
    )
}
pub fn export_named(
    project: &Project,
    destination: &Path,
    format: &str,
    names: &std::collections::HashMap<String, String>,
) -> Result<usize> {
    if project.pages.is_empty() {
        bail!("Add pages before exporting")
    }
    let filename = |p: &Page| {
        names
            .get(&p.id)
            .cloned()
            .unwrap_or_else(|| format!("{:04}.png", p.number + 1))
    };
    for p in &project.pages {
        source_health(p)?;
    }
    let protected = crate::library::export_protection(Path::new(&project.path))?;
    let protect = |target: &Path| -> Result<()> {
        protected.check(target)?;
        if target.exists() {
            let absolute = target.canonicalize()?;
            if project
                .pages
                .iter()
                .any(|p| Path::new(&p.source.path).canonicalize().ok().as_ref() == Some(&absolute))
                || Path::new(&project.path).canonicalize().ok().as_ref() == Some(&absolute)
            {
                bail!(
                    "Export cannot overwrite an original source or the editable book; choose another destination"
                );
            }
        }
        Ok(())
    };
    if format == "images" {
        for p in &project.pages {
            protect(&destination.join(filename(p)))?;
        }
    } else {
        protect(destination)?;
    }
    let mut images = Vec::new();
    for p in &project.pages {
        let path = if let Some(r) = &p.rendered {
            PathBuf::from(r)
        } else {
            materialize(Path::new(&project.path), p)?
        };
        images.push((p, path));
    }
    match format {
        "images" => {
            fs::create_dir_all(destination)?;
            for (p, path) in &images {
                let target = destination.join(filename(p));
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(path, target)?;
            }
        }
        "cbz" => {
            let (output, file) = ExportOutput::create(destination)?;
            let mut z = zip::ZipWriter::new(file);
            for (p, path) in &images {
                z.start_file(
                    filename(p),
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )?;
                z.write_all(&fs::read(path)?)?;
            }
            z.finish()?.sync_all()?;
            output.publish(destination)?;
        }
        "pdf" => export_pdf(&images, destination)?,
        _ => bail!("Unsupported export format"),
    };
    Ok(images.len())
}

// A guard owns only the unique create_new file from this export attempt.
// Writer handles are declared after it and dropped before its Windows cleanup.
struct ExportOutput {
    path: PathBuf,
    published: bool,
}
impl ExportOutput {
    fn create(destination: &Path) -> Result<(Self, fs::File)> {
        let path = destination.with_extension(format!("{}.partial", uid()));
        let file = fs::File::create_new(&path)?;
        Ok((
            Self {
                path,
                published: false,
            },
            file,
        ))
    }
    fn publish(mut self, destination: &Path) -> Result<()> {
        store::replace_file(&self.path, destination)?;
        self.published = true;
        Ok(())
    }
}
impl Drop for ExportOutput {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn export_pdf(images: &[(&Page, PathBuf)], destination: &Path) -> Result<()> {
    use std::io::Seek;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let (output, mut file) = ExportOutput::create(destination)?;
    file.write_all(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = vec![0u64; 3 + images.len() * 3];
    fn object(file: &mut fs::File, offsets: &mut [u64], id: usize, body: &str) -> Result<()> {
        offsets[id] = file.stream_position()?;
        writeln!(file, "{id} 0 obj\n{body}\nendobj")?;
        Ok(())
    }
    object(
        &mut file,
        &mut offsets,
        1,
        "<< /Type /Catalog /Pages 2 0 R >>",
    )?;
    let kids = (0..images.len())
        .map(|i| format!("{} 0 R", 3 + i * 3))
        .collect::<Vec<_>>()
        .join(" ");
    object(
        &mut file,
        &mut offsets,
        2,
        &format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", images.len()),
    )?;
    for (i, (page, path)) in images.iter().enumerate() {
        let id = 3 + i * 3;
        let image_ref = id + 1;
        let content_ref = id + 2;
        let [w, h] = page.pdf_points.unwrap_or([
            page.width as f32 * 72. / 200.,
            page.height as f32 * 72. / 200.,
        ]);
        object(
            &mut file,
            &mut offsets,
            id,
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im {image_ref} 0 R >> >> /Contents {content_ref} 0 R >>"
            ),
        )?;
        let image = crate::image_input::open(path)?.to_rgb8();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 96)
            .encode_image(&DynamicImage::ImageRgb8(image))?;
        offsets[image_ref] = file.stream_position()?;
        writeln!(
            file,
            "{image_ref} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream",
            page.width,
            page.height,
            jpeg.len()
        )?;
        file.write_all(&jpeg)?;
        file.write_all(b"\nendstream\nendobj\n")?;
        let content = format!("q {w} 0 0 {h} 0 0 cm /Im Do Q\n");
        object(
            &mut file,
            &mut offsets,
            content_ref,
            &format!(
                "<< /Length {} >>\nstream\n{}endstream",
                content.len(),
                content
            ),
        )?;
    }
    let xref = file.stream_position()?;
    writeln!(file, "xref\n0 {}\n0000000000 65535 f ", offsets.len())?;
    for offset in offsets.iter().skip(1) {
        writeln!(file, "{offset:010} 00000 n ")?;
    }
    writeln!(
        file,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        offsets.len()
    )?;
    file.sync_all()?;
    drop(file);
    output.publish(destination)?;
    Ok(())
}
