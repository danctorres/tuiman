//! Homebrew: the whole formula and cask sets are one JSON document each,
//! matched by URL. `brew install <name>` takes either, formula first.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tuiman_index::Ecosystem;

use super::bulk::{assign, offer, Rank};
use super::Found;
use crate::http::Http;
use crate::model::{github_repo, Item};
use crate::Result;

const FORMULAE: &str = "https://formulae.brew.sh/api/formula.json";
const CASKS: &str = "https://formulae.brew.sh/api/cask.json";

pub fn resolve(http: &Http, items: &[Item]) -> Result<Vec<Found>> {
    let formulae = http.get_json(FORMULAE)?.ok_or("brew: formula.json not found")?;
    let casks = http.get_json(CASKS)?.ok_or("brew: cask.json not found")?;
    let mut by_repo = formulae_by_repo(&formulae);
    add_casks(&mut by_repo, &formulae, &casks);
    Ok(assign(items, Ecosystem::Brew, &by_repo))
}

/// Several formulae can share a repository (`foo`, `foo@2`): prefer the
/// unversioned one, then the shortest name.
fn formulae_by_repo(formulae: &Value) -> HashMap<String, (Rank, String)> {
    let mut by_repo = HashMap::new();
    for f in array(formulae) {
        let Some(name) = f["name"].as_str() else { continue };
        if f["disabled"].as_bool() == Some(true) {
            continue;
        }
        let urls = [&f["homepage"], &f["urls"]["stable"]["url"], &f["urls"]["head"]["url"]];
        for repo in urls.into_iter().filter_map(Value::as_str).filter_map(github_repo) {
            offer(&mut by_repo, repo, (u8::from(name.contains('@')), name.len()), name);
        }
    }
    by_repo
}

fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// Some CLIs ship only as casks (codex). Casks rank after formulae; GUI apps
/// (no `binary` artifact) are skipped, as are tokens a formula shadows.
fn add_casks(by_repo: &mut HashMap<String, (Rank, String)>, formulae: &Value, casks: &Value) {
    let formula_names: HashSet<&str> = array(formulae).iter().filter_map(|f| f["name"].as_str()).collect();
    for c in array(casks) {
        let Some(token) = c["token"].as_str() else { continue };
        let binary = c["artifacts"].as_array().into_iter().flatten().any(|a| a.get("binary").is_some());
        if c["disabled"].as_bool() == Some(true) || !binary || formula_names.contains(token) {
            continue;
        }
        for repo in [&c["homepage"], &c["url"]].into_iter().filter_map(Value::as_str).filter_map(github_repo)
        {
            offer(by_repo, repo, (2, token.len()), token);
        }
    }
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
        assert_eq!(map.get("aristocratos/btop").map(|(_, n)| n.as_str()), Some("btop"));
        assert_eq!(map.get("jesseduffield/lazygit").map(|(_, n)| n.as_str()), Some("lazygit"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn cli_casks_fill_in_behind_formulae() {
        let formulae =
            json!([{ "name": "btop", "homepage": "https://github.com/aristocratos/btop", "urls": {} }]);
        let bin = json!([{ "binary": ["bin/x"] }]);
        let casks = json!([
            { "token": "codex", "homepage": "https://github.com/openai/codex", "artifacts": bin },
            { "token": "btop-app", "homepage": "https://github.com/aristocratos/btop", "artifacts": bin },
            { "token": "btop", "homepage": "https://github.com/o/other", "artifacts": bin },
            { "token": "gui", "homepage": "https://github.com/o/gui", "artifacts": [{ "app": ["Gui.app"] }] }
        ]);
        let mut map = formulae_by_repo(&formulae);
        add_casks(&mut map, &formulae, &casks);
        assert_eq!(map["openai/codex"].1, "codex");
        assert_eq!(map["aristocratos/btop"].1, "btop");
        assert_eq!(map.len(), 2);
    }
}
