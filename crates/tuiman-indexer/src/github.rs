//! GitHub enrichment: one aliased GraphQL query per batch of repositories
//! returns stars, language, license, activity and the root build manifests.

use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::http::Http;
use crate::model::Item;
use crate::Result;

const ENDPOINT: &str = "https://api.github.com/graphql";
/// Each repository costs six object lookups, so batches stay modest.
const BATCH: usize = 25;

const FRAGMENT: &str = r#"
fragment F on Repository {
  nameWithOwner stargazerCount isArchived pushedAt
  primaryLanguage { name }
  licenseInfo { spdxId }
  cargo: object(expression: "HEAD:Cargo.toml") { ... on Blob { text } }
  npm: object(expression: "HEAD:package.json") { ... on Blob { text } }
  py: object(expression: "HEAD:pyproject.toml") { ... on Blob { text } }
  gomod: object(expression: "HEAD:go.mod") { ... on Blob { text } }
  maingo: object(expression: "HEAD:main.go") { id }
  src: object(expression: "HEAD:src") { ... on Tree { entries { name } } }
}"#;

/// Fills in GitHub metadata and returns the indices of items whose repository
/// no longer exists.
pub fn enrich(http: &Http, token: &str, items: &mut [Item]) -> Result<Vec<usize>> {
    let targets: Vec<usize> = (0..items.len()).filter(|&i| items[i].repo.is_some()).collect();
    let mut dead = Vec::new();
    for (n, batch) in targets.chunks(BATCH).enumerate() {
        eprintln!("github: batch {}/{}", n + 1, targets.len().div_ceil(BATCH));
        fetch(http, token, items, batch, &mut dead)?;
    }
    Ok(dead)
}

/// Queries one batch. A gateway timeout is retried with backoff, then the
/// batch is split in half, since a smaller query is what gets under GitHub's limit.
fn fetch(http: &Http, token: &str, items: &mut [Item], batch: &[usize], dead: &mut Vec<usize>) -> Result<()> {
    let repos: Vec<&str> = batch.iter().map(|&i| items[i].repo.as_deref().unwrap()).collect();
    let body = json!({ "query": query(&repos) });
    let mut response = http.post_json(ENDPOINT, token, &body)?;
    for secs in [2, 4, 8] {
        if response.is_some() {
            break;
        }
        thread::sleep(Duration::from_secs(secs));
        response = http.post_json(ENDPOINT, token, &body)?;
    }
    let response = match response {
        Some(response) => response,
        None if batch.len() > 1 => {
            let (a, b) = batch.split_at(batch.len() / 2);
            fetch(http, token, items, a, dead)?;
            return fetch(http, token, items, b, dead);
        }
        None => {
            let name = &items[batch[0]].name;
            eprintln!("warn: github: {name}: kept without metadata: gateway timeout");
            return Ok(());
        }
    };
    let Some(data) = response.get("data").filter(|d| d.is_object()) else {
        return Err(format!("github: no data in response: {}", response["errors"]).into());
    };
    let errors = response["errors"].as_array().map(Vec::as_slice).unwrap_or_default();
    for (alias, &i) in batch.iter().enumerate() {
        let alias = format!("r{alias}");
        match data.get(&alias) {
            Some(repo) if repo.is_object() => apply(&mut items[i], repo),
            _ if not_found(errors, &alias) => dead.push(i),
            // Any other per-repository error is transient. Dropping the
            // entry would publish a smaller catalog; keeping it without
            // metadata is what a run with no token does anyway.
            _ => eprintln!("warn: github: {}: kept without metadata: {}", items[i].name, why(errors, &alias)),
        }
    }
    Ok(())
}

/// GraphQL reports a missing repository as a null alias plus an error at that path.
fn not_found(errors: &[Value], alias: &str) -> bool {
    errors.iter().any(|e| e["type"] == "NOT_FOUND" && e["path"][0] == alias)
}

fn why(errors: &[Value], alias: &str) -> String {
    let messages: Vec<&str> =
        errors.iter().filter(|e| e["path"][0] == alias).filter_map(|e| e["message"].as_str()).collect();
    if messages.is_empty() {
        "no error reported".to_owned()
    } else {
        messages.join("; ")
    }
}

/// `repos` are `owner/name` keys already restricted to `[a-z0-9._-]` by
/// `model::github_repo`, so they can be spliced into the query text.
fn query(repos: &[&str]) -> String {
    let mut q = String::from("query {\n");
    for (alias, repo) in repos.iter().enumerate() {
        let (owner, name) = repo.split_once('/').expect("repo key is owner/name");
        q.push_str(&format!("  r{alias}: repository(owner: \"{owner}\", name: \"{name}\") {{ ...F }}\n"));
    }
    q.push('}');
    q.push_str(FRAGMENT);
    q
}

fn apply(item: &mut Item, repo: &Value) {
    let text = |key: &str| repo[key]["text"].as_str().map(str::to_owned);

    if let Some(current) = repo["nameWithOwner"].as_str().map(str::to_ascii_lowercase) {
        if item.repo.as_deref() != Some(current.as_str()) {
            item.former_repo = item.repo.replace(current);
        }
    }
    item.stars = repo["stargazerCount"].as_u64().map(|s| s.min(u64::from(u32::MAX - 1)) as u32);
    item.archived = repo["isArchived"].as_bool().unwrap_or(false);
    item.pushed_at = repo["pushedAt"].as_str().map(str::to_owned);
    item.language = repo["primaryLanguage"]["name"].as_str().unwrap_or_default().to_owned();
    item.license = match repo["licenseInfo"]["spdxId"].as_str() {
        Some("NOASSERTION") | None => String::new(),
        Some(id) => id.to_owned(),
    };
    item.manifests.cargo_toml = text("cargo");
    item.manifests.package_json = text("npm");
    item.manifests.pyproject_toml = text("py");
    item.manifests.go_mod = text("gomod");
    item.manifests.has_root_main_go = repo["maingo"].is_object();
    // Cargo's automatic binary targets: `src/main.rs` and `src/bin/`.
    let src = repo["src"]["entries"].as_array().into_iter().flatten();
    item.manifests.has_rust_bin =
        src.filter_map(|e| e["name"].as_str()).any(|n| n == "main.rs" || n == "bin");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_aliases_every_repo() {
        let q = query(&["a/b", "c/d.e"]);
        assert!(q.contains(r#"r0: repository(owner: "a", name: "b") { ...F }"#));
        assert!(q.contains(r#"r1: repository(owner: "c", name: "d.e") { ...F }"#));
        assert!(q.ends_with(FRAGMENT));
    }

    #[test]
    fn applies_a_repository_node() {
        let node = json!({
            "nameWithOwner": "New-Owner/bottom",
            "stargazerCount": 12345,
            "isArchived": true,
            "pushedAt": "2026-09-01T00:00:00Z",
            "primaryLanguage": { "name": "Rust" },
            "licenseInfo": { "spdxId": "MIT" },
            "cargo": { "text": "[package]\nname = \"bottom\"" },
            "npm": null, "py": null, "gomod": null, "maingo": null,
            "src": { "entries": [{ "name": "lib.rs" }, { "name": "bin" }] }
        });
        let mut item = Item { repo: Some("old-owner/bottom".into()), ..Item::default() };
        apply(&mut item, &node);

        assert_eq!(item.repo.as_deref(), Some("new-owner/bottom"));
        assert_eq!(item.former_repo.as_deref(), Some("old-owner/bottom"));
        assert!(item.is_repo("old-owner/bottom") && item.is_repo("new-owner/bottom"));
        assert_eq!(item.stars, Some(12345));
        assert!(item.archived);
        assert_eq!((item.language.as_str(), item.license.as_str()), ("Rust", "MIT"));
        assert!(item.manifests.cargo_toml.is_some() && item.manifests.go_mod.is_none());
        assert!(!item.manifests.has_root_main_go && item.manifests.has_rust_bin);
    }

    #[test]
    fn only_not_found_errors_mean_a_dead_repository() {
        let errors = [
            json!({ "type": "NOT_FOUND", "path": ["r1"], "message": "Could not resolve" }),
            json!({ "path": ["r2"], "message": "timeout" }),
        ];
        assert!(not_found(&errors, "r1"));
        assert!(!not_found(&errors, "r2") && !not_found(&errors, "r3"));
        assert_eq!(why(&errors, "r2"), "timeout");
        assert_eq!(why(&errors, "r3"), "no error reported");
    }

    #[test]
    fn missing_metadata_stays_empty() {
        let node = json!({ "nameWithOwner": "a/b", "stargazerCount": 1, "licenseInfo": { "spdxId": "NOASSERTION" } });
        let mut item = Item { repo: Some("a/b".into()), ..Item::default() };
        apply(&mut item, &node);
        assert_eq!(item.former_repo, None);
        assert_eq!((item.language.as_str(), item.license.as_str()), ("", ""));
    }
}
