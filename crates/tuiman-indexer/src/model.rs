//! The indexer's working representation of one catalog entry.

use std::collections::BTreeMap;

use serde::Serialize;
use tuiman_index::Ecosystem;

use crate::readme::Listing;

#[derive(Debug, Default, Serialize)]
pub struct Item {
    pub name: String,
    pub url: String,
    pub description: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subcategory: Option<String>,
    /// Lower-cased `owner/name` for GitHub-hosted projects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub stars: Option<u32>,
    pub language: String,
    pub license: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    pub archived: bool,
    pub library: bool,
    /// Ecosystem name → package name.
    pub packages: BTreeMap<&'static str, String>,

    /// The key as listed, when GitHub reports the repository under a new name.
    /// Package metadata often still points at the old one.
    #[serde(skip)]
    pub former_repo: Option<String>,
    #[serde(skip)]
    pub manifests: Manifests,
}

/// Build manifests found at the repository root, used to derive registry names.
#[derive(Debug, Default)]
pub struct Manifests {
    pub cargo_toml: Option<String>,
    pub package_json: Option<String>,
    pub pyproject_toml: Option<String>,
    pub go_mod: Option<String>,
    pub has_root_main_go: bool,
}

impl Item {
    pub fn from_listing(l: Listing) -> Item {
        Item {
            repo: github_repo(&l.url),
            library: l.category == "Libraries",
            name: l.name,
            url: l.url,
            description: l.desc,
            category: l.category,
            subcategory: l.subcategory,
            ..Item::default()
        }
    }

    pub fn is_repo(&self, key: &str) -> bool {
        self.repo.as_deref() == Some(key) || self.former_repo.as_deref() == Some(key)
    }

    pub fn set_package(&mut self, eco: Ecosystem, package: &str) {
        if tuiman_index::valid_package_name(package) {
            self.packages.insert(eco.name(), package.to_owned());
        } else {
            eprintln!("warn: {}: dropping unsafe {} package name {package:?}", self.name, eco.name());
        }
    }
}

/// Extracts a lower-cased `owner/name` from any URL that points into a GitHub
/// repository. Used both for entries and for matching package metadata.
pub fn github_repo(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let rest = lower.split_once("github.com/")?.1;
    let mut parts = rest.split(['/', '#', '?']);
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let name = parts.next().filter(|s| !s.is_empty())?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    let ok = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    (ok(owner) && ok(name) && !name.is_empty()).then(|| format!("{owner}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_keys() {
        let key = |u| github_repo(u);
        assert_eq!(key("https://github.com/ClementTsang/bottom"), Some("clementtsang/bottom".into()));
        assert_eq!(key("https://github.com/a/b.git"), Some("a/b".into()));
        assert_eq!(key("git+https://github.com/a/b.git#readme"), Some("a/b".into()));
        assert_eq!(key("https://github.com/a/b/archive/refs/tags/v1.tar.gz"), Some("a/b".into()));
        assert_eq!(key("https://github.com/a"), None);
        assert_eq!(key("https://codeberg.org/a/b"), None);
        assert_eq!(key("https://github.com/a/b\"){evil}"), None);
    }
}
