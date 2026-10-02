//! Search results on disk, so opening a folder again does not ask every provider again (and
//! spend their rate limits). One JSON file per provider and query; found results are kept for
//! 3 days, empty ones for 12 hours (new releases get subtitles within hours).

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::provider::{Candidate, SearchQuery};

const KEEP_FOUND: Duration = Duration::from_secs(3 * 24 * 3600);
const KEEP_EMPTY: Duration = Duration::from_secs(12 * 3600);

pub struct SearchCache {
    dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    saved: u64,
    candidates: Vec<Candidate>,
}

impl SearchCache {
    pub fn new(dir: PathBuf) -> Self {
        SearchCache { dir }
    }

    fn path(&self, provider: &str, q: &SearchQuery) -> PathBuf {
        let n = &q.name;
        let key = format!(
            "{provider}|{}|{}|{}|{:?}|{:?}|{:?}|{}",
            q.languages.join(","),
            q.hash.as_deref().unwrap_or_default(),
            n.title.as_deref().unwrap_or(&q.file_name).to_lowercase(),
            n.year,
            n.season,
            n.episode,
            q.file_name.to_lowercase(),
        );
        self.dir.join(format!("{:016x}.json", fnv1a(key.as_bytes())))
    }

    pub fn get(&self, provider: &str, q: &SearchQuery) -> Option<Vec<Candidate>> {
        let entry: Entry = serde_json::from_slice(&fs::read(self.path(provider, q)).ok()?).ok()?;
        let keep = if entry.candidates.is_empty() { KEEP_EMPTY } else { KEEP_FOUND };
        (now().saturating_sub(entry.saved) < keep.as_secs()).then_some(entry.candidates)
    }

    pub fn put(&self, provider: &str, q: &SearchQuery, candidates: &[Candidate]) {
        let entry = Entry { saved: now(), candidates: candidates.to_vec() };
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(&self.dir)?;
            fs::write(self.path(provider, q), serde_json::to_vec(&entry)?)
        };
        if let Err(e) = write() {
            log::warn!("search cache not written: {e}");
        }
    }

    /// Deletes every cached search.
    pub fn clear(&self) -> std::io::Result<()> {
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name;

    fn query(file: &str) -> SearchQuery {
        SearchQuery {
            file_name: file.into(),
            size: 1,
            hash: None,
            name: name::parse(file),
            languages: vec!["tr".into()],
        }
    }

    #[test]
    fn stores_and_expires() {
        let dir = std::env::temp_dir().join(format!("submagician-cache-{}", std::process::id()));
        let cache = SearchCache::new(dir.clone());
        let q = query("Film.2020.mkv");
        assert!(cache.get("A", &q).is_none());
        let c = Candidate { provider: "A".into(), id: "1".into(), language: "tr".into(), ..Default::default() };
        cache.put("A", &q, std::slice::from_ref(&c));
        assert_eq!(cache.get("A", &q).unwrap()[0].id, "1");
        assert!(cache.get("B", &q).is_none(), "per provider");
        assert!(cache.get("A", &query("Other.2020.mkv")).is_none(), "per query");

        // An old empty result is gone; an equally old found one is still there.
        let old = Entry { saved: now() - KEEP_EMPTY.as_secs() - 10, candidates: vec![] };
        fs::write(cache.path("A", &q), serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(cache.get("A", &q).is_none());
        let old = Entry { saved: now() - KEEP_EMPTY.as_secs() - 10, candidates: vec![c] };
        fs::write(cache.path("A", &q), serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(cache.get("A", &q).is_some());

        cache.clear().unwrap();
        assert!(cache.get("A", &q).is_none());
        cache.clear().unwrap();
    }
}
