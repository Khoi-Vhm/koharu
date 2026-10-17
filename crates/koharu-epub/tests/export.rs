use std::{fs, io::Read};

use image::{Rgba, RgbaImage};
use koharu_epub::EpubWriter;
use zip::{CompressionMethod, ZipArchive};

fn read_entry(archive: &mut ZipArchive<fs::File>, name: &str) -> String {
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn exports_ordered_fixed_layout_pages_with_lossless_pixels_and_safe_labels() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.epub");
    let mut writer = EpubWriter::new(&path, "漫画 <&> \"book\"\0").unwrap();
    let first = RgbaImage::from_fn(3, 2, |x, y| Rgba([x as u8 * 70, y as u8 * 90, 128, 127]));
    let second = RgbaImage::from_pixel(2, 4, Rgba([12, 34, 56, 255]));
    writer
        .push_page("../../first <&> \"日本語\"\u{1}", &first)
        .unwrap();
    writer.push_page("second", &second).unwrap();
    assert!(!path.exists(), "only a completed book may be published");
    writer.finish().unwrap();

    let mut archive = ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    {
        let mut mimetype = archive.by_index(0).unwrap();
        assert_eq!(mimetype.name(), "mimetype");
        assert_eq!(mimetype.compression(), CompressionMethod::Stored);
        assert_eq!(mimetype.extra_data().unwrap_or_default(), b"");
        let mut bytes = Vec::new();
        mimetype.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"application/epub+zip");
    }
    let container = read_entry(&mut archive, "META-INF/container.xml");
    let container = roxmltree::Document::parse(&container).unwrap();
    assert_eq!(
        container
            .descendants()
            .find(|node| node.has_tag_name("rootfile"))
            .unwrap()
            .attribute("full-path"),
        Some("EPUB/package.opf")
    );

    let package = read_entry(&mut archive, "EPUB/package.opf");
    let package = roxmltree::Document::parse(&package).unwrap();
    let title = package
        .descendants()
        .find(|node| node.tag_name().name() == "title")
        .unwrap();
    assert_eq!(title.text(), Some("漫画 <&> \"book\"�"));
    let spine: Vec<_> = package
        .descendants()
        .filter(|node| node.has_tag_name("itemref"))
        .map(|node| node.attribute("idref").unwrap())
        .collect();
    assert_eq!(spine, ["page-1", "page-2"]);
    assert!(package.descendants().any(|node| node.attribute("property")
        == Some("rendition:layout")
        && node.text() == Some("pre-paginated")));
    let modified = package
        .descendants()
        .find(|node| node.attribute("property") == Some("dcterms:modified"))
        .unwrap()
        .text()
        .unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(modified).is_ok());

    let navigation = read_entry(&mut archive, "EPUB/nav.xhtml");
    let navigation = roxmltree::Document::parse(&navigation).unwrap();
    let links: Vec<_> = navigation
        .descendants()
        .filter(|node| node.has_tag_name("a"))
        .map(|node| (node.attribute("href").unwrap(), node.text().unwrap()))
        .collect();
    assert_eq!(
        links,
        [
            ("pages/1.xhtml", "../../first <&> \"日本語\"�"),
            ("pages/2.xhtml", "second")
        ]
    );

    for (number, expected) in [(1, first), (2, second)] {
        let page = read_entry(&mut archive, &format!("EPUB/pages/{number}.xhtml"));
        let page = roxmltree::Document::parse(&page).unwrap();
        let viewport = page
            .descendants()
            .find(|node| node.attribute("name") == Some("viewport"))
            .unwrap();
        assert_eq!(
            viewport.attribute("content").unwrap(),
            format!("width={}, height={}", expected.width(), expected.height())
        );
        let img = page
            .descendants()
            .find(|node| node.has_tag_name("img"))
            .unwrap();
        assert_eq!(
            img.attribute("src").unwrap(),
            format!("../images/{number}.png")
        );
        let mut entry = archive
            .by_name(&format!("EPUB/images/{number}.png"))
            .unwrap();
        assert_eq!(entry.compression(), CompressionMethod::Stored);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let actual = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(actual, expected);
    }
}

#[test]
fn unfinished_empty_or_invalid_books_leave_existing_destination_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.epub");
    fs::write(&path, b"existing book").unwrap();
    {
        let mut writer = EpubWriter::new(&path, "book").unwrap();
        writer
            .push_page("page", &RgbaImage::from_pixel(2, 2, Rgba([0; 4])))
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"existing book");
    }
    let writer = EpubWriter::new(&path, "book").unwrap();
    assert!(writer.finish().is_err());
    let mut writer = EpubWriter::new(&path, "book").unwrap();
    writer
        .push_page("valid", &RgbaImage::from_pixel(1, 1, Rgba([255; 4])))
        .unwrap();
    assert!(writer.push_page("invalid", &RgbaImage::new(0, 1)).is_err());
    assert!(writer.finish().is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing book");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn rejects_oversized_surfaces_without_allocating_them() {
    use koharu_epub::validate_page_size;
    assert!(validate_page_size(4096, 8192).is_ok());
    assert!(validate_page_size(4096, 8193).is_err());
    assert!(validate_page_size(32769, 1).is_err());
    assert!(validate_page_size(u32::MAX, u32::MAX).is_err());
    assert!(validate_page_size(1, 0).is_err());
}

#[test]
fn replaces_destination_only_after_success_and_cleans_up_failed_publication() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.epub");
    fs::write(&path, b"existing book").unwrap();
    let mut writer = EpubWriter::new(&path, "book").unwrap();
    writer
        .push_page("page", &RgbaImage::from_pixel(1, 1, Rgba([255; 4])))
        .unwrap();
    writer.finish().unwrap();
    assert!(ZipArchive::new(fs::File::open(&path).unwrap()).is_ok());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut writer = EpubWriter::new(&path, "book").unwrap();
    writer
        .push_page("page", &RgbaImage::from_pixel(1, 1, Rgba([255; 4])))
        .unwrap();
    assert!(writer.finish().is_err());
    assert!(path.is_dir());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

/// Run the compiled test binary under `/usr/bin/time -l` in separate processes
/// with KOHARU_EPUB_PROBE_PAGES=10 and 100 to compare peak resident memory.
#[test]
#[ignore = "manual streaming-memory measurement"]
fn streaming_memory_probe() {
    let pages: usize = std::env::var("KOHARU_EPUB_PROBE_PAGES")
        .unwrap_or_else(|_| "100".into())
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("book.epub");
    let image = RgbaImage::from_fn(1536, 2048, |x, y| {
        let value = ((x * 37 + y * 71 + x * y) % 256) as u8;
        Rgba([value, value, value, 255])
    });
    let start = std::time::Instant::now();
    let mut writer = EpubWriter::new(&path, "Memory probe").unwrap();
    for number in 0..pages {
        writer.push_page(&format!("Page {number}"), &image).unwrap();
    }
    writer.finish().unwrap();
    eprintln!(
        "{pages} pages, 1536x2048 RGBA, {:?}, {} bytes",
        start.elapsed(),
        fs::metadata(path).unwrap().len()
    );
}
