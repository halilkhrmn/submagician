//! The steps the app runs for one video: build the query, ask every provider, rank, fetch, save.

use std::path::PathBuf;
use std::sync::Arc;

use crate::archive::SubtitleFile;
use crate::cache::SearchCache;
use crate::media::MediaFile;
use crate::provider::{Candidate, Provider, SearchQuery};
use crate::{Error, Result, hash, name, output, score, text};

pub struct Engine {
    providers: Vec<Arc<dyn Provider>>,
    cache: Option<SearchCache>,
}

pub struct SearchOutcome {
    /// Ranked, best first (see [`score::rank`]).
    pub candidates: Vec<Candidate>,
    /// Providers that failed, with why. The others' results are still in `candidates`.
    pub errors: Vec<Error>,
}

#[derive(Debug, Clone)]
pub struct Saved {
    pub path: PathBuf,
    /// Encoding the subtitle came in, before conversion to UTF-8.
    pub source_encoding: &'static str,
    pub remaining: Option<i64>,
}

impl Engine {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Engine { providers, cache: None }
    }

    /// Keeps search results in `cache` (see [`SearchCache`]).
    pub fn with_cache(mut self, cache: SearchCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn provider_names(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// Builds the query for `media`. Reads 128 KiB of the file for the hash, so call it off the
    /// UI thread.
    pub fn query_for(media: &MediaFile, languages: &[String]) -> SearchQuery {
        SearchQuery {
            file_name: media.file_name(),
            size: media.size,
            hash: hash::file_hash(&media.path).ok(),
            name: name::parse_path(&media.path),
            languages: languages.to_vec(),
        }
    }

    /// Asks every provider. Cached results are used unless `fresh`; new results are cached.
    pub async fn search(&self, query: &SearchQuery, fresh: bool) -> SearchOutcome {
        let mut candidates = Vec::new();
        let mut errors = Vec::new();
        for provider in &self.providers {
            let cached = if fresh { None } else { self.cache.as_ref().and_then(|c| c.get(provider.name(), query)) };
            if let Some(found) = cached {
                candidates.extend(found);
                continue;
            }
            match provider.search(query).await {
                Ok(found) => {
                    if let Some(cache) = &self.cache {
                        cache.put(provider.name(), query, &found);
                    }
                    candidates.extend(found);
                }
                Err(e) => {
                    log::warn!("{} search failed: {e}", provider.name());
                    errors.push(e);
                }
            }
        }
        candidates.retain(|c| query.languages.contains(&c.language));
        score::rank(&query.file_name, &query.name, &query.languages, &mut candidates);
        SearchOutcome { candidates, errors }
    }

    /// Downloads `candidate` and saves it next to the video as `<stem>.<lang>.<ext>`, UTF-8.
    pub async fn fetch(&self, media: &MediaFile, query: &SearchQuery, candidate: &Candidate) -> Result<Saved> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.name() == candidate.provider)
            .ok_or_else(|| Error::Parse(format!("unknown provider {}", candidate.provider)))?;
        let downloaded = provider.download(candidate).await?;
        let file = pick_file(&query.file_name, &query.name, downloaded.files).ok_or(Error::NoSubtitleInArchive)?;
        let decoded = text::decode(&file.bytes, Some(&candidate.language));
        let format = text::detect_format(&decoded.text);
        let path = output::write_subtitle(&media.path, &candidate.language, format.extension(), &decoded.text)?;
        Ok(Saved { path, source_encoding: decoded.encoding, remaining: downloaded.remaining })
    }
}

/// Season packs hold many files; take the one for this episode, else the most similar name.
fn pick_file(video_name: &str, video: &name::ParsedName, files: Vec<SubtitleFile>) -> Option<SubtitleFile> {
    if files.len() <= 1 {
        return files.into_iter().next();
    }
    files
        .into_iter()
        .map(|f| {
            let parsed = name::parse(&f.name);
            let mut s = (score::similarity(video_name, &f.name) * 100.0) as i32;
            if video.is_episode && parsed.episode.is_some() {
                s += if parsed.episode == video.episode && parsed.season.or(video.season) == video.season {
                    1000
                } else {
                    -1000
                };
            }
            (s, f)
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, f)| f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{BoxFuture, Downloaded};
    use std::fs;

    struct Fake;

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<Candidate>>> {
            let release = q.file_name.clone();
            Box::pin(async move {
                Ok(vec![
                    Candidate {
                        provider: "Fake".into(),
                        id: "1".into(),
                        language: "tr".into(),
                        release,
                        ..Default::default()
                    },
                    Candidate { provider: "Fake".into(), id: "2".into(), language: "fr".into(), ..Default::default() },
                ])
            })
        }
        fn download<'a>(&'a self, _: &'a Candidate) -> BoxFuture<'a, Result<Downloaded>> {
            Box::pin(async {
                let (cp1254, _, _) =
                    encoding_rs::WINDOWS_1254.encode("1\r\n00:00:01,000 --> 00:00:02,000\r\nŞişli'de ığdır\r\n");
                let files = vec![
                    SubtitleFile { name: "Show.S01E01.srt".into(), bytes: b"wrong".to_vec() },
                    SubtitleFile { name: "Show.S01E02.srt".into(), bytes: cp1254.into_owned() },
                ];
                Ok(Downloaded { files, remaining: Some(9) })
            })
        }
    }

    struct Broken;

    impl Provider for Broken {
        fn name(&self) -> &'static str {
            "Broken"
        }
        fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<Candidate>>> {
            Box::pin(async { Err(Error::Parse("down".into())) })
        }
        fn download<'a>(&'a self, _: &'a Candidate) -> BoxFuture<'a, Result<Downloaded>> {
            Box::pin(async { Err(Error::Parse("down".into())) })
        }
    }

    #[tokio::test]
    async fn searches_fetches_and_saves_utf8() {
        let dir = std::env::temp_dir().join(format!("submagician-engine-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Show.S01E02.720p.mkv");
        fs::write(&video, b"tiny").unwrap();
        let media = crate::media::scan(&dir, false).remove(0);

        let engine = Engine::new(vec![Arc::new(Broken), Arc::new(Fake)]);
        let langs = vec!["tr".to_string()];
        let query = Engine::query_for(&media, &langs);
        assert!(query.hash.is_none(), "tiny files have no hash");

        let outcome = engine.search(&query, false).await;
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(outcome.candidates.len(), 1, "unwanted languages are dropped");

        let saved = engine.fetch(&media, &query, &outcome.candidates[0]).await.unwrap();
        assert_eq!(saved.path, dir.join("Show.S01E02.720p.tr.srt"));
        assert_eq!(saved.source_encoding, "windows-1254");
        let written = fs::read_to_string(&saved.path).unwrap();
        assert!(written.contains("Şişli'de ığdır"), "{written}");
        fs::remove_dir_all(&dir).unwrap();
    }

    struct Counting(std::sync::atomic::AtomicUsize);

    impl Provider for Counting {
        fn name(&self) -> &'static str {
            "Counting"
        }
        fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<Candidate>>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Ok(vec![Candidate {
                    provider: "Counting".into(),
                    id: "1".into(),
                    language: "tr".into(),
                    ..Default::default()
                }])
            })
        }
        fn download<'a>(&'a self, _: &'a Candidate) -> BoxFuture<'a, Result<Downloaded>> {
            Box::pin(async { Err(Error::Parse("no".into())) })
        }
    }

    #[tokio::test]
    async fn uses_the_cache_unless_fresh() {
        let dir = std::env::temp_dir().join(format!("submagician-engine-cache-{}", std::process::id()));
        let counting = Arc::new(Counting(Default::default()));
        let engine = Engine::new(vec![counting.clone()]).with_cache(SearchCache::new(dir.clone()));
        let file = "Film.2020.mkv";
        let q = SearchQuery {
            file_name: file.into(),
            size: 1,
            hash: None,
            name: crate::name::parse(file),
            languages: vec!["tr".into()],
        };
        assert_eq!(engine.search(&q, false).await.candidates.len(), 1);
        assert_eq!(engine.search(&q, false).await.candidates.len(), 1);
        assert_eq!(counting.0.load(std::sync::atomic::Ordering::SeqCst), 1, "second search came from the cache");
        engine.search(&q, true).await;
        assert_eq!(counting.0.load(std::sync::atomic::Ordering::SeqCst), 2, "fresh asks again");
        let _ = fs::remove_dir_all(&dir);
    }
}
