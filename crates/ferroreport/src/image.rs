//! Image loading and decoding.
//!
//! Images come from template resources (paths relative to the resource directories) or from
//! bytes produced by an expression (base64 data from the data source). Network locations are
//! never fetched, and paths cannot escape the resource directories, so a template from an
//! untrusted source cannot read arbitrary files.

use crate::error::{Error, Result};
use base64::Engine;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Pixel data ready to be embedded in a PDF.
#[derive(Debug)]
pub enum Pixels {
    /// A JPEG file, embedded as is.
    Jpeg { data: Vec<u8>, components: u8 },
    /// Decoded 8-bit samples (gray or RGB) with an optional 8-bit alpha channel.
    Raw { samples: Vec<u8>, gray: bool, alpha: Option<Vec<u8>> },
}

#[derive(Debug)]
pub struct ImageData {
    /// Stable identifier within a render (used to share one PDF object between uses).
    pub id: usize,
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

/// Loads and caches the resources of one render: decoded images and parsed subreports.
#[derive(Debug, Default)]
pub struct ImageStore {
    roots: Vec<PathBuf>,
    by_key: HashMap<String, Arc<ImageData>>,
    next_id: usize,
    /// Parsed subreport templates by location.
    reports: HashMap<String, Arc<crate::model::Report>>,
    /// Subreports being filled right now (guards against runaway recursion).
    pub(crate) depth: usize,
}

impl ImageStore {
    pub fn new() -> ImageStore {
        ImageStore::default()
    }

    /// Adds a directory relative image paths are resolved against.
    pub fn add_root(&mut self, dir: impl Into<PathBuf>) {
        self.roots.push(dir.into());
    }

    /// Resolves a location string (a relative path, or a `data:` URI) to an image.
    pub fn load_location(&mut self, location: &str) -> Result<Arc<ImageData>> {
        if let Some(img) = self.by_key.get(location) {
            return Ok(img.clone());
        }
        let bytes = if let Some(data) = location.strip_prefix("data:") {
            let (_, b64) =
                data.split_once(";base64,").ok_or_else(|| Error::Data("only base64 data URIs are supported".into()))?;
            decode_base64(b64)?
        } else if location.contains("://") {
            return Err(Error::Unsupported(format!("image `{location}`: remote images are not fetched")));
        } else {
            std::fs::read(self.resolve(location)?).map_err(|e| Error::Data(format!("image `{location}`: {e}")))?
        };
        let img = self.decode(&bytes)?;
        self.by_key.insert(location.to_string(), img.clone());
        Ok(img)
    }

    /// Loads a subreport template. A compiled `.jasper` location is resolved to the `.jrxml`
    /// source next to it: compiled templates are never deserialized.
    pub fn load_report(&mut self, location: &str) -> Result<Arc<crate::model::Report>> {
        if let Some(r) = self.reports.get(location) {
            return Ok(r.clone());
        }
        if location.contains("://") {
            return Err(Error::Unsupported(format!("subreport `{location}`: remote templates are not fetched")));
        }
        let source = match location.strip_suffix(".jasper") {
            Some(stem) => format!("{stem}.jrxml"),
            None => location.to_string(),
        };
        let path = self.resolve(&source)?;
        let text = std::fs::read_to_string(&path).map_err(|e| Error::Data(format!("subreport `{source}`: {e}")))?;
        let report = Arc::new(crate::jrxml::parse_report(&text)?);
        self.reports.insert(location.to_string(), report.clone());
        Ok(report)
    }

    /// Decodes image bytes (PNG or JPEG).
    pub fn load_bytes(&mut self, bytes: &[u8]) -> Result<Arc<ImageData>> {
        let key = format!("bytes:{:016x}:{}", fnv(bytes), bytes.len());
        if let Some(img) = self.by_key.get(&key) {
            return Ok(img.clone());
        }
        let img = self.decode(bytes)?;
        self.by_key.insert(key, img.clone());
        Ok(img)
    }

    fn resolve(&self, location: &str) -> Result<PathBuf> {
        let rel = Path::new(location);
        let escapes = rel.is_absolute()
            || rel.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)));
        if escapes {
            return Err(Error::Unsupported(format!("`{location}`: paths must be relative to the template's resource directory")));
        }
        let roots: Vec<PathBuf> = if self.roots.is_empty() { vec![PathBuf::from(".")] } else { self.roots.clone() };
        roots
            .iter()
            .map(|r| r.join(rel))
            .find(|p| p.is_file())
            .ok_or_else(|| Error::Data(format!("`{location}` not found in the resource directories")))
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Arc<ImageData>> {
        let id = self.next_id;
        self.next_id += 1;
        let img = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            decode_png(bytes, id)?
        } else if bytes.starts_with(&[0xFF, 0xD8]) {
            decode_jpeg(bytes, id)?
        } else {
            return Err(Error::Unsupported("image format (only PNG and JPEG are supported)".into()));
        };
        Ok(Arc::new(img))
    }
}

pub fn decode_base64(s: &str) -> Result<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .map_err(|e| Error::Data(format!("invalid base64 data: {e}")))
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn decode_png(bytes: &[u8], id: usize) -> Result<ImageData> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error::Data(format!("PNG: {e}")))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| Error::Data(format!("PNG: {e}")))?;
    let buf = &buf[..info.buffer_size()];
    let (gray, has_alpha, channels) = match info.color_type {
        png::ColorType::Grayscale => (true, false, 1),
        png::ColorType::GrayscaleAlpha => (true, true, 2),
        png::ColorType::Rgb => (false, false, 3),
        png::ColorType::Rgba => (false, true, 4),
        png::ColorType::Indexed => return Err(Error::Data("PNG: unexpanded palette".into())),
    };
    let color_channels = if gray { 1 } else { 3 };
    let pixels = (info.width * info.height) as usize;
    let mut samples = Vec::with_capacity(pixels * color_channels);
    let mut alpha = if has_alpha { Some(Vec::with_capacity(pixels)) } else { None };
    for px in buf.chunks_exact(channels) {
        samples.extend_from_slice(&px[..color_channels]);
        if let Some(a) = &mut alpha {
            a.push(px[channels - 1]);
        }
    }
    // Java's ImageIO decodes gray PNGs into a linear gray color space and JasperReports
    // embeds them converted to sRGB, which brightens mid tones; do the same.
    let (samples, gray) = if gray {
        let lut: Vec<u8> = (0..=255u32)
            .map(|v| {
                let l = v as f64 / 255.0;
                let s = if l <= 0.003_130_8 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
                (s * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        (
            samples
                .iter()
                .flat_map(|&v| {
                    let c = lut[v as usize];
                    [c, c, c]
                })
                .collect(),
            false,
        )
    } else {
        (samples, gray)
    };
    // A fully opaque alpha channel is dropped.
    if alpha.as_ref().is_some_and(|a| a.iter().all(|&v| v == 255)) {
        alpha = None;
    }
    Ok(ImageData { id, width: info.width, height: info.height, pixels: Pixels::Raw { samples, gray, alpha } })
}

fn decode_jpeg(bytes: &[u8], id: usize) -> Result<ImageData> {
    // Walk the markers up to the first start-of-frame to read the dimensions.
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof && i + 9 < bytes.len() {
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            let components = bytes[i + 9];
            return Ok(ImageData { id, width, height, pixels: Pixels::Jpeg { data: bytes.to_vec(), components } });
        }
        i += 2 + len;
    }
    Err(Error::Data("JPEG: no frame header found".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_the_resource_directory() {
        let mut store = ImageStore::new();
        store.add_root(std::env::temp_dir());
        for bad in ["/etc/passwd", "../secret.png", "a/../../b.png", "http://example.com/x.png"] {
            assert!(store.load_location(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn decodes_a_data_uri_png() {
        // 1x1 transparent PNG.
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let img = ImageStore::new().load_location(uri).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
    }
}
