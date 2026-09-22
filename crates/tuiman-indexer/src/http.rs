//! Thin blocking HTTP helpers over one shared `ureq` agent.

use std::time::Duration;

use serde_json::Value;
use ureq::http::Response;
use ureq::Body;

use crate::Result;

const USER_AGENT: &str = "tuiman-indexer (+https://github.com/danctorres/tuiman)";
/// Bulk package datasets (brew, AUR, nixpkgs) are tens of megabytes.
const MAX_BODY: u64 = 512 << 20;

pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Http {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build();
        Http { agent: config.into() }
    }

    pub fn get_text(&self, url: &str) -> Result<String> {
        let mut resp = ok("GET", url, self.agent.get(url).call()?)?;
        Ok(resp.body_mut().with_config().limit(MAX_BODY).read_to_string()?)
    }

    /// Streams a large body, for datasets that are decompressed on the fly.
    pub fn get_reader(&self, url: &str) -> Result<impl std::io::Read> {
        let resp = ok("GET", url, self.agent.get(url).call()?)?;
        Ok(resp.into_body().into_with_config().limit(MAX_BODY).reader())
    }

    /// `Ok(None)` on 404, which registries use for "no such package".
    pub fn get_json(&self, url: &str) -> Result<Option<Value>> {
        let resp = self.agent.get(url).header("Accept", "application/json").call()?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        let mut resp = ok("GET", url, resp)?;
        Ok(Some(resp.body_mut().with_config().limit(MAX_BODY).read_json()?))
    }

    /// `Ok(None)` on 502/503/504, which GitHub returns when a query runs too long.
    pub fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<Option<Value>> {
        let resp =
            self.agent.post(url).header("Authorization", &format!("Bearer {bearer}")).send_json(body)?;
        if matches!(resp.status().as_u16(), 502..=504) {
            eprintln!("warn: POST {url}: {}", resp.status());
            return Ok(None);
        }
        let mut resp = ok("POST", url, resp)?;
        Ok(Some(resp.body_mut().with_config().limit(MAX_BODY).read_json()?))
    }
}

fn ok(method: &str, url: &str, resp: Response<Body>) -> Result<Response<Body>> {
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(format!("{method} {url}: {}", resp.status()).into())
    }
}
