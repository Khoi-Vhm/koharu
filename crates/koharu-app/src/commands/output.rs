mod files;

use anyhow::{Context as _, Result};
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use koharu_epub::EpubWriter;
use koharu_psd::{PsdExportOptions, export_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use serde::Deserialize;
use specta::Type;
use std::{io::Write as _, sync::Arc};
use tauri::{State, WebviewWindow, ipc::IpcResponse};
use tauri_runtime_cef::CefRuntime;

use super::{Error, project::CurrentProject};
use koharu_desktop::Desktop;

const THUMBNAIL_EDGE: u32 = 128;
// Export jobs share GPU resources; concurrent invocations must not multiply
// page-sized render and encoding allocations.
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Type)]
#[specta(transparent)]
pub(crate) struct ThumbnailBytes(#[specta(type = Vec<u8>)] Vec<u8>);

impl IpcResponse for ThumbnailBytes {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(self.0.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
    Epub,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn export(
    window: WebviewWindow<CefRuntime>,
    format: ExportFormat,
    project: State<'_, CurrentProject>,
    desktop: State<'_, Desktop>,
) -> std::result::Result<(), Error> {
    let _export_guard = EXPORT_LOCK.lock().await;
    let (name, snapshot) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.name.clone(), project.snapshot())
    };
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match format {
        ExportFormat::Png | ExportFormat::Psd => dialog.pick_folder().await,
        ExportFormat::Cbz => {
            dialog
                .add_filter("Comic Book Archive", &["cbz"])
                .set_file_name(format!("{name}.cbz"))
                .save_file()
                .await
        }
        ExportFormat::Epub => {
            dialog
                .add_filter("EPUB Book", &["epub"])
                .set_file_name(format!("{name}.epub"))
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|destination| destination.path().to_owned()) else {
        return Ok(());
    };
    if matches!(format, ExportFormat::Epub) {
        // Check the whole project before decoding images or allocating GPU
        // surfaces, including pages that would otherwise fail late in the book.
        for &page_id in &pages {
            let page = snapshot.page(page_id)?.page()?;
            koharu_epub::validate_page_size(page.width.ceil() as u32, page.height.ceil() as u32)
                .with_context(|| format!("cannot export page {}", page.label))?;
        }
        let mut book = tokio_rayon::spawn(move || EpubWriter::new(&destination, &name)).await?;
        let renderer = desktop.renderer();
        let rasterizer = desktop.rasterizer().await?;
        for page_id in pages {
            let label = snapshot.page(page_id)?.page()?.label;
            let frame = renderer.render(&snapshot, page_id).await?;
            let rasterizer = Arc::clone(&rasterizer);
            book = tokio_rayon::spawn(move || -> Result<_> {
                let image = rasterizer
                    .rasterize(&frame.raster_frame()?, RasterOptions::default())?
                    .image;
                book.push_page(&label, &image)?;
                Ok(book)
            })
            .await?;
        }
        tokio_rayon::spawn(move || book.finish()).await?;
        return Ok(());
    }
    let renderer = desktop.renderer();
    let rasterizer = desktop.rasterizer().await?;
    let directory = destination.clone();
    let mut archive = tokio_rayon::spawn(move || -> Result<_> {
        if matches!(format, ExportFormat::Cbz) {
            let parent = directory.parent().context("archive path has no parent")?;
            Ok(Some(zip::ZipWriter::new(tempfile::NamedTempFile::new_in(
                parent,
            )?)))
        } else {
            Ok(None)
        }
    })
    .await?;
    let width = pages.len().to_string().len().max(4);
    // Finish and release each page before loading the next one's image resources.
    let extension = if matches!(format, ExportFormat::Psd) {
        "psd"
    } else {
        "png"
    };
    for (index, page_id) in pages.into_iter().enumerate() {
        let page = snapshot.page(page_id)?.page()?;
        let name = page
            .label
            .trim()
            .trim_end_matches(|character: char| character == '.' || character.is_whitespace());
        let name = name
            .rsplit_once('.')
            .map_or(name, |(stem, _)| stem)
            .replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
        let name = format!(
            "{:0width$}_{}.{extension}",
            index + 1,
            if name.is_empty() { "page" } else { &name }
        );
        if archive.is_none() {
            let path = destination.join(&name);
            if tokio_rayon::spawn(move || files::is_complete(&path)).await? {
                continue;
            }
        }
        let frame = renderer.render(&snapshot, page_id).await?;
        let bytes = match format {
            ExportFormat::Png | ExportFormat::Cbz => {
                let rasterizer = Arc::clone(&rasterizer);
                tokio_rayon::spawn(move || -> Result<Vec<u8>> {
                    let image = rasterizer
                        .rasterize(&frame.raster_frame()?, RasterOptions::default())?
                        .image;
                    let mut bytes = Vec::new();
                    PngEncoder::new_with_quality(
                        &mut bytes,
                        CompressionType::Best,
                        FilterType::Adaptive,
                    )
                    .write_image(
                        image.as_raw(),
                        image.width(),
                        image.height(),
                        ExtendedColorType::Rgba8,
                    )?;
                    Ok(bytes)
                })
                .await?
            }
            ExportFormat::Psd => {
                export_page(
                    Arc::clone(&rasterizer),
                    &snapshot,
                    &frame,
                    &PsdExportOptions::default(),
                )
                .await?
            }
            ExportFormat::Epub => unreachable!("EPUB is written as a streaming book above"),
        };
        let destination = destination.clone();
        archive = tokio_rayon::spawn(move || -> Result<_> {
            if let Some(archive) = &mut archive {
                // PNG data is already compressed.
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                archive.start_file(name, options)?;
                archive.write_all(&bytes)?;
            } else {
                files::write(&destination.join(name), &bytes)?;
            }
            Ok(archive)
        })
        .await?;
    }
    if let Some(archive) = archive {
        tokio_rayon::spawn(move || -> Result<()> {
            archive.finish()?.persist(destination)?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_thumbnail(
    page: EntityId,
    project: State<'_, CurrentProject>,
) -> std::result::Result<ThumbnailBytes, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    snapshot.page(page)?;
    let blob = snapshot
        .asset(page, &AssetRole::new("source")?)?
        .with_context(|| format!("page {page} has no source image"))?
        .blob;
    let bytes = snapshot.read_blob(blob).await?;
    let bytes = tokio_rayon::spawn(move || -> Result<Vec<u8>> {
        let image = image::load_from_memory(&bytes).context("failed to decode source image")?;
        if image.width() == 0 || image.height() == 0 {
            return Err(anyhow::anyhow!("source image is empty"));
        }
        let image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(encoder.encode(80.0).to_vec())
    })
    .await?;
    Ok(ThumbnailBytes(bytes))
}

pub(crate) async fn rendered_preview(
    renderer: &Renderer,
    rasterizer: Arc<Rasterizer>,
    snapshot: &Snapshot,
    page: EntityId,
) -> Result<Vec<u8>> {
    snapshot.page(page)?;
    let frame = renderer.render(snapshot, page).await?;
    let image = rasterize(rasterizer, &frame, RasterOptions::default())
        .await?
        .image;
    tokio_rayon::spawn(move || {
        let image = image::DynamicImage::ImageRgba8(image)
            .resize(1024, 1024, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok::<_, anyhow::Error>(encoder.encode(85.0).to_vec())
    })
    .await
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster> {
    let frame = frame.raster_frame()?;
    tokio_rayon::spawn(move || rasterizer.rasterize(&frame, options))
        .await
        .map_err(Into::into)
}
