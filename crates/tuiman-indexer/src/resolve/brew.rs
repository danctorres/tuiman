//! Homebrew: the whole formula set is one JSON document, matched by URL.

use std::collections::HashMap;

use serde_json::Value;
use tuiman_index::Ecosystem;

use super::Found;
use crate::http::Http;
use crate::model::{github_repo, Item};
use crate::Result;

const FORMULAE: &str = "https://formulae.brew.sh/api/formula.json";

pub fn resolve(http: &Http, items: &[Item]) -> Result<Vec<Found>> {
    let formulae = http.get_json(FORMULAE)?.ok_or("brew: formula.json not found")?;
    let by_repo = formulae_by_repo(&formulae);
    Ok(items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.library)
        .filter_map(|(i, item)| {
            let name = [&item.repo, &item.former_repo].into_iter().flatten().find_map(|k| by_repo.get(k))?;
            Some((i, Ecosystem::Brew, name.clone()))
        })
        .collect())
}

fn formulae_by_repo(formulae: &Value) -> HashMap<String, String> {
    let mut by_repo: HashMap<String, String> = HashMap::new();
    for f in formulae.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(name) = f["name"].as_str() else { continue };
        if f["disabled"].as_bool() == Some(true) {
            continue;
        }
        let urls = [&f["homepage"], &f["urls"]["stable"]["url"], &f["urls"]["head"]["url"]];
        for repo in urls.into_iter().filter_map(Value::as_str).filter_map(github_repo) {
            let best = by_repo.entry(repo).or_insert_with(|| name.to_owned());
            if rank(name) < rank(best) {
                name.clone_into(best);
            }
        }
    }
    by_repo
}

/// Several formulae can share a repository (`foo`, `foo@2`): prefer the
/// unversioned one, then the shortest name.
fn rank(name: &str) -> (bool, usize) {
    (name.contains('@'), name.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_by_any_url_and_prefers_unversioned() {
        let formulae = json!([
            { "name": "btop@1", "homepage": "https://github.com/aristocratos/btop", "urls": {} },
            { "name": "btop", "homepage": "https://github.com/aristocratos/btop", "urls": {} },
            { "name": "lazygit", "homepage": "https://example.com",
              "urls": { "stable": { "url": "https://github.com/jesseduffield/lazygit/archive/v1.tar.gz" } } },
            { "name": "gone", "homepage": "https://github.com/x/gone", "disabled": true, "urls": {} },
            { "name": "elsewhere", "homepage": "https://gitlab.com/x/y", "urls": {} }
        ]);
        let map = formulae_by_repo(&formulae);
        assert_eq!(map.get("aristocratos/btop").map(String::as_str), Some("btop"));
        assert_eq!(map.get("jesseduffield/lazygit").map(String::as_str), Some("lazygit"));
        assert_eq!(map.len(), 2);
    }
}
