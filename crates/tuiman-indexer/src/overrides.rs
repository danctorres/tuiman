//! Hand-curated corrections from `overrides.toml`, applied after every
//! resolver so a human always has the last word.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use tuiman_index::Ecosystem;

use crate::model::Item;
use crate::Result;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default, rename = "entry")]
    entries: Vec<Override>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Override {
    /// GitHub `owner/name` as listed in awesome-tuis (case-insensitive).
    repo: String,
    /// Drop the entry from the index altogether.
    #[serde(default)]
    exclude: bool,
    /// Ecosystem → package name; adds to or replaces automatic matches.
    #[serde(default)]
    packages: BTreeMap<String, String>,
    /// Ecosystems whose automatic match is wrong.
    #[serde(default)]
    remove: Vec<String>,
}

impl Overrides {
    /// A missing file means no overrides; a malformed one is an error, so a
    /// typo cannot silently disable a correction.
    pub fn load(path: &Path) -> Result<Overrides> {
        match std::fs::read_to_string(path) {
            Ok(text) => Overrides::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Overrides::default()),
            Err(e) => Err(format!("{}: {e}", path.display()).into()),
        }
    }

    fn parse(text: &str) -> Result<Overrides> {
        let overrides: Overrides = toml::from_str(text)?;
        for o in &overrides.entries {
            for eco in o.packages.keys().chain(&o.remove) {
                if Ecosystem::from_name(eco).is_none() {
                    return Err(format!("overrides: {}: unknown ecosystem {eco:?}", o.repo).into());
                }
            }
            if let Some(bad) = o.packages.values().find(|p| !tuiman_index::valid_package_name(p)) {
                return Err(format!("overrides: {}: invalid package name {bad:?}", o.repo).into());
            }
        }
        Ok(overrides)
    }

    pub fn apply(&self, items: &mut Vec<Item>) {
        for o in &self.entries {
            let key = o.repo.to_ascii_lowercase();
            let Some(i) = items.iter().position(|item| item.is_repo(&key)) else {
                eprintln!("warn: overrides: {} matches no entry (renamed or removed upstream?)", o.repo);
                continue;
            };
            if o.exclude {
                items.remove(i);
                continue;
            }
            for eco in o.remove.iter().filter_map(|name| Ecosystem::from_name(name)) {
                items[i].packages.remove(eco.name());
            }
            for (eco, package) in &o.packages {
                if let Some(eco) = Ecosystem::from_name(eco) {
                    items[i].set_package(eco, package);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        let mut a = Item { name: "a".into(), repo: Some("o/a".into()), ..Item::default() };
        a.packages.insert("crates", "wrong".into());
        let b = Item { name: "b".into(), repo: Some("o/b".into()), ..Item::default() };
        vec![a, b]
    }

    #[test]
    fn adds_removes_and_excludes() {
        let overrides = Overrides::parse(
            "[[entry]]\nrepo = \"O/A\"\nremove = [\"crates\"]\npackages = { brew = \"a-tui\" }\n\n\
             [[entry]]\nrepo = \"o/b\"\nexclude = true\n\n[[entry]]\nrepo = \"o/gone\"\nexclude = true\n",
        )
        .unwrap();
        let mut items = items();
        overrides.apply(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].packages.iter().collect::<Vec<_>>(), [(&"brew", &"a-tui".to_owned())]);
    }

    #[test]
    fn typos_are_errors() {
        assert!(Overrides::parse("[[entry]]\nrepo = \"o/a\"\npackages = { homebrew = \"x\" }").is_err());
        assert!(Overrides::parse("[[entry]]\nrepo = \"o/a\"\npackages = { brew = \"--x\" }").is_err());
        assert!(Overrides::parse("[[entry]]\nrepo = \"o/a\"\nexcluded = true").is_err());
        assert!(Overrides::parse("").unwrap().entries.is_empty());
    }
}
