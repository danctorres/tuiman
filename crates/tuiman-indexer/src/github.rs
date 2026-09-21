//! GitHub enrichment: one aliased GraphQL query per batch of repositories
//! returns stars, language, license, activity and the root build manifests.

use serde_json::{json, Value};

use crate::http::Http;
use crate::model::Item;
use crate::Result;

const ENDPOINT: &str = "https://api.github.com/graphql";
/// Each repository costs five blob lookups, so batches stay modest.
const BATCH: usize = 40;

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
}"#;

/// Fills in GitHub metadata and returns the indices of items whose repository
/// no longer exists.
pub fn enrich(http: &Http, token: &str, items: &mut [Item]) -> Result<Vec<usize>> {
    let targets: Vec<usize> = (0..items.len()).filter(|&i| items[i].repo.is_some()).collect();
    let mut dead = Vec::new();
    for (n, batch) in targets.chunks(BATCH).enumerate() {
        eprintln!("github: batch {}/{}", n + 1, targets.len().div_ceil(BATCH));
        let repos: Vec<&str> = batch.iter().map(|&i| items[i].repo.as_deref().unwrap()).collect();
        let response = http.post_json(ENDPOINT, token, &json!({ "query": query(&repos) }))?;
        let Some(data) = response.get("data").filter(|d| d.is_object()) else {
            return Err(format!("github: no data in response: {}", response["errors"]).into());
        };
        for (alias, &i) in batch.iter().enumerate() {
            match data.get(format!("r{alias}")) {
                Some(repo) if repo.is_object() => apply(&mut items[i], repo),
                _ => dead.push(i),
            }
        }
    }
    Ok(dead)
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
            "npm": null, "py": null, "gomod": null, "maingo": null
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
        assert!(!item.manifests.has_root_main_go);
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
