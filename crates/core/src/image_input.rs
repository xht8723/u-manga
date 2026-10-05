//! Bounded image admission shared by originals, covers, backgrounds, and cached pixels.
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};
pub const MAX_ENCODED: u64 = 256 * 1024 * 1024;
pub const MAX_DECODED: u64 = 512 * 1024 * 1024;
pub fn geometry(width: u32, height: u32, bytes: u64) -> Result<()> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .context("Image size overflow")?;
    ensure!(
        width > 0 && height > 0 && pixels <= 100_000_000,
        "Page exceeds 100 megapixels"
    );
    ensure!(bytes <= MAX_DECODED, "Image exceeds decoder memory budget");
    Ok(())
}
pub fn encoded(reader: impl Read, declared: u64) -> Result<Vec<u8>> {
    ensure!(declared <= MAX_ENCODED, "Page exceeds size limit");
    let mut bytes = Vec::new();
    reader.take(MAX_ENCODED + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_ENCODED, "Page exceeds size limit");
    Ok(bytes)
}
// Keep a seekable file bounded even if it grows after the metadata check.
struct BoundedFile(File);
impl Read for BoundedFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = MAX_ENCODED.saturating_sub(self.0.stream_position()?);
        let count = bytes.len().min(remaining as usize);
        self.0.read(&mut bytes[..count])
    }
}
impl Seek for BoundedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = self.0.seek(position)?;
        if next > MAX_ENCODED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Page exceeds size limit",
            ));
        }
        Ok(next)
    }
}
fn reader(path: &Path) -> Result<BufReader<BoundedFile>> {
    let file = File::open(path).context("Image file is missing or inaccessible")?;
    ensure!(
        file.metadata()?.len() <= MAX_ENCODED,
        "Page exceeds size limit"
    );
    Ok(BufReader::new(BoundedFile(file)))
}
fn decoder<'a>(reader: impl BufRead + Seek + 'a) -> Result<impl ImageDecoder + 'a> {
    let mut reader = ImageReader::new(reader).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED);
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    geometry(width, height, decoder.total_bytes())?;
    Ok(decoder)
}
pub fn dimensions(path: &Path) -> Result<(u32, u32)> {
    Ok(decoder(reader(path)?)?.dimensions())
}
pub fn open(path: &Path) -> Result<DynamicImage> {
    decode(reader(path)?)
}
pub fn decode(reader: impl BufRead + Seek) -> Result<DynamicImage> {
    // Geometry and pixels come from the same decoder and file/entry, never a second read.
    Ok(DynamicImage::from_decoder(decoder(reader)?)?)
}
pub fn bytes_dimensions(bytes: Vec<u8>) -> Result<(u32, u32)> {
    Ok(decoder(io::Cursor::new(bytes))?.dimensions())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_metadata_and_decoder_budget_before_allocation() {
        assert!(encoded(io::empty(), MAX_ENCODED + 1).is_err());
        assert!(geometry(10001, 10001, 0).is_err());
        assert!(geometry(10000, 10000, 800_000_000).is_err());
        assert!(geometry(0, 1, 0).is_err());
        assert_eq!(encoded(&b"pixels"[..], 0).unwrap(), b"pixels");
    }
}
