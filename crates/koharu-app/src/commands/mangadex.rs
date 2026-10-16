use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};
use koharu_runtime::http_client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use specta::Type;
use url::Url;

const API: &str = "https://api.mangadex.org";
const CHAPTER_PAGE_SIZE: usize = 100;

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

pub(crate) async fn chapter_manifest(manga_id: &str, language: &str) -> Result<Vec<ChapterManifest>> {
    chapter_manifest_at(manga_id, language, API).await
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
    if available_languages.is_empty() {
        return Ok(MangaDexMetadata {
            manga_id: manga.data.id,
            title,
            languages: Vec::new(),
        });
    }
    let language_names: Vec<LanguageResponse> = request(endpoint(api, "static/languages")?).await?;
    let language_names = language_names
        .into_iter()
        .map(|language| (language.locale, language.name))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let languages = available_languages
        .into_iter()
        .filter(|code| seen.insert(code.clone()))
        .map(|code| MangaDexLanguage {
            name: language_names
                .get(&code)
                .cloned()
                .unwrap_or_else(|| code.clone()),
            code,
        })
        .collect();
    Ok(MangaDexMetadata {
        manga_id: manga.data.id,
        title,
        languages,
    })
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
        bail!("MangaDex returned an empty chapter page before the collection ended");
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
    http_client()?
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
struct LanguageResponse {
    locale: String,
    name: String,
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

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
        task::JoinHandle,
    };

    use super::{chapter_manifest_at, manga_id, resolve_manga_at};

    const MANGA_ID: &str = "b3f7d2b9-23f5-4b31-a042-40b7943c3983";

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
    async fn resolves_english_title_and_unique_named_languages() {
        let server = FixtureServer::start(|target| match target {
            path if path.starts_with(&format!("/manga/{MANGA_ID}")) => manga(
                r#"{"ja":"日本語の題名","en":"English title"}"#,
                r#"["en","ja","en"]"#,
            ),
            "/static/languages" => languages(),
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
            [("en".into(), "English".into()), ("ja".into(), "Japanese".into())]
        );
    }

    #[tokio::test]
    async fn preserves_empty_language_metadata() {
        let server = FixtureServer::start(|target| match target {
            path if path.starts_with(&format!("/manga/{MANGA_ID}")) => {
                manga(r#"{"en":"Untitled"}"#, "[]")
            }
            "/static/languages" => languages(),
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
        assert!(requests.iter().all(|request| request.contains("translatedLanguage%5B%5D=en")));
        assert!(requests[0].contains("offset=0"));
        assert!(requests[1].contains("offset=2"));
    }

    fn manga(title: &str, languages: &str) -> String {
        format!(
            r#"{{"data":{{"id":"{MANGA_ID}","attributes":{{"title":{title},"availableTranslatedLanguages":{languages}}}}}}}"#
        )
    }

    fn languages() -> String {
        r#"[{"locale":"en","name":"English"},{"locale":"ja","name":"Japanese"}]"#.into()
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
