//! SubDL (<https://subdl.com/api-doc>): JSON API, application key built in from
//! `SUBMAGICIAN_SUBDL_API_KEY` like OpenSubtitles' (settings can override it). Downloads are zips.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::{BoxFuture, Candidate, Downloaded, Provider, SearchQuery};
use crate::{Error, Result, archive};

pub const NAME: &str = "SubDL";
const API: &str = "https://api.subdl.com/api/v1/subtitles";
const DOWNLOAD: &str = "https://dl.subdl.com";

/// Built-in application key, if this build has one.
pub const BUILT_IN_KEY: Option<&str> = option_env!("SUBMAGICIAN_SUBDL_API_KEY");

pub struct SubDl {
    client: Client,
    api_key: String,
}

impl SubDl {
    /// `None` when there is neither a key in the settings nor a built-in one.
    pub fn new(api_key: Option<String>) -> Option<Self> {
        let api_key = api_key.filter(|k| !k.trim().is_empty()).or(BUILT_IN_KEY.map(str::to_owned))?;
        let client = crate::net::client(Some(Duration::from_secs(30))).expect("HTTP client");
        Some(SubDl { client, api_key })
    }

    async fn do_search(&self, q: &SearchQuery) -> Result<Vec<Candidate>> {
        let languages: Vec<&str> = q.languages.iter().filter_map(|c| language_param(c)).collect();
        if languages.is_empty() {
            return Ok(Vec::new());
        }
        let title = q.name.title.clone().unwrap_or_else(|| q.file_name.clone());
        let mut params = vec![
            ("api_key", self.api_key.clone()),
            ("film_name", title),
            ("languages", languages.join(",")),
            ("subs_per_page", "30".into()),
            ("hi", "1".into()),
        ];
        if q.name.is_episode {
            params.push(("type", "tv".into()));
            params.extend(q.name.season.map(|s| ("season_number", s.to_string())));
            params.extend(q.name.episode.map(|e| ("episode_number", e.to_string())));
        } else {
            params.push(("type", "movie".into()));
            params.extend(q.name.year.map(|y| ("year", y.to_string())));
        }
        let resp = self.client.get(API).query(&params).send().await?;
        let status = resp.status();
        let body: Response = resp.json().await.map_err(|e| Error::Parse(format!("{NAME}: {e}")))?;
        if !body.status {
            let message = body.message.or(body.error).unwrap_or_default();
            return match body.status_code.unwrap_or(status.as_u16()) {
                401 | 403 => Err(Error::Auth { provider: NAME, message }),
                429 => Err(Error::Quota { provider: NAME, message }),
                // "can't find movie or tv" and the like: nothing for this title.
                _ if status.is_success() || status.as_u16() == 404 => Ok(Vec::new()),
                code => Err(Error::Api { provider: NAME, status: code, message }),
            };
        }
        Ok(body.subtitles.into_iter().filter_map(Sub::into_candidate).collect())
    }

    async fn do_download(&self, c: &Candidate) -> Result<Downloaded> {
        let bytes = self.client.get(format!("{DOWNLOAD}{}", c.id)).send().await?.error_for_status()?.bytes().await?;
        let name = format!("{}.zip", c.release);
        Ok(Downloaded { files: archive::extract(&name, bytes.to_vec())?, remaining: None })
    }
}

impl Provider for SubDl {
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

/// SubDL language codes are upper case, with a few of their own.
fn language_param(code: &str) -> Option<&'static str> {
    Some(match code {
        "pt-br" => "BR_PT",
        "pt-pt" => "PT",
        "zh-cn" => "ZH",
        "zh-tw" => "ZH_BG",
        "tr" => "TR",
        "en" => "EN",
        "de" => "DE",
        "fr" => "FR",
        "es" => "ES",
        "it" => "IT",
        "nl" => "NL",
        "pl" => "PL",
        "cs" => "CS",
        "hu" => "HU",
        "ro" => "RO",
        "bg" => "BG",
        "el" => "EL",
        "ru" => "RU",
        "uk" => "UK",
        "ar" => "AR",
        "fa" => "FA",
        "he" => "HE",
        "az" => "AZ",
        "sv" => "SV",
        "da" => "DA",
        "no" => "NO",
        "fi" => "FI",
        "ja" => "JA",
        "ko" => "KO",
        _ => return None,
    })
}

fn language_code(param: &str) -> Option<&'static str> {
    match param.to_ascii_uppercase().as_str() {
        "BR_PT" => Some("pt-br"),
        "PT" => Some("pt-pt"),
        "ZH" => Some("zh-cn"),
        "ZH_BG" => Some("zh-tw"),
        other => crate::lang::find(other).map(|l| l.code),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    #[serde(default)]
    status: bool,
    status_code: Option<u16>,
    error: Option<String>,
    message: Option<String>,
    #[serde(default)]
    subtitles: Vec<Sub>,
}

#[derive(Deserialize)]
struct Sub {
    release_name: Option<String>,
    language: Option<String>,
    url: Option<String>,
    author: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
    #[serde(default)]
    hi: bool,
    #[serde(default)]
    full_season: bool,
}

impl Sub {
    fn into_candidate(self) -> Option<Candidate> {
        if self.full_season {
            return None; // a pack for the whole season; episode packs come as their own entries
        }
        Some(Candidate {
            provider: NAME.into(),
            id: self.url.filter(|u| u.starts_with('/'))?,
            language: language_code(self.language.as_deref()?)?.to_owned(),
            release: self.release_name.unwrap_or_default(),
            season: self.season.filter(|s| *s > 0),
            episode: self.episode.filter(|e| *e > 0),
            hearing_impaired: self.hi,
            uploader: self.author,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape from the SubDL API documentation.
    const FOUND: &str = r#"{"status":true,
      "results":[{"sd_id":1,"type":"movie","name":"Inception","imdb_id":"tt1375666","tmdb_id":27205,"first_air_date":null,"year":2010}],
      "subtitles":[
        {"release_name":"Inception.2010.1080p.BluRay.x264-SPARKS","name":"SUBDL::inception.zip","lang":"turkish","author":"someone","url":"/subtitle/3197651-3213944.zip","subtitle_page":"/s/info/x","season":null,"episode":null,"language":"TR","hi":false,"episode_from":null,"episode_end":0,"full_season":false},
        {"release_name":"Inception.2010.720p","lang":"brazillian portuguese","url":"/subtitle/1-2.zip","language":"BR_PT","hi":true,"full_season":false},
        {"release_name":"pack","url":"/subtitle/9-9.zip","language":"TR","full_season":true}]}"#;
    // Real answer without a key (2026-10-02).
    const NO_KEY: &str = r#"{"status":false,"statusCode":403,"error":"not_authorized","message":"Not Authorized"}"#;

    #[test]
    fn reads_results() {
        let r: Response = serde_json::from_str(FOUND).unwrap();
        let c: Vec<Candidate> = r.subtitles.into_iter().filter_map(Sub::into_candidate).collect();
        assert_eq!(c.len(), 2, "season packs are skipped");
        assert_eq!(c[0].id, "/subtitle/3197651-3213944.zip");
        assert_eq!(c[0].language, "tr");
        assert_eq!(c[0].release, "Inception.2010.1080p.BluRay.x264-SPARKS");
        assert_eq!(c[1].language, "pt-br");
        assert!(c[1].hearing_impaired);
    }

    #[test]
    fn reads_errors() {
        let r: Response = serde_json::from_str(NO_KEY).unwrap();
        assert!(!r.status);
        assert_eq!(r.status_code, Some(403));
        assert_eq!(r.message.as_deref(), Some("Not Authorized"));
    }

    #[test]
    fn maps_languages() {
        assert_eq!(language_param("tr"), Some("TR"));
        assert_eq!(language_param("pt-br"), Some("BR_PT"));
        assert_eq!(language_code("TR"), Some("tr"));
        assert_eq!(language_code("BR_PT"), Some("pt-br"));
    }

    #[test]
    fn needs_a_key() {
        if BUILT_IN_KEY.is_none() {
            assert!(SubDl::new(None).is_none());
            assert!(SubDl::new(Some(" ".into())).is_none());
        }
        assert!(SubDl::new(Some("k".into())).is_some());
    }

    /// Live check against api.subdl.com with the built-in key; runs only with
    /// `SUBMAGICIAN_LIVE=1` (CI sets it with the key from the repository secrets).
    #[tokio::test]
    async fn live_subdl() {
        if !std::env::var("SUBMAGICIAN_LIVE").is_ok_and(|v| v == "1") {
            return;
        }
        let subdl = SubDl::new(None).expect("build with SUBMAGICIAN_SUBDL_API_KEY for the live test");
        let file = "Inception.2010.1080p.BluRay.x264-SPARKS.mkv";
        let q = SearchQuery {
            file_name: file.into(),
            size: 0,
            hash: None,
            name: crate::name::parse(file),
            languages: vec!["tr".into(), "en".into()],
        };
        let found = subdl.search(&q).await.unwrap();
        eprintln!("SubDL: {} results, first: {:?}", found.len(), found.first().map(|c| (&c.language, &c.release)));
        assert!(!found.is_empty());
        let d = subdl.download(&found[0]).await.unwrap();
        eprintln!("downloaded {}", d.files[0].name);
        assert!(String::from_utf8_lossy(&d.files[0].bytes).contains("-->"));
    }
}
