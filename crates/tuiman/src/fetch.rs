//! Index download and cache. One conditional GET; the body is validated by
//! `tuiman_index::decode` before it replaces the cached copy, and the
//! replacement is atomic, so a bad download can never brick startup.

use std::path::Path;
use std::time::{Duration, SystemTime};
use std::{env, fs, io};

use tuiman_index::{Catalog, MAX_INDEX_BYTES};

use crate::app::IndexUpdate;

const DEFAULT_URL: &str = "https://raw.githubusercontent.com/danctorres/tuiman/index/index.bin";
const INDEX_FILE: &str = "index.bin";
const ETAG_FILE: &str = "index.etag";
/// The index is rebuilt daily; checking more often than this buys nothing.
const FRESH_FOR: Duration = Duration::from_secs(6 * 60 * 60);

/// The cached catalog, if there is a valid one.
pub fn load_cached(cache_dir: &Path) -> Option<Catalog> {
    tuiman_index::decode(&fs::read(cache_dir.join(INDEX_FILE)).ok()?).ok()
}

/// Whether the last successful check is recent enough to skip the network.
pub fn is_fresh(cache_dir: &Path) -> bool {
    let checked = fs::metadata(cache_dir.join(ETAG_FILE)).and_then(|m| m.modified());
    checked.is_ok_and(|at| SystemTime::now().duration_since(at).is_ok_and(|age| age < FRESH_FOR))
}

pub fn refresh(cache_dir: &Path) -> IndexUpdate {
    match try_refresh(cache_dir) {
        Ok(update) => update,
        Err(why) => IndexUpdate::Failed(why),
    }
}

fn try_refresh(cache_dir: &Path) -> Result<IndexUpdate, String> {
    let source = env::var("TUIMAN_INDEX_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
    let have_index = cache_dir.join(INDEX_FILE).exists();
    let etag = fs::read_to_string(cache_dir.join(ETAG_FILE)).ok().filter(|e| have_index && !e.is_empty());

    let (bytes, new_etag) = if source.starts_with("https://") {
        match download(&source, etag.as_deref())? {
            Some(fresh) => fresh,
            None => {
                // Record the check so the next start within FRESH_FOR stays offline.
                let _ = fs::write(cache_dir.join(ETAG_FILE), etag.unwrap_or_default());
                return Ok(IndexUpdate::NotModified);
            }
        }
    } else {
        // A local path: for developing the indexer and for air-gapped setups.
        (fs::read(&source).map_err(|e| format!("{source}: {e}"))?, String::new())
    };

    let catalog = tuiman_index::decode(&bytes).map_err(|e| e.to_string())?;
    store(cache_dir, &bytes, &new_etag).map_err(|e| format!("cannot write cache: {e}"))?;
    Ok(IndexUpdate::Fresh(Box::new(catalog)))
}

/// `Ok(None)` means 304 Not Modified.
fn download(url: &str, etag: Option<&str>) -> Result<Option<(Vec<u8>, String)>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .https_only(true)
        .http_status_as_error(false)
        .user_agent(concat!("tuiman/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut request = agent.get(url);
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let mut response = request.call().map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        304 => Ok(None),
        200 => {
            let etag =
                response.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
            let body = response.body_mut().with_config().limit(MAX_INDEX_BYTES as u64).read_to_vec();
            Ok(Some((body.map_err(|e| e.to_string())?, etag)))
        }
        404 => Err("the index has not been published yet (404)".into()),
        status => Err(format!("server answered {status}")),
    }
}

fn store(cache_dir: &Path, bytes: &[u8], etag: &str) -> io::Result<()> {
    fs::create_dir_all(cache_dir)?;
    let tmp = cache_dir.join(format!("{INDEX_FILE}.{}.tmp", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, cache_dir.join(INDEX_FILE))?;
    fs::write(cache_dir.join(ETAG_FILE), etag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuiman_index::{Builder, Entry};

    #[test]
    fn store_then_load_and_reject_corruption() {
        let dir = env::temp_dir().join(format!("tuiman-fetch-test-{}", std::process::id()));
        assert!(load_cached(&dir).is_none() && !is_fresh(&dir));

        let mut b = Builder::new(7);
        b.push(&Entry { name: "x", url: "https://x.y", ..Entry::default() }).unwrap();
        store(&dir, &tuiman_index::encode(&b.finish()), "\"abc\"").unwrap();
        assert_eq!(load_cached(&dir).map(|c| c.len()), Some(1));
        assert!(is_fresh(&dir));

        fs::write(dir.join(INDEX_FILE), b"garbage").unwrap();
        assert!(load_cached(&dir).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
}
