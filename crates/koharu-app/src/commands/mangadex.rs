use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, bail, ensure};
use koharu_runtime::mangadex::MangaDexDownloader;
use parking_lot::Mutex;
use reqwest::{Client, tls::Version};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use specta::Type;
use tauri::{AppHandle, Manager as _, WebviewWindow, ipc::Channel};
use tauri_runtime_cef::CefRuntime;
use tokio::{io::AsyncReadExt as _, process::Child, sync::watch, time::Duration};
use url::Url;
use walkdir::WalkDir;

use super::{
    Error,
    canvas::CanvasChannel,
    import, lifecycle,
    project::{CurrentProject, ProjectLibrary},
};

const API: &str = "https://api.mangadex.org";
const CHAPTER_PAGE_SIZE: usize = 100;
static API_CLIENT: OnceLock<Client> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Type)]
pub struct MangaDexImportEvent {
    pub job_id: String,
    pub state: MangaDexImportState,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MangaDexImportState {
    RuntimeSetup,
    Downloading,
    Importing,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(crate) struct MangaDexImports {
    jobs: Mutex<HashMap<String, MangaDexImportControl>>,
}

#[derive(Clone)]
struct MangaDexImportControl {
    cancelled: watch::Sender<bool>,
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
    transition: Arc<tokio::sync::Mutex<ImportTransition>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportTransition {
    Active,
    Committing,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct MangaDexMetadata {
    pub manga_id: String,
    pub title: String,
    pub languages: Vec<MangaDexLanguage>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct MangaDexLanguage {
    pub code: String,
    pub name: String,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ChapterManifest {
    pub(crate) id: String,
    pub(crate) number: String,
    pub(crate) page_count: u32,
}

pub(crate) async fn resolve_manga(url: &str) -> Result<MangaDexMetadata> {
    resolve_manga_at(url, API).await
}

pub(crate) async fn chapter_manifest(
    manga_id: &str,
    language: &str,
) -> Result<Vec<ChapterManifest>> {
    chapter_manifest_at(manga_id, language, API).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn resolve_mangadex(url: String) -> std::result::Result<MangaDexMetadata, Error> {
    Ok(resolve_manga(&url).await?)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn pick_mangadex_output_folder(
    window: WebviewWindow<CefRuntime>,
) -> std::result::Result<Option<String>, Error> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned()))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn start_mangadex_import(
    name: String,
    url: String,
    language: String,
    output: String,
    on_event: Channel<MangaDexImportEvent>,
    handle: AppHandle<CefRuntime>,
) -> std::result::Result<String, Error> {
    let output = PathBuf::from(output);
    let manga_id = validate_import_request(&name, &url, &language, &output)?;

    let job_id = uuid::Uuid::new_v4().to_string();
    let (cancelled, receiver) = watch::channel(false);
    let control = MangaDexImportControl {
        cancelled,
        child: Arc::new(tokio::sync::Mutex::new(None)),
        transition: Arc::new(tokio::sync::Mutex::new(ImportTransition::Active)),
    };
    handle
        .state::<MangaDexImports>()
        .jobs
        .lock()
        .insert(job_id.clone(), control.clone());
    publish(&on_event, &job_id, MangaDexImportState::RuntimeSetup);

    let task_handle = handle.clone();
    let task_job_id = job_id.clone();
    drop(tokio::spawn(async move {
        let mut receiver = receiver;
        let result = run_mangadex_import(
            &task_handle,
            &control,
            &mut receiver,
            &name,
            &manga_id,
            &language,
            &output,
            &on_event,
            &task_job_id,
        )
        .await;
        let cancelled = {
            let imports = task_handle.state::<MangaDexImports>();
            let mut jobs = imports.jobs.lock();
            let cancelled = *receiver.borrow();
            jobs.remove(&task_job_id);
            cancelled
        };
        let event = match (result, cancelled) {
            (_, true) => MangaDexImportEvent {
                job_id: task_job_id.clone(),
                state: MangaDexImportState::Cancelled,
                error: None,
            },
            (Ok(()), false) => MangaDexImportEvent {
                job_id: task_job_id.clone(),
                state: MangaDexImportState::Finished,
                error: None,
            },
            (Err(error), false) => MangaDexImportEvent {
                job_id: task_job_id.clone(),
                state: MangaDexImportState::Failed,
                error: Some(format!("{error:#}")),
            },
        };
        let _ = on_event.send(event);
    }));
    Ok(job_id)
}

fn validate_import_request(name: &str, url: &str, language: &str, output: &Path) -> Result<String> {
    ensure!(!name.trim().is_empty(), "project name cannot be empty");
    let manga_id = manga_id(url)?;
    ensure!(
        !language.trim().is_empty(),
        "MangaDex language cannot be empty"
    );
    ensure!(
        output.is_absolute(),
        "MangaDex output folder must be absolute"
    );
    ensure!(output.is_dir(), "MangaDex output folder does not exist");
    Ok(manga_id)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel_mangadex_import(
    job_id: String,
    handle: AppHandle<CefRuntime>,
) -> std::result::Result<(), Error> {
    let control = {
        let imports = handle.state::<MangaDexImports>();
        let jobs = imports.jobs.lock();
        let control = jobs
            .get(&job_id)
            .cloned()
            .context("MangaDex import job does not exist")?;
        control
    };
    cancel_transition(&control).await?;
    if let Some(child) = control.child.lock().await.as_mut() {
        let _ = child.start_kill();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_mangadex_import(
    handle: &AppHandle<CefRuntime>,
    control: &MangaDexImportControl,
    cancelled: &mut watch::Receiver<bool>,
    name: &str,
    manga_id: &str,
    language: &str,
    output: &Path,
    on_event: &Channel<MangaDexImportEvent>,
    job_id: &str,
) -> Result<()> {
    create_and_open_project(handle, name).await?;
    let manifest = cancelable(cancelled, chapter_manifest(manga_id, language)).await?;
    ensure!(
        !manifest.is_empty(),
        "MangaDex has no chapters for the selected language"
    );

    let downloader = cancelable(cancelled, MangaDexDownloader::prepare()).await?;
    let download = prepare_download_folder(output, manga_id)?;

    publish(on_event, job_id, MangaDexImportState::Downloading);
    for chapter in &manifest {
        let chapter_download = prepare_chapter_download_folder(&download, &chapter.id)?;
        run_downloader(
            &downloader,
            &chapter_url(&chapter.id),
            language,
            &chapter_download,
            control,
            cancelled,
        )
        .await?;
        rename_downloaded_pages(&chapter_download, chapter)?;
    }

    publish(on_event, job_id, MangaDexImportState::Importing);
    let download = download.clone();
    let pages = cancelable(cancelled, async move {
        Ok(
            tokio_rayon::spawn(move || import_downloaded_chapters(&download, &manifest))
                .await
                .context("MangaDex page import task failed")?,
        )
    })
    .await?;
    ensure!(
        !pages.is_empty(),
        "MangaDex downloader did not produce supported images"
    );
    begin_import_commit(control).await?;
    let result = lifecycle::import_pages(
        handle.state::<koharu_desktop::Desktop>().inner(),
        handle.state::<CurrentProject>().inner(),
        handle.state::<CanvasChannel>().inner(),
        pages,
    )
    .await;
    finish_import_commit(control).await;
    result
}

fn import_downloaded_chapters(
    download: &Path,
    manifest: &[ChapterManifest],
) -> Result<Vec<import::Page>> {
    let mut pages = Vec::new();
    for chapter in manifest {
        pages.append(&mut import::import_folder(&download.join(&chapter.id))?);
    }
    Ok(pages)
}

async fn cancel_transition(control: &MangaDexImportControl) -> Result<()> {
    let mut transition = control.transition.lock().await;
    ensure!(
        matches!(*transition, ImportTransition::Active),
        "MangaDex import is already completing"
    );
    *transition = ImportTransition::Cancelled;
    control.cancelled.send_replace(true);
    Ok(())
}

async fn begin_import_commit(control: &MangaDexImportControl) -> Result<()> {
    let mut transition = control.transition.lock().await;
    match *transition {
        ImportTransition::Active => {
            *transition = ImportTransition::Committing;
            Ok(())
        }
        ImportTransition::Cancelled => bail!("MangaDex import cancelled"),
        ImportTransition::Committing | ImportTransition::Completed => {
            bail!("MangaDex import is already completing")
        }
    }
}

async fn finish_import_commit(control: &MangaDexImportControl) {
    let mut transition = control.transition.lock().await;
    debug_assert!(matches!(*transition, ImportTransition::Committing));
    *transition = ImportTransition::Completed;
}

async fn create_and_open_project(handle: &AppHandle<CefRuntime>, name: &str) -> Result<()> {
    let library = handle.state::<ProjectLibrary>().inner().clone();
    let opened = library.create(name).await?;
    lifecycle::replace_project(handle, opened).await
}

fn publish(channel: &Channel<MangaDexImportEvent>, job_id: &str, state: MangaDexImportState) {
    let _ = channel.send(MangaDexImportEvent {
        job_id: job_id.to_owned(),
        state,
        error: None,
    });
}

async fn cancelable<T>(
    cancelled: &mut watch::Receiver<bool>,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        result = operation => result,
        () = wait_for_cancel(cancelled) => bail!("MangaDex import cancelled"),
    }
}

async fn wait_for_cancel(cancelled: &mut watch::Receiver<bool>) {
    while !*cancelled.borrow_and_update() {
        if cancelled.changed().await.is_err() {
            return;
        }
    }
}

async fn run_downloader(
    downloader: &MangaDexDownloader,
    url: &str,
    language: &str,
    output: &Path,
    control: &MangaDexImportControl,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<()> {
    ensure!(!*cancelled.borrow_and_update(), "MangaDex import cancelled");
    let mut command = downloader.command(url, language, output);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .context("failed to start MangaDex Downloader")?;
    let stdout = child
        .stdout
        .take()
        .context("MangaDex Downloader stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("MangaDex Downloader stderr is unavailable")?;
    let stdout = tokio::spawn(async move {
        let mut output = Vec::new();
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await?;
        Ok::<_, std::io::Error>(output)
    });
    let stderr = tokio::spawn(async move {
        let mut output = Vec::new();
        let mut stderr = stderr;
        stderr.read_to_end(&mut output).await?;
        Ok::<_, std::io::Error>(output)
    });
    *control.child.lock().await = Some(child);

    let status = loop {
        if *cancelled.borrow_and_update() {
            if let Some(child) = control.child.lock().await.as_mut() {
                let _ = child.start_kill();
            }
        }
        let status = control
            .child
            .lock()
            .await
            .as_mut()
            .context("MangaDex Downloader process is unavailable")?
            .try_wait()
            .context("failed to wait for MangaDex Downloader")?;
        if let Some(status) = status {
            break status;
        }
        tokio::select! {
            () = wait_for_cancel(cancelled) => {},
            () = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    };
    control.child.lock().await.take();
    let stdout = stdout
        .await
        .context("MangaDex Downloader stdout task failed")??;
    let stderr = stderr
        .await
        .context("MangaDex Downloader stderr task failed")??;
    if *cancelled.borrow() {
        bail!("MangaDex import cancelled");
    }
    ensure!(
        status.success(),
        "MangaDex Downloader exited with {status}: {}",
        process_output(&stderr, &stdout)
    );
    Ok(())
}

fn process_output(stderr: &[u8], stdout: &[u8]) -> String {
    let output = if stderr.is_empty() { stdout } else { stderr };
    String::from_utf8_lossy(output).trim().to_owned()
}

fn prepare_download_folder(output: &Path, manga_id: &str) -> Result<PathBuf> {
    ensure!(output.is_dir(), "MangaDex output folder does not exist");
    let folder = output.join(format!("mangadex-{manga_id}"));
    ensure!(
        !folder.exists(),
        "MangaDex output folder {} already exists",
        folder.display()
    );
    std::fs::create_dir(&folder).with_context(|| {
        format!(
            "failed to create MangaDex output folder {}",
            folder.display()
        )
    })?;
    Ok(folder)
}

fn prepare_chapter_download_folder(download: &Path, chapter_id: &str) -> Result<PathBuf> {
    let id = Path::new(chapter_id);
    ensure!(
        id.file_name()
            .is_some_and(|name| name == OsStr::new(chapter_id)),
        "MangaDex chapter ID is not a path component"
    );
    let folder = download.join(id);
    std::fs::create_dir(&folder).with_context(|| {
        format!(
            "failed to create MangaDex chapter download folder {}",
            folder.display()
        )
    })?;
    Ok(folder)
}

fn chapter_url(chapter_id: &str) -> String {
    format!("https://mangadex.org/chapter/{chapter_id}")
}

fn rename_downloaded_pages(download: &Path, chapter: &ChapterManifest) -> Result<()> {
    let mut covers = Vec::new();
    let mut pages = WalkDir::new(download)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_file() && is_supported_image(entry.path()) => {
                let path = entry.into_path();
                if is_cover_image(&path) {
                    covers.push(path);
                    None
                } else {
                    Some(Ok(path))
                }
            }
            Ok(_) => None,
            Err(error) => Some(Err(error.into())),
        })
        .collect::<Result<Vec<_>>>()?;
    alphanumeric_sort::sort_slice_by_os_str_key(&mut pages, |path| {
        path.file_name().unwrap_or_else(|| path.as_os_str())
    });
    ensure!(
        pages.len() == chapter.page_count as usize,
        "MangaDex chapter {} has {} downloaded pages, expected {}",
        chapter.id,
        pages.len(),
        chapter.page_count
    );
    let mut renames = Vec::new();
    for (index, source) in pages.into_iter().enumerate() {
        let extension = source
            .extension()
            .context("downloaded image has no extension")?;
        let target = source.with_file_name(format!(
            "chapter {} page {}.{}",
            chapter.number,
            index + 1,
            extension.to_string_lossy()
        ));
        ensure!(
            !target.exists() || target == source,
            "refusing to overwrite downloaded page {}",
            target.display()
        );
        renames.push((source, target));
    }
    let mut staged = Vec::new();
    for (index, (source, target)) in renames.into_iter().enumerate() {
        if source == target {
            continue;
        }
        let temporary =
            source.with_file_name(format!(".koharu-mangadex-{index}-{}", uuid::Uuid::new_v4()));
        if temporary.exists() {
            rollback_renames(&staged, 0);
            bail!(
                "refusing to overwrite temporary downloaded page {}",
                temporary.display()
            );
        }
        if let Err(error) = std::fs::rename(&source, &temporary) {
            rollback_renames(&staged, 0);
            return Err(error).with_context(|| {
                format!(
                    "failed to stage downloaded page {} for renaming",
                    source.display()
                )
            });
        }
        staged.push((source, temporary, target));
    }
    for index in 0..staged.len() {
        let (source, temporary, target) = &staged[index];
        if let Err(error) = std::fs::rename(temporary, target) {
            rollback_renames(&staged, index);
            return Err(error).with_context(|| {
                format!(
                    "failed to rename downloaded page {} to {}",
                    source.display(),
                    target.display()
                )
            });
        }
    }
    for cover in covers {
        std::fs::remove_file(&cover).with_context(|| {
            format!("failed to remove MangaDex cover image {}", cover.display())
        })?;
    }
    Ok(())
}

fn rollback_renames(staged: &[(PathBuf, PathBuf, PathBuf)], completed: usize) {
    for (source, _, target) in staged[..completed].iter().rev() {
        let _ = std::fs::rename(target, source);
    }
    for (source, temporary, _) in staged[completed..].iter().rev() {
        let _ = std::fs::rename(temporary, source);
    }
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

fn is_cover_image(path: &Path) -> bool {
    path.file_stem()
        .and_then(OsStr::to_str)
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cover"))
}

fn manga_id(url: &str) -> Result<String> {
    let url = Url::parse(url).context("MangaDex URL is invalid")?;
    ensure!(url.scheme() == "https", "MangaDex URL must use HTTPS");
    ensure!(
        matches!(url.host_str(), Some("mangadex.org" | "www.mangadex.org")),
        "URL is not a MangaDex URL"
    );
    let mut segments = url
        .path_segments()
        .context("MangaDex URL does not contain a title")?;
    ensure!(
        segments.next() == Some("title"),
        "URL is not a MangaDex title URL"
    );
    let manga_id = segments
        .next()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .context("MangaDex title URL does not contain a valid manga ID")?;
    Ok(manga_id.to_owned())
}

async fn resolve_manga_at(url: &str, api: &str) -> Result<MangaDexMetadata> {
    let manga_id = manga_id(url)?;
    let manga: MangaResponse = request(endpoint(api, &format!("manga/{manga_id}"))?).await?;
    let title = manga
        .data
        .attributes
        .title
        .get("en")
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            manga
                .data
                .attributes
                .title
                .values()
                .find(|title| !title.trim().is_empty())
        })
        .context("MangaDex manga does not have a title")?
        .clone();
    let available_languages = manga
        .data
        .attributes
        .available_translated_languages
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let languages = available_languages
        .into_iter()
        .filter(|code| seen.insert(code.clone()))
        .map(|code| MangaDexLanguage {
            name: language_name(&code).to_owned(),
            code,
        })
        .collect();
    Ok(MangaDexMetadata {
        manga_id: manga.data.id,
        title,
        languages,
    })
}

fn language_name(code: &str) -> &str {
    match code {
        "ar" => "Arabic",
        "az" => "Azerbaijani",
        "bg" => "Bulgarian",
        "bn" => "Bengali",
        "ca" => "Catalan",
        "cs" => "Czech",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "en" => "English",
        "es" => "Spanish",
        "es-la" => "Spanish (Latin America)",
        "fa" => "Persian",
        "fi" => "Finnish",
        "fil" => "Filipino",
        "fr" => "French",
        "he" => "Hebrew",
        "hi" => "Hindi",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "it" => "Italian",
        "ja" => "Japanese",
        "ja-ro" => "Japanese (Romanized)",
        "ko" => "Korean",
        "ko-ro" => "Korean (Romanized)",
        "ms" => "Malay",
        "my" => "Burmese",
        "nl" => "Dutch",
        "no" => "Norwegian",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "pt-br" => "Portuguese (Brazil)",
        "ro" => "Romanian",
        "ru" => "Russian",
        "sr" => "Serbian",
        "sv" => "Swedish",
        "th" => "Thai",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "vi" => "Vietnamese",
        "zh" => "Chinese",
        "zh-hk" => "Chinese (Traditional)",
        "zh-ro" => "Chinese (Romanized)",
        _ => code,
    }
}

async fn chapter_manifest_at(
    manga_id: &str,
    language: &str,
    api: &str,
) -> Result<Vec<ChapterManifest>> {
    let mut manifest = Vec::new();
    let mut offset = 0;
    loop {
        let mut url = endpoint(api, "chapter")?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("manga", manga_id)
                .append_pair("translatedLanguage[]", language)
                .append_pair("limit", &CHAPTER_PAGE_SIZE.to_string())
                .append_pair("offset", &offset.to_string())
                .append_pair("order[volume]", "asc")
                .append_pair("order[chapter]", "asc")
                .append_pair("order[createdAt]", "asc");
        }
        let page: ChapterResponse = request(url).await?;
        let page_len = page.data.len();
        for chapter in page.data {
            let ordinal = manifest.len() + 1;
            manifest.push(ChapterManifest {
                id: chapter.id,
                number: chapter_number(chapter.attributes.chapter.as_deref(), ordinal),
                page_count: chapter.attributes.pages,
            });
        }
        offset += page_len;
        if offset >= page.total {
            return Ok(manifest);
        }
        if page_len == 0 {
            bail!("MangaDex returned an empty chapter page before the collection ended");
        }
    }
}

fn chapter_number(number: Option<&str>, ordinal: usize) -> String {
    number
        .filter(|number| {
            number
                .trim()
                .parse::<f64>()
                .is_ok_and(|number| number.is_finite())
        })
        .map_or_else(|| ordinal.to_string(), ToOwned::to_owned)
}

fn endpoint(api: &str, path: &str) -> Result<Url> {
    Url::parse(api)
        .context("MangaDex API URL is invalid")?
        .join(path)
        .context("MangaDex API endpoint is invalid")
}

async fn request<T: DeserializeOwned>(url: Url) -> Result<T> {
    api_client()?
        .get(url)
        .send()
        .await
        .context("MangaDex API request failed")?
        .error_for_status()
        .context("MangaDex API returned an error")?
        .json()
        .await
        .context("MangaDex API response was invalid")
}

fn api_client() -> Result<Client> {
    if let Some(client) = API_CLIENT.get() {
        return Ok(client.clone());
    }
    let builder = Client::builder()
        .user_agent(concat!("koharu/", env!("CARGO_PKG_VERSION")))
        .http1_only()
        .tls_backend_rustls()
        .tls_version_min(Version::TLS_1_3)
        .tls_version_max(Version::TLS_1_3);
    let client = builder
        .build()
        .context("failed to build MangaDex API client")?;
    Ok(API_CLIENT.get_or_init(|| client).clone())
}

#[derive(Deserialize)]
struct MangaResponse {
    data: Manga,
}

#[derive(Deserialize)]
struct Manga {
    id: String,
    attributes: MangaAttributes,
}

#[derive(Deserialize)]
struct MangaAttributes {
    title: BTreeMap<String, String>,
    #[serde(default, rename = "availableTranslatedLanguages")]
    available_translated_languages: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ChapterResponse {
    data: Vec<Chapter>,
    total: usize,
}

#[derive(Deserialize)]
struct Chapter {
    id: String,
    attributes: ChapterAttributes,
}

#[derive(Deserialize)]
struct ChapterAttributes {
    chapter: Option<String>,
    pages: u32,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::{fs, io::Cursor};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
        task::JoinHandle,
    };

    use super::{
        ChapterManifest, ImportTransition, MangaDexImportControl, MangaDexImportEvent,
        MangaDexImportState, begin_import_commit, cancel_transition, cancelable,
        chapter_manifest_at, chapter_url, finish_import_commit, import_downloaded_chapters,
        manga_id, prepare_chapter_download_folder, prepare_download_folder,
        rename_downloaded_pages, resolve_manga_at, resolve_mangadex,
    };

    const MANGA_ID: &str = "b3f7d2b9-23f5-4b31-a042-40b7943c3983";

    #[test]
    fn renames_raw_pages_in_a_human_readable_chapter_folder_without_changing_extensions() {
        let directory = tempfile::tempdir().expect("create output directory");
        let raw = directory
            .path()
            .join("A Manga")
            .join("Volume 1")
            .join("Chapter 1");
        fs::create_dir_all(&raw).expect("create raw chapter");
        fs::write(directory.path().join("cover.jpg"), b"cover").expect("write cover");
        fs::write(raw.join("z.webp"), b"page").expect("write page");
        fs::write(raw.join("a.PNG"), b"page").expect("write page");

        rename_downloaded_pages(
            directory.path(),
            &ChapterManifest {
                id: "first-id".to_owned(),
                number: "1".to_owned(),
                page_count: 2,
            },
        )
        .expect("rename pages");

        assert!(raw.join("chapter 1 page 1.PNG").is_file());
        assert!(raw.join("chapter 1 page 2.webp").is_file());
        assert!(!directory.path().join("cover.jpg").exists());
    }

    #[test]
    fn keeps_duplicate_chapter_numbers_separate_in_their_id_roots() {
        let directory = tempfile::tempdir().expect("create output directory");
        let first = prepare_chapter_download_folder(directory.path(), "first-id")
            .expect("create first chapter root");
        let second = prepare_chapter_download_folder(directory.path(), "second-id")
            .expect("create second chapter root");
        fs::write(first.join("one.jpg"), b"page").expect("write first page");
        fs::write(second.join("one.jpg"), b"page").expect("write second page");

        rename_downloaded_pages(
            &first,
            &ChapterManifest {
                id: "first-id".to_owned(),
                number: "1".to_owned(),
                page_count: 1,
            },
        )
        .expect("rename first chapter");
        rename_downloaded_pages(
            &second,
            &ChapterManifest {
                id: "second-id".to_owned(),
                number: "1".to_owned(),
                page_count: 1,
            },
        )
        .expect("rename second chapter");

        assert!(first.join("chapter 1 page 1.jpg").is_file());
        assert!(second.join("chapter 1 page 1.jpg").is_file());
    }

    #[test]
    fn imports_duplicate_chapter_labels_in_manifest_order_despite_reversed_uuid_roots() {
        let directory = tempfile::tempdir().expect("create output directory");
        let first_id = "f0000000-0000-0000-0000-000000000000";
        let second_id = "10000000-0000-0000-0000-000000000000";
        let first = prepare_chapter_download_folder(directory.path(), first_id)
            .expect("create first chapter root");
        let second = prepare_chapter_download_folder(directory.path(), second_id)
            .expect("create second chapter root");
        let first_page = png([255, 0, 0, 255]);
        let second_page = png([0, 255, 0, 255]);
        fs::write(first.join("chapter 1 page 1.png"), &first_page).expect("write first page");
        fs::write(second.join("chapter 1 page 1.png"), &second_page).expect("write second page");
        let manifest = [
            ChapterManifest {
                id: first_id.to_owned(),
                number: "1".to_owned(),
                page_count: 1,
            },
            ChapterManifest {
                id: second_id.to_owned(),
                number: "1".to_owned(),
                page_count: 1,
            },
        ];

        let pages =
            import_downloaded_chapters(directory.path(), &manifest).expect("import chapter pages");

        assert_eq!(
            pages
                .iter()
                .map(|page| page.name.as_str())
                .collect::<Vec<_>>(),
            ["chapter 1 page 1.png", "chapter 1 page 1.png"]
        );
        assert_eq!(pages[0].bytes.as_ref(), first_page.as_slice());
        assert_eq!(pages[1].bytes.as_ref(), second_page.as_slice());
    }

    #[test]
    fn downloads_each_manifest_entry_from_its_chapter_url() {
        assert_eq!(
            chapter_url("first-id"),
            "https://mangadex.org/chapter/first-id"
        );
    }

    fn png(pixel: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(pixel));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode fixture");
        encoded.into_inner()
    }

    #[test]
    fn refuses_an_existing_manga_output_folder() {
        let output = tempfile::tempdir().expect("create output directory");
        let existing = output.path().join("mangadex-manga-id");
        fs::create_dir(&existing).expect("create existing download directory");

        let error = prepare_download_folder(output.path(), "manga-id")
            .expect_err("must not overwrite a previous download");

        assert!(error.to_string().contains("already exists"));
        assert!(existing.is_dir());
    }

    #[tokio::test]
    async fn cancellation_interrupts_runtime_setup_before_import() {
        let (sender, mut cancelled) = tokio::sync::watch::channel(false);
        sender.send_replace(true);

        let error = cancelable(&mut cancelled, std::future::pending::<anyhow::Result<()>>())
            .await
            .expect_err("cancelled setup must not continue");

        assert!(error.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn cancellation_interrupts_download_before_import() {
        let (sender, mut cancelled) = tokio::sync::watch::channel(false);
        sender.send_replace(true);

        let error = cancelable(&mut cancelled, std::future::pending::<anyhow::Result<()>>())
            .await
            .expect_err("cancelled download must not continue to import");

        assert!(error.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn commit_transition_rejects_cancellation_after_decoding() {
        let (control, _) = import_control();

        begin_import_commit(&control)
            .await
            .expect("decoded pages claim the commit transition");
        let cancellation = cancel_transition(&control).await;

        assert!(cancellation.is_err());
        finish_import_commit(&control).await;
        assert_eq!(
            *control.transition.lock().await,
            ImportTransition::Completed
        );
    }

    #[tokio::test]
    async fn cancellation_transition_prevents_commit_after_decoding() {
        let (control, mut cancelled) = import_control();

        cancel_transition(&control)
            .await
            .expect("cancellation claims the active transition");
        let commit = begin_import_commit(&control).await;

        assert!(commit.is_err());
        assert!(*cancelled.borrow_and_update());
        assert_eq!(
            *control.transition.lock().await,
            ImportTransition::Cancelled
        );
    }

    fn import_control() -> (MangaDexImportControl, tokio::sync::watch::Receiver<bool>) {
        let (cancelled, receiver) = tokio::sync::watch::channel(false);
        (
            MangaDexImportControl {
                cancelled,
                child: Arc::new(tokio::sync::Mutex::new(None)),
                transition: Arc::new(tokio::sync::Mutex::new(ImportTransition::Active)),
            },
            receiver,
        )
    }

    #[test]
    fn accepts_mangadex_title_urls() {
        for url in [
            format!("https://mangadex.org/title/{MANGA_ID}"),
            format!("https://www.mangadex.org/title/{MANGA_ID}/a-title"),
        ] {
            assert_eq!(manga_id(&url).unwrap(), MANGA_ID);
        }
    }

    #[test]
    fn rejects_non_title_urls() {
        for url in [
            "https://mangadex.org/author/b3f7d2b9-23f5-4b31-a042-40b7943c3983",
            "https://mangadex.org/title/not-an-id",
            "https://example.com/title/b3f7d2b9-23f5-4b31-a042-40b7943c3983",
            "http://mangadex.org/title/b3f7d2b9-23f5-4b31-a042-40b7943c3983",
        ] {
            assert!(manga_id(url).is_err(), "{url} should be rejected");
        }
    }

    #[tokio::test]
    async fn resolve_command_rejects_an_invalid_url_before_requesting_metadata() {
        let error = resolve_mangadex("https://example.com/title/not-a-manga".to_owned())
            .await
            .expect_err("invalid MangaDex URL must be rejected");

        assert!(
            serde_json::to_value(&error)
                .expect("serialize command error")
                .as_str()
                .expect("command error is a string")
                .contains("MangaDex")
        );
    }

    #[test]
    fn import_events_serialize_for_the_typed_protocol() {
        let event = MangaDexImportEvent {
            job_id: "job-123".to_owned(),
            state: MangaDexImportState::RuntimeSetup,
            error: None,
        };

        assert_eq!(
            serde_json::to_value(event).expect("serialize import event"),
            serde_json::json!({
                "job_id": "job-123",
                "state": "runtime_setup",
                "error": null,
            })
        );
    }

    #[tokio::test]
    async fn resolves_localized_languages_without_a_registry_request() {
        let server = FixtureServer::start(|target| match target {
            path if path.starts_with(&format!("/manga/{MANGA_ID}")) => manga(
                r#"{"ja":"日本語の題名","en":"English title"}"#,
                r#"["en","ja","en","x-klingon"]"#,
            ),
            _ => failure(),
        })
        .await;

        let metadata = resolve_manga_at(
            &format!("https://mangadex.org/title/{MANGA_ID}/ignored-slug"),
            &server.base(),
        )
        .await
        .unwrap();

        assert_eq!(metadata.manga_id, MANGA_ID);
        assert_eq!(metadata.title, "English title");
        assert_eq!(
            metadata
                .languages
                .into_iter()
                .map(|language| (language.code, language.name))
                .collect::<Vec<_>>(),
            [
                ("en".into(), "English".into()),
                ("ja".into(), "Japanese".into()),
                ("x-klingon".into(), "x-klingon".into()),
            ]
        );
        assert_eq!(server.requests().await, vec![format!("/manga/{MANGA_ID}")]);
    }

    #[tokio::test]
    async fn preserves_empty_language_metadata() {
        let server = FixtureServer::start(|target| match target {
            path if path.starts_with(&format!("/manga/{MANGA_ID}")) => {
                manga(r#"{"en":"Untitled"}"#, "[]")
            }
            _ => failure(),
        })
        .await;

        let metadata = resolve_manga_at(
            &format!("https://mangadex.org/title/{MANGA_ID}"),
            &server.base(),
        )
        .await
        .unwrap();

        assert!(metadata.languages.is_empty());
        assert_eq!(server.requests().await.len(), 1);
    }

    #[tokio::test]
    async fn chapter_manifest_paginates_and_retains_ids_for_duplicate_numbers() {
        let server = FixtureServer::start(|target| {
            if target.starts_with("/chapter?") && target.contains("offset=0") {
                chapters(
                    4,
                    0,
                    r#"[
                        {"id":"first-group","attributes":{"chapter":"1","pages":20}},
                        {"id":"second-group","attributes":{"chapter":"1","pages":18}}
                    ]"#,
                )
            } else if target.starts_with("/chapter?") && target.contains("offset=2") {
                chapters(
                    4,
                    2,
                    r#"[
                        {"id":"unlabelled","attributes":{"chapter":null,"pages":12}},
                        {"id":"special","attributes":{"chapter":"extra","pages":9}}
                    ]"#,
                )
            } else {
                failure()
            }
        })
        .await;

        let manifest = chapter_manifest_at(MANGA_ID, "en", &server.base())
            .await
            .unwrap();

        assert_eq!(
            manifest
                .into_iter()
                .map(|chapter| (chapter.id, chapter.number, chapter.page_count))
                .collect::<Vec<_>>(),
            [
                ("first-group".into(), "1".into(), 20),
                ("second-group".into(), "1".into(), 18),
                ("unlabelled".into(), "3".into(), 12),
                ("special".into(), "4".into(), 9),
            ]
        );
        let requests = server.requests().await;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.contains("translatedLanguage%5B%5D=en"))
        );
        assert!(requests[0].contains("offset=0"));
        assert!(requests[1].contains("offset=2"));
    }

    fn manga(title: &str, languages: &str) -> String {
        format!(
            r#"{{"data":{{"id":"{MANGA_ID}","attributes":{{"title":{title},"availableTranslatedLanguages":{languages}}}}}}}"#
        )
    }

    fn chapters(total: usize, offset: usize, data: &str) -> String {
        format!(r#"{{"data":{data},"total":{total},"offset":{offset}}}"#)
    }

    fn failure() -> String {
        r#"{"errors":[{"title":"unexpected fixture request"}]}"#.into()
    }

    struct FixtureServer {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: JoinHandle<()>,
    }

    impl FixtureServer {
        async fn start(response: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            let response = Arc::new(response);
            let task = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let response = Arc::clone(&response);
                    let requests = Arc::clone(&server_requests);
                    tokio::spawn(async move {
                        let mut request = vec![0; 4096];
                        let read = stream.read(&mut request).await.unwrap();
                        let target = std::str::from_utf8(&request[..read])
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                            .to_owned();
                        requests.lock().await.push(target.clone());
                        let body = response(&target);
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(reply.as_bytes()).await.unwrap();
                    });
                }
            });
            Self {
                base,
                requests,
                task,
            }
        }

        fn base(&self) -> String {
            self.base.clone()
        }

        async fn requests(&self) -> Vec<String> {
            self.requests.lock().await.clone()
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
}
