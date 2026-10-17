//! Streaming fixed-layout EPUB export of rendered project pages.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use image::RgbaImage;
use tempfile::NamedTempFile;
use uuid::Uuid;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Limits a single RGBA export surface to 128 MiB before rendering/encoding.
/// GPU surfaces and renderer resources require additional memory.
pub fn validate_page_size(width: u32, height: u32) -> Result<()> {
    ensure!(
        width > 0 && height > 0,
        "EPUB page dimensions must be positive"
    );
    ensure!(
        width <= 32_768 && height <= 32_768 && u64::from(width) * u64::from(height) <= 33_554_432,
        "EPUB page {width}x{height} exceeds the export memory limit (32 megapixels, 32768 pixels per side)"
    );
    Ok(())
}

/// Owns the unfinished archive. Dropping it never replaces the destination.
/// Pixel data is encoded directly to disk; only the ZIP index and XML metadata
/// grow with the page count.
pub struct EpubWriter {
    archive: ZipWriter<NamedTempFile>,
    destination: PathBuf,
    title: String,
    manifest: String,
    spine: String,
    navigation: String,
    pages: usize,
    failed: bool,
}

impl EpubWriter {
    pub fn new(destination: &Path, title: &str) -> Result<Self> {
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut archive = ZipWriter::new(NamedTempFile::new_in(parent)?);
        // EPUB OCF requires this to be the first entry, stored without extras.
        archive.start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )?;
        archive.write_all(b"application/epub+zip")?;
        archive.start_file(
            "META-INF/container.xml",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        archive.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)?;
        Ok(Self {
            archive,
            destination: destination.to_owned(),
            title: xml(if title.trim().is_empty() {
                "Untitled"
            } else {
                title
            }),
            manifest: String::new(),
            spine: String::new(),
            navigation: String::new(),
            pages: 0,
            failed: false,
        })
    }

    pub fn push_page(&mut self, label: &str, image: &RgbaImage) -> Result<()> {
        ensure!(!self.failed, "cannot continue a failed EPUB export");
        self.failed = true;
        validate_page_size(image.width(), image.height())?;
        let number = self.pages + 1;
        let label = if label.trim().is_empty() {
            format!("Page {number}")
        } else {
            xml(label)
        };
        let (width, height) = image.dimensions();
        self.archive.start_file(
            format!("EPUB/images/{number}.png"),
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )?;
        // PNG is already compressed. Stream it into ZIP rather than retaining
        // another page-sized encoded buffer or recompressing it in the archive.
        {
            // image::PngEncoder buffers a complete compressed image internally.
            // png's stream writer keeps only row buffers and a 64 KiB IDAT chunk.
            let mut encoder = png::Encoder::new(&mut self.archive, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut png = encoder.write_header()?;
            let mut stream = png.stream_writer_with_size(64 * 1024)?;
            stream.write_all(image.as_raw())?;
            stream.finish()?;
            png.finish()?;
        }
        self.archive.start_file(
            format!("EPUB/pages/{number}.xhtml"),
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        write!(
            self.archive,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="und"><head><title>{label}</title><meta name="viewport" content="width={width}, height={height}"/><style>html,body{{margin:0;padding:0;width:100%;height:100%;}}img{{display:block;width:100%;height:100%;}}</style></head><body><img src="../images/{number}.png" width="{width}" height="{height}" alt="{label}"/></body></html>"#
        )?;
        use std::fmt::Write as _;
        write!(
            self.manifest,
            r#"<item id="image-{number}" href="images/{number}.png" media-type="image/png"/><item id="page-{number}" href="pages/{number}.xhtml" media-type="application/xhtml+xml"/>"#
        )?;
        write!(self.spine, r#"<itemref idref="page-{number}"/>"#)?;
        write!(
            self.navigation,
            r#"<li><a href="pages/{number}.xhtml">{label}</a></li>"#
        )?;
        self.pages = number;
        self.failed = false;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        ensure!(!self.failed, "cannot finish a failed EPUB export");
        ensure!(self.pages > 0, "there are no pages to export");
        let identifier = Uuid::new_v4();
        let modified = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        self.archive.start_file(
            "EPUB/package.opf",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        write!(
            self.archive,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:uuid:{identifier}</dc:identifier><dc:title>{}</dc:title><dc:language>und</dc:language><meta property="dcterms:modified">{modified}</meta><meta property="rendition:layout">pre-paginated</meta><meta property="rendition:spread">none</meta></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>{}</manifest><spine>{}</spine></package>"#,
            self.title, self.manifest, self.spine
        )?;
        self.archive.start_file(
            "EPUB/nav.xhtml",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        write!(
            self.archive,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="und"><head><title>{}</title></head><body><nav epub:type="toc"><h1>{}</h1><ol>{}</ol></nav></body></html>"#,
            self.title, self.title, self.navigation
        )?;
        let file = self.archive.finish()?;
        file.as_file().sync_all()?;
        file.persist(&self.destination)
            .with_context(|| format!("failed to save EPUB {}", self.destination.display()))?;
        Ok(())
    }
}

fn xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t'
            | '\n'
            | '\r'
            | '\u{20}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..='\u{10ffff}' => escaped.push(character),
            _ => escaped.push('\u{fffd}'),
        }
    }
    escaped
}
