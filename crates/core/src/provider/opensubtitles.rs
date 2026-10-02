//! OpenSubtitles.com REST API (<https://opensubtitles.stoplight.io/docs/opensubtitles-api>).
//!
//! The API key belongs to the application, not to the user (VLSub does the same): it is built
//! in from `SUBMAGICIAN_OPENSUBTITLES_API_KEY` at compile time and can be overridden in the
//! settings. Logging in is optional and only raises the daily download limit.

use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use super::{BoxFuture, Candidate, Downloaded, Provider, SearchQuery, user_agent};
use crate::archive;
use crate::{Error, Result};

pub const NAME: &str = "OpenSubtitles";
const API: &str = "https://api.opensubtitles.com/api/v1";

/// Built-in application key, if this build has one.
pub const BUILT_IN_KEY: Option<&str> = option_env!("SUBMAGICIAN_OPENSUBTITLES_API_KEY");

#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

pub struct OpenSubtitles {
    client: Client,
    api_key: Option<String>,
    credentials: Option<Credentials>,
    session: Mutex<Option<Session>>,
}

#[derive(Debug, Clone)]
struct Session {
    token: String,
    base: String,
}

impl OpenSubtitles {
    /// `api_key` overrides the built-in key when set; `credentials` are used to log in.
    pub fn new(api_key: Option<String>, credentials: Option<Credentials>) -> Self {
        let client =
            Client::builder().user_agent(user_agent()).timeout(Duration::from_secs(30)).build().expect("HTTP client");
        let api_key = api_key.filter(|k| !k.trim().is_empty()).or(BUILT_IN_KEY.map(str::to_owned));
        let credentials = credentials.filter(|c| !c.username.is_empty() && !c.password.is_empty());
        OpenSubtitles { client, api_key, credentials, session: Mutex::new(None) }
    }

    fn key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| Error::NotConfigured {
            provider: NAME,
            message: "no API key (set one in Settings or build with SUBMAGICIAN_OPENSUBTITLES_API_KEY)".into(),
        })
    }

    /// Logs in once if credentials are set; returns the token and the API base to use.
    async fn session(&self) -> Result<Option<Session>> {
        let Some(creds) = &self.credentials else { return Ok(None) };
        let mut guard = self.session.lock().await;
        if let Some(s) = guard.as_ref() {
            return Ok(Some(s.clone()));
        }
        let resp = self
            .client
            .post(format!("{API}/login"))
            .header("Api-Key", self.key()?)
            .header("Accept", "application/json")
            .json(&json!({ "username": creds.username, "password": creds.password }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let message = error_message(resp).await;
            return Err(Error::Auth { provider: NAME, message });
        }
        let login: LoginResponse = resp.json().await?;
        let base = match login.base_url.as_deref() {
            Some(host) if !host.is_empty() => format!("https://{}/api/v1", host.trim_start_matches("https://")),
            _ => API.to_owned(),
        };
        let session = Session { token: login.token, base };
        *guard = Some(session.clone());
        Ok(Some(session))
    }

    async fn get_subtitles(&self, params: &[(&str, String)]) -> Result<Vec<Candidate>> {
        // The API wants parameters sorted and lowercase, or it answers with a redirect.
        let mut params: Vec<(&str, String)> =
            params.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (*k, v.to_lowercase())).collect();
        params.sort_by(|a, b| a.0.cmp(b.0));
        let resp = self
            .client
            .get(format!("{API}/subtitles"))
            .header("Api-Key", self.key()?)
            .header("Accept", "application/json")
            .query(&params)
            .send()
            .await?;
        let resp = check(resp).await?;
        let body: SearchResponse = resp.json().await?;
        Ok(body.into_candidates())
    }

    async fn do_search(&self, q: &SearchQuery) -> Result<Vec<Candidate>> {
        let languages = q.languages.join(",");
        let mut found = Vec::new();
        if let Some(hash) = &q.hash {
            found = self.get_subtitles(&[("languages", languages.clone()), ("moviehash", hash.clone())]).await?;
        }
        let title = q.name.title.clone().unwrap_or_else(|| q.file_name.clone());
        let mut params = vec![("languages", languages), ("query", title)];
        if q.name.is_episode {
            params.push(("type", "episode".into()));
            params.extend(q.name.season.map(|s| ("season_number", s.to_string())));
            params.extend(q.name.episode.map(|e| ("episode_number", e.to_string())));
        } else {
            params.extend(q.name.year.map(|y| ("year", y.to_string())));
        }
        for c in self.get_subtitles(&params).await? {
            match found.iter_mut().find(|f| f.id == c.id) {
                Some(f) => f.hash_match |= c.hash_match,
                None => found.push(c),
            }
        }
        Ok(found)
    }

    async fn do_download(&self, c: &Candidate) -> Result<Downloaded> {
        let file_id: u64 = c.id.parse().map_err(|_| Error::Parse(format!("bad file id {}", c.id)))?;
        let session = self.session().await?;
        let base = session.as_ref().map_or(API, |s| s.base.as_str());
        let mut req = self
            .client
            .post(format!("{base}/download"))
            .header("Api-Key", self.key()?)
            .header("Accept", "application/json")
            .json(&json!({ "file_id": file_id }));
        if let Some(s) = &session {
            req = req.bearer_auth(&s.token);
        }
        let resp = req.send().await?;
        let resp = check(resp).await?;
        let link: DownloadResponse = resp.json().await?;
        let bytes = self.client.get(&link.link).send().await?.error_for_status()?.bytes().await?.to_vec();
        let name = link.file_name.or_else(|| c.file_name.clone()).unwrap_or_else(|| format!("{}.srt", c.id));
        let files = archive::extract(&name, bytes)?;
        Ok(Downloaded { files, remaining: link.remaining })
    }
}

impl Provider for OpenSubtitles {
    fn name(&self) -> &'static str {
        NAME
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<Candidate>>> {
        Box::pin(self.do_search(query))
    }

    fn download<'a>(&'a self, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded>> {
        Box::pin(self.do_download(candidate))
    }
}

async fn check(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let message = error_message(resp).await;
    Err(match status {
        StatusCode::NOT_ACCEPTABLE => Error::Quota { provider: NAME, message },
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::Auth { provider: NAME, message },
        _ => Error::Api { provider: NAME, status: status.as_u16(), message },
    })
}

async fn error_message(resp: Response) -> String {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    match serde_json::from_str::<ErrorBody>(&text) {
        Ok(ErrorBody { message: Some(m), .. }) => m,
        Ok(ErrorBody { errors: Some(e), .. }) if !e.is_empty() => e.join("; "),
        _ if !text.trim().is_empty() && text.len() < 300 => text.trim().to_owned(),
        _ => status.canonical_reason().unwrap_or("error").to_owned(),
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    message: Option<String>,
    errors: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct DownloadResponse {
    link: String,
    file_name: Option<String>,
    remaining: Option<i64>,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<SubtitleItem>,
}

#[derive(Deserialize)]
struct SubtitleItem {
    attributes: Attributes,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Attributes {
    language: Option<String>,
    download_count: u64,
    hearing_impaired: bool,
    fps: Option<f32>,
    ratings: f32,
    from_trusted: bool,
    ai_translated: bool,
    machine_translated: bool,
    release: Option<String>,
    uploader: Option<Uploader>,
    feature_details: Option<FeatureDetails>,
    files: Vec<FileItem>,
    moviehash_match: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Uploader {
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FeatureDetails {
    season_number: Option<i32>,
    episode_number: Option<i32>,
}

#[derive(Deserialize)]
struct FileItem {
    file_id: u64,
    file_name: Option<String>,
}

impl SearchResponse {
    fn into_candidates(self) -> Vec<Candidate> {
        let mut out = Vec::new();
        for item in self.data {
            let a = item.attributes;
            // Multi-CD subtitles (one file per CD) don't fit a single video; skip them.
            let [file] = a.files.as_slice() else { continue };
            let feature = a.feature_details.unwrap_or_default();
            out.push(Candidate {
                provider: NAME.into(),
                id: file.file_id.to_string(),
                language: a.language.unwrap_or_default().to_lowercase(),
                release: a.release.unwrap_or_default(),
                file_name: file.file_name.clone(),
                hash_match: a.moviehash_match,
                season: feature.season_number,
                episode: feature.episode_number,
                fps: a.fps.filter(|f| *f > 0.0),
                downloads: a.download_count,
                rating: a.ratings,
                hearing_impaired: a.hearing_impaired,
                machine_translated: a.machine_translated || a.ai_translated,
                trusted: a.from_trusted,
                uploader: a.uploader.and_then(|u| u.name),
                score: 0,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_search(json: &str) -> Vec<Candidate> {
        serde_json::from_str::<SearchResponse>(json).unwrap().into_candidates()
    }

    // Shape taken from the API documentation's /subtitles example, trimmed.
    const SEARCH: &str = r#"{
      "total_pages": 1, "total_count": 3, "per_page": 60, "page": 1,
      "data": [
        { "id": "5491112", "type": "subtitle", "attributes": {
            "subtitle_id": "5491112", "language": "tr", "download_count": 12345,
            "hearing_impaired": false, "hd": true, "fps": 23.976, "votes": 3, "ratings": 8.5,
            "from_trusted": true, "foreign_parts_only": false, "ai_translated": false,
            "machine_translated": false, "release": "Inception.2010.1080p.BluRay.x264-SPARKS",
            "uploader": { "uploader_id": 1, "name": "someone", "rank": "trusted" },
            "feature_details": { "feature_id": 1, "feature_type": "Movie", "year": 2010,
              "title": "Inception", "movie_name": "2010 - Inception", "imdb_id": 1375666 },
            "files": [ { "file_id": 5980334, "cd_number": 1, "file_name": "Inception.2010.1080p.BluRay.x264-SPARKS.srt" } ],
            "moviehash_match": true } },
        { "id": "2", "type": "subtitle", "attributes": {
            "language": "EN", "download_count": 5, "fps": 0, "ratings": 0, "release": "Inception.CD1-CD2",
            "files": [ { "file_id": 10, "cd_number": 1 }, { "file_id": 11, "cd_number": 2 } ] } },
        { "id": "3", "type": "subtitle", "attributes": {
            "language": "en", "download_count": 7, "fps": 25.0, "ratings": 0.0, "machine_translated": true,
            "release": null, "uploader": null, "feature_details": null,
            "files": [ { "file_id": 12, "file_name": null } ] } }
      ]
    }"#;

    #[test]
    fn parses_search_response() {
        let c = parse_search(SEARCH);
        assert_eq!(c.len(), 2, "multi-CD entries are skipped");
        assert_eq!(c[0].id, "5980334");
        assert_eq!(c[0].language, "tr");
        assert!(c[0].hash_match && c[0].trusted);
        assert_eq!(c[0].fps, Some(23.976));
        assert_eq!(c[0].downloads, 12345);
        assert_eq!(c[1].id, "12");
        assert!(c[1].machine_translated && !c[1].hash_match);
        assert_eq!(c[1].release, "");
    }

    #[test]
    fn missing_key_is_reported() {
        if BUILT_IN_KEY.is_some() {
            return;
        }
        let os = OpenSubtitles::new(None, None);
        assert!(matches!(os.key(), Err(Error::NotConfigured { .. })));
    }
}
