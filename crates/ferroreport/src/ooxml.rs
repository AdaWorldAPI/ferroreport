//! What the Office Open XML exporters ([`crate::docx`], [`crate::pptx`]) share: the parts of the
//! package every document has (relationships, properties) and the pictures it carries.

use crate::image::{ImageData, Pixels};
use crate::xlsx::escape;
use std::collections::HashMap;
use std::fmt::Write as _;

pub(crate) const RELATIONSHIPS_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">";

/// The relationship type of a picture.
pub(crate) const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// The package's own relationships: its main part (`main`) and its properties.
pub(crate) fn root_rels(main: &str) -> String {
    format!(
        "{RELATIONSHIPS_HEAD}<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"{main}\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>\n"
    )
}

/// The document's title and creator.
pub(crate) fn core_xml(title: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>ferroreport</dc:creator></cp:coreProperties>\n",
        escape(title)
    )
}

/// The application that wrote the document.
pub(crate) const APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>ferroreport</Application></Properties>\n";

/// One relationship of a part.
pub(crate) fn relationship(out: &mut String, id: &str, kind: &str, target: &str, external: bool) {
    let mode = if external { " TargetMode=\"External\"" } else { "" };
    let _ = write!(out, "\n <Relationship Id=\"{}\" Type=\"{kind}\" Target=\"{}\"{mode}/>", escape(id), escape(target));
}

/// The pictures of a document: each image once however often it is used, and the pictures
/// drawn for charts and barcodes. Files are named `img_<n>.<ext>`.
#[derive(Default)]
pub(crate) struct Media {
    /// Every file name, in order.
    pub names: Vec<String>,
    /// Files not yet written to the package: name and bytes.
    pub pending: Vec<(String, Vec<u8>)>,
    /// Pictures of images already made, by image identifier.
    by_image: HashMap<usize, String>,
}

impl Media {
    pub fn len(&self) -> usize {
        self.names.len()
    }

    fn push(&mut self, ext: &str, bytes: Vec<u8>) -> String {
        let name = format!("img_{}.{ext}", self.names.len());
        self.names.push(name.clone());
        self.pending.push((name.clone(), bytes));
        name
    }

    /// The file of an image (JPEG as it is, anything else as PNG).
    pub fn image(&mut self, data: &ImageData) -> Option<String> {
        if let Some(n) = self.by_image.get(&data.id) {
            return Some(n.clone());
        }
        let (ext, bytes) = match &data.pixels {
            Pixels::Jpeg { data, .. } => ("jpg", data.clone()),
            Pixels::Raw { samples, gray, alpha } => {
                ("png", crate::xlsx::encode_png(data.width, data.height, samples, *gray, alpha.as_deref())?)
            }
            Pixels::Svg(svg) => {
                let (w, h) = crate::svg::picture_size(&svg.tree);
                ("png", crate::svg::png(&svg.tree, w, h)?)
            }
        };
        let name = self.push(ext, bytes);
        self.by_image.insert(data.id, name.clone());
        Some(name)
    }

    /// A PNG drawn for the document (a chart, a barcode).
    pub fn png(&mut self, bytes: Vec<u8>) -> String {
        self.push("png", bytes)
    }
}
