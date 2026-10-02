//! Addic7ed TV subtitles through Gestdown (<https://api.gestdown.info>), a public API that
//! mirrors Addic7ed. No key needed. Only episodes: movies are skipped.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{BoxFuture, Candidate, Downloaded, Provider, SearchQuery, user_agent};
use crate::{Error, Result, archive, lang, name, score};

pub const NAME: &str = "Addic7ed";
const API: &str = "https://api.gestdown.info";

pub struct Gestdown {
    client: Client,
    /// Show search results by title, so a season is one search, not one per episode.
    shows: Mutex<HashMap<String, Vec<Show>>>,
}

impl Default for Gestdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Gestdown {
    pub fn new() -> Self {
        let client =
            Client::builder().user_agent(user_agent()).timeout(Duration::from_secs(30)).build().expect("HTTP client");
        Gestdown { client, shows: Mutex::new(HashMap::new()) }
    }

    async fn find_shows(&self, title: &str) -> Result<Vec<Show>> {
        let key = title.to_lowercase();
        if let Some(found) = self.shows.lock().await.get(&key) {
            return Ok(found.clone());
        }
        let mut url = Url::parse(API).expect("API url");
        url.path_segments_mut().expect("base url").extend(["shows", "search", title]);
        let resp = self.client.get(url).header("Accept", "application/json").send().await?;
        let shows = match resp.status() {
            StatusCode::NOT_FOUND => Vec::new(),
            s if s.is_success() => resp.json::<ShowSearch>().await?.shows,
            s => return Err(api_error(s, resp.text().await.unwrap_or_default())),
        };
        self.shows.lock().await.insert(key, shows.clone());
        Ok(shows)
    }

    async fn do_search(&self, q: &SearchQuery) -> Result<Vec<Candidate>> {
        let (Some(title), Some(season), Some(episode)) = (&q.name.title, q.name.season, q.name.episode) else {
            return Ok(Vec::new());
        };
        if !q.name.is_episode {
            return Ok(Vec::new());
        }
        let shows = self.find_shows(title).await?;
        let Some(show) = pick_show(&shows, title, &q.file_name, season) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for code in &q.languages {
            let Some(language) = language_name(code) else { continue };
            let mut url = Url::parse(API).expect("API url");
            url.path_segments_mut().expect("base url").extend([
                "subtitles",
                "get",
                &show.id,
                &season.to_string(),
                &episode.to_string(),
                language,
            ]);
            let resp = self.client.get(url).header("Accept", "application/json").send().await?;
            match resp.status() {
                StatusCode::NOT_FOUND => continue,
                s if s.is_success() => {
                    let found: EpisodeSubtitles = resp.json().await?;
                    out.extend(found.into_candidates(season, episode));
                }
                s => return Err(api_error(s, resp.text().await.unwrap_or_default())),
            }
        }
        Ok(out)
    }

    async fn do_download(&self, c: &Candidate) -> Result<Downloaded> {
        let resp = self.client.get(format!("{API}{}", c.id)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(api_error(status, resp.text().await.unwrap_or_default()));
        }
        let name = file_name_from(resp.headers()).unwrap_or_else(|| format!("{}.srt", c.release));
        let bytes = resp.bytes().await?.to_vec();
        Ok(Downloaded { files: archive::extract(&name, bytes)?, remaining: None })
    }
}

impl Provider for Gestdown {
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

fn api_error(status: StatusCode, body: String) -> Error {
    let message = serde_json::from_str::<String>(&body).unwrap_or(body);
    let message = if message.trim().is_empty() || message.len() > 300 {
        status.canonical_reason().unwrap_or("error").to_owned()
    } else {
        message.trim().to_owned()
    };
    Error::Api { provider: NAME, status: status.as_u16(), message }
}

/// Addic7ed language names.
fn language_name(code: &str) -> Option<&'static str> {
    match code {
        "pt-br" => Some("Portuguese (Brazilian)"),
        "pt-pt" => Some("Portuguese"),
        "zh-cn" => Some("Chinese (Simplified)"),
        "zh-tw" => Some("Chinese (Traditional)"),
        other => lang::find(other).map(|l| l.name),
    }
}

fn language_code(name: &str) -> Option<&'static str> {
    match name {
        "Portuguese (Brazilian)" => Some("pt-br"),
        "Portuguese" => Some("pt-pt"),
        "Chinese (Simplified)" => Some("zh-cn"),
        "Chinese (Traditional)" => Some("zh-tw"),
        other => lang::find(other).map(|l| l.code),
    }
}

/// `attachment; filename="x.srt"; filename*=…` → `x.srt`.
fn file_name_from(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers.get(reqwest::header::CONTENT_DISPOSITION)?.to_str().ok()?;
    let start = value.find("filename=\"")? + "filename=\"".len();
    let end = value[start..].find('"')? + start;
    Some(value[start..end].to_owned())
}

/// The show that fits the file: same title, and a "(US)" / "(2005)" tag that the file name also
/// has. Ties go to the show that has the season, then to the longer-running one.
fn pick_show<'a>(shows: &'a [Show], title: &str, file_name: &str, season: i32) -> Option<&'a Show> {
    let file_tokens = name::tokens(file_name);
    let rank = |s: &Show| {
        let (base, tag) = match s.name.split_once('(') {
            Some((b, t)) => (b.trim(), Some(t.trim_end_matches(')').to_lowercase())),
            None => (s.name.as_str(), None),
        };
        let mut r = score::similarity(base, title);
        match tag {
            Some(t) if file_tokens.contains(&t) => r += 0.5,
            Some(_) => r -= 0.05,
            None => {}
        }
        (r, s.seasons.contains(&season), s.nb_seasons)
    };
    shows
        .iter()
        .filter(|s| score::similarity(s.name.split('(').next().unwrap_or_default(), title) >= 0.5)
        .max_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal))
}

#[derive(Deserialize)]
struct ShowSearch {
    #[serde(default)]
    shows: Vec<Show>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Show {
    id: String,
    name: String,
    #[serde(default)]
    nb_seasons: i32,
    #[serde(default)]
    seasons: Vec<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeSubtitles {
    #[serde(default)]
    matching_subtitles: Vec<Sub>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sub {
    version: Option<String>,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    hearing_impaired: bool,
    download_uri: String,
    language: String,
    #[serde(default)]
    download_count: u64,
    #[serde(default)]
    qualities: Vec<String>,
}

impl EpisodeSubtitles {
    fn into_candidates(self, season: i32, episode: i32) -> Vec<Candidate> {
        self.matching_subtitles
            .into_iter()
            .filter(|s| s.completed)
            .filter_map(|s| {
                let language = language_code(&s.language)?.to_owned();
                let mut release = s.version.unwrap_or_default();
                if !s.qualities.is_empty() {
                    release = format!("{release} {}", s.qualities.join(" "));
                }
                Some(Candidate {
                    provider: NAME.into(),
                    id: s.download_uri,
                    language,
                    release,
                    season: Some(season),
                    episode: Some(episode),
                    downloads: s.download_count,
                    hearing_impaired: s.hearing_impaired,
                    ..Default::default()
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real api.gestdown.info answers (2026-10-02).
    const SHOWS: &str = r#"{"shows":[
      {"id":"8ddf8c80-e207-46c5-9cb6-6c4656c8de12","name":"The Office (UK)","nbSeasons":4,"seasons":[0,1,2,3],"tvDbId":78107,"tmdbId":2996,"slug":"the-office-uk"},
      {"id":"f5867464-7071-4a96-a596-886d2d67605e","name":"The Office (US)","nbSeasons":9,"seasons":[1,2,3,4,5,6,7,8,9],"tvDbId":73244,"tmdbId":2316,"slug":"the-office-us"},
      {"id":"019c78ed-0a64-77c9-a1fb-6150581980d7","name":"The Office (AU)","nbSeasons":1,"seasons":[1],"tvDbId":453455,"tmdbId":256480,"slug":"the-office-au"}]}"#;
    const EPISODE: &str = r#"{"matchingSubtitles":[
      {"subtitleId":"0b204495","version":"NOTV","completed":true,"hearingImpaired":false,"corrected":true,"hd":false,"downloadUri":"/subtitles/download/0b204495-dc52-41a0-98e5-17815ba3ba6c","language":"English","discovered":"2022-05-22T05:50:24.935625Z","downloadCount":720,"source":"Addic7ed","qualities":[],"release":null},
      {"subtitleId":"fdcc6d79","version":"720p.Web-Dl.Extended","completed":true,"hearingImpaired":true,"corrected":true,"hd":true,"downloadUri":"/subtitles/download/fdcc6d79","language":"English","discovered":"2022-05-22T05:50:24.935632Z","downloadCount":642,"source":"Addic7ed","qualities":["720p","1080p"],"release":null},
      {"subtitleId":"x","version":"LOL","completed":false,"hearingImpaired":false,"downloadUri":"/subtitles/download/x","language":"Turkish","downloadCount":1,"qualities":[]}],
      "episode":{"season":3,"number":7,"title":"Branch closing","show":"The Office (US)","discovered":"2022-05-22T05:50:24.935623Z"}}"#;

    #[test]
    fn picks_the_show_from_the_file_name() {
        let shows = serde_json::from_str::<ShowSearch>(SHOWS).unwrap().shows;
        let us = pick_show(&shows, "The Office", "The.Office.US.S03E07.720p.WEB-DL.mkv", 3).unwrap();
        assert_eq!(us.name, "The Office (US)");
        let uk = pick_show(&shows, "The Office", "The.Office.UK.S02E01.mkv", 2).unwrap();
        assert_eq!(uk.name, "The Office (UK)");
        // No tag in the file name: the show that has season 5.
        assert_eq!(pick_show(&shows, "The Office", "The.Office.S05E01.mkv", 5).unwrap().name, "The Office (US)");
        assert!(pick_show(&shows, "Breaking Bad", "Breaking.Bad.S01E01.mkv", 1).is_none());
    }

    #[test]
    fn reads_episode_subtitles() {
        let c = serde_json::from_str::<EpisodeSubtitles>(EPISODE).unwrap().into_candidates(3, 7);
        assert_eq!(c.len(), 2, "unfinished translations are skipped");
        assert_eq!(c[0].language, "en");
        assert_eq!(c[0].release, "NOTV");
        assert_eq!(c[0].downloads, 720);
        assert_eq!(c[1].release, "720p.Web-Dl.Extended 720p 1080p");
        assert!(c[1].hearing_impaired);
        assert_eq!((c[1].season, c[1].episode), (Some(3), Some(7)));
    }

    #[test]
    fn maps_languages() {
        assert_eq!(language_name("tr"), Some("Turkish"));
        assert_eq!(language_name("pt-br"), Some("Portuguese (Brazilian)"));
        assert_eq!(language_code("Turkish"), Some("tr"));
        assert_eq!(language_code("Portuguese (Brazilian)"), Some("pt-br"));
    }

    #[test]
    fn reads_the_file_name_header() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::CONTENT_DISPOSITION,
            "attachment; filename=\"The.Office.(US).S03E07.NOTV.en.srt\"; filename*=UTF-8''x".parse().unwrap(),
        );
        assert_eq!(file_name_from(&h).as_deref(), Some("The.Office.(US).S03E07.NOTV.en.srt"));
    }

    /// Live check against api.gestdown.info; runs only with `SUBMAGICIAN_LIVE=1`.
    #[tokio::test]
    async fn live_search_and_download() {
        if !std::env::var("SUBMAGICIAN_LIVE").is_ok_and(|v| v == "1") {
            return;
        }
        let g = Gestdown::new();
        let file = "The.Office.US.S03E07.720p.WEB-DL.mkv";
        let q = SearchQuery {
            file_name: file.into(),
            size: 0,
            hash: None,
            name: name::parse(file),
            languages: vec!["en".into()],
        };
        let found = g.search(&q).await.unwrap();
        assert!(!found.is_empty());
        let d = g.download(&found[0]).await.unwrap();
        assert!(String::from_utf8_lossy(&d.files[0].bytes).contains("-->"));
    }
}
