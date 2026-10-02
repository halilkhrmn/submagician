//! Podnapisi.NET through its XML search (`/subtitles/search/old?sXML=1`, the interface subliminal
//! uses). No key needed. Downloads are zips.
//!
//! Not verified live yet: podnapisi.net was unreachable from the development environment.

use std::time::Duration;

use reqwest::Client;

use super::{BoxFuture, Candidate, Downloaded, Provider, SearchQuery, user_agent};
use crate::{Error, Result, archive, lang};

pub const NAME: &str = "Podnapisi";
const BASE: &str = "https://www.podnapisi.net/subtitles";
/// Result pages to read per language at most (each has up to 10 subtitles).
const MAX_PAGES: u32 = 3;

pub struct Podnapisi {
    client: Client,
}

impl Default for Podnapisi {
    fn default() -> Self {
        Self::new()
    }
}

impl Podnapisi {
    pub fn new() -> Self {
        let client =
            Client::builder().user_agent(user_agent()).timeout(Duration::from_secs(30)).build().expect("HTTP client");
        Podnapisi { client }
    }

    async fn do_search(&self, q: &SearchQuery) -> Result<Vec<Candidate>> {
        let title = q.name.title.clone().unwrap_or_else(|| q.file_name.clone());
        let mut out: Vec<Candidate> = Vec::new();
        for code in &q.languages {
            let Some(language) = language_param(code) else { continue };
            let mut page = 1;
            loop {
                let mut params = vec![("sXML", "1".to_owned()), ("sL", language.to_owned()), ("sK", title.clone())];
                if q.name.is_episode {
                    params.extend(q.name.season.map(|s| ("sTS", s.to_string())));
                    params.extend(q.name.episode.map(|e| ("sTE", e.to_string())));
                } else {
                    params.extend(q.name.year.map(|y| ("sY", y.to_string())));
                }
                if page > 1 {
                    params.push(("page", page.to_string()));
                }
                let resp = self.client.get(format!("{BASE}/search/old")).query(&params).send().await?;
                let status = resp.status();
                if !status.is_success() {
                    let message = status.canonical_reason().unwrap_or("error").to_owned();
                    return Err(Error::Api { provider: NAME, status: status.as_u16(), message });
                }
                let page_result = parse(&resp.text().await?)?;
                for c in page_result.candidates {
                    if !out.iter().any(|o| o.id == c.id) {
                        out.push(c);
                    }
                }
                if page_result.current >= page_result.pages || page >= MAX_PAGES {
                    break;
                }
                page += 1;
            }
        }
        Ok(out)
    }

    async fn do_download(&self, c: &Candidate) -> Result<Downloaded> {
        let url = format!("{BASE}/{}/download", c.id);
        let resp = self.client.get(url).query(&[("container", "zip")]).send().await?.error_for_status()?;
        let bytes = resp.bytes().await?.to_vec();
        Ok(Downloaded { files: archive::extract(&format!("{}.zip", c.id), bytes)?, remaining: None })
    }
}

impl Provider for Podnapisi {
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

fn language_param(code: &str) -> Option<&'static str> {
    match code {
        "pt-pt" => Some("pt"),
        "zh-cn" => Some("zh"),
        "pt-br" | "zh-tw" => None,
        other => lang::find(other).map(|l| l.code),
    }
}

struct Page {
    candidates: Vec<Candidate>,
    current: u32,
    pages: u32,
}

fn parse(xml: &str) -> Result<Page> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Parse(format!("{NAME}: {e}")))?;
    let root = doc.root_element();
    let text = |node: roxmltree::Node, tag: &str| {
        node.children()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text())
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    };
    let pagination = root.children().find(|n| n.has_tag_name("pagination"));
    let num = |tag: &str| pagination.map(|p| text(p, tag)).and_then(|t| t.parse().ok()).unwrap_or(1);
    let mut candidates = Vec::new();
    for sub in root.children().filter(|n| n.has_tag_name("subtitle")) {
        let pid = text(sub, "pid");
        let Some(language) = lang::find(&text(sub, "language")).map(|l| l.code) else { continue };
        if pid.is_empty() {
            continue;
        }
        let release =
            text(sub, "release").split_whitespace().next().unwrap_or_default().trim_end_matches('.').to_owned();
        let flags = text(sub, "flags");
        let positive = |tag: &str| text(sub, tag).parse::<i32>().ok().filter(|v| *v > 0);
        candidates.push(Candidate {
            provider: NAME.into(),
            id: pid,
            language: language.to_owned(),
            release,
            season: positive("tvSeason"),
            episode: positive("tvEpisode"),
            downloads: text(sub, "downloads").parse().unwrap_or(0),
            rating: text(sub, "rating").parse().unwrap_or(0.0),
            hearing_impaired: flags.contains('n'),
            fps: text(sub, "fps").parse::<f32>().ok().filter(|f| *f > 0.0),
            ..Default::default()
        });
    }
    Ok(Page { candidates, current: num("current"), pages: num("count") })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built from the fields subliminal reads from this interface.
    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<results>
  <pagination><current>1</current><count>2</count><results>12</results></pagination>
  <subtitle>
    <pid>Xx1a</pid><title>Inception</title><year>2010</year>
    <tvSeason>0</tvSeason><tvEpisode>0</tvEpisode>
    <language>tr</language><flags>n</flags>
    <release>Inception.2010.1080p.BluRay.x264-SPARKS. Inception.2010.720p</release>
    <downloads>321</downloads><rating>4.5</rating><fps>23.976</fps>
    <url>https://www.podnapisi.net/subtitles/tr-inception-2010/Xx1a</url>
  </subtitle>
  <subtitle><pid>Yy2b</pid><language>en</language><release></release><tvSeason>2</tvSeason><tvEpisode>5</tvEpisode></subtitle>
  <subtitle><pid>Zz3c</pid><language>xx</language></subtitle>
</results>"#;

    #[test]
    fn parses_results() {
        let page = parse(XML).unwrap();
        assert_eq!((page.current, page.pages), (1, 2));
        assert_eq!(page.candidates.len(), 2, "unknown languages are skipped");
        let a = &page.candidates[0];
        assert_eq!(a.id, "Xx1a");
        assert_eq!(a.language, "tr");
        assert_eq!(a.release, "Inception.2010.1080p.BluRay.x264-SPARKS");
        assert!(a.hearing_impaired);
        assert_eq!((a.season, a.episode, a.downloads), (None, None, 321));
        assert_eq!(page.candidates[1].episode, Some(5));
    }

    #[test]
    fn bad_xml_is_an_error() {
        assert!(parse("<html>").is_err());
    }

    /// Live check against podnapisi.net; runs only with `SUBMAGICIAN_LIVE=1`.
    #[tokio::test]
    async fn live_podnapisi() {
        if !std::env::var("SUBMAGICIAN_LIVE").is_ok_and(|v| v == "1") {
            return;
        }
        let p = Podnapisi::new();
        let file = "Inception.2010.1080p.BluRay.x264-SPARKS.mkv";
        let q = SearchQuery {
            file_name: file.into(),
            size: 0,
            hash: None,
            name: crate::name::parse(file),
            languages: vec!["en".into()],
        };
        let found = p.search(&q).await.unwrap();
        eprintln!("Podnapisi: {} results, first: {:?}", found.len(), found.first().map(|c| &c.release));
        assert!(!found.is_empty());
        let d = p.download(&found[0]).await.unwrap();
        eprintln!("downloaded {}", d.files[0].name);
        assert!(String::from_utf8_lossy(&d.files[0].bytes).contains("-->"));
    }
}
