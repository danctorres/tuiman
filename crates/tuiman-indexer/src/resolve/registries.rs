//! Language registries (crates.io, npm, PyPI, Go modules).
//!
//! The candidate name comes from the manifest at the repository root; it is
//! accepted only when the registry's record for that name points back at the
//! same repository. That rules out name squatting and unpublished projects.

use std::thread;
use std::time::Duration;

use serde_json::Value;
use tuiman_index::Ecosystem;

use super::Found;
use crate::http::Http;
use crate::model::{github_repo, Item};

pub fn resolve(http: &Http, items: &[Item]) -> Vec<Found> {
    let all = || items.iter().enumerate();

    let mut found: Vec<Found> =
        all().filter_map(|(i, item)| Some((i, Ecosystem::Go, go_package(item)?))).collect();

    // One thread per registry: they are independent hosts, and crates.io asks
    // crawlers to stay at one request per second.
    let verified = thread::scope(|s| {
        let crates = s.spawn(|| {
            verify(
                all(),
                Ecosystem::Crates,
                Duration::from_secs(1),
                |i| cargo_name(i).into_iter().collect(),
                |name| {
                    let doc = http.get_json(&format!("https://crates.io/api/v1/crates/{name}"))?;
                    Ok(doc.map(|d| vec![d["crate"]["repository"].clone(), d["crate"]["homepage"].clone()]))
                },
            )
        });
        let npm = s.spawn(|| {
            verify(all(), Ecosystem::Npm, Duration::from_millis(100), npm_names, |name| {
                // `/latest` is one version's manifest, not every release ever.
                let doc = http
                    .get_json(&format!("https://registry.npmjs.org/{}/latest", name.replace('/', "%2F")))?;
                Ok(doc.filter(npm_runnable).map(|d| {
                    vec![d["repository"]["url"].clone(), d["repository"].clone(), d["homepage"].clone()]
                }))
            })
        });
        let pypi = s.spawn(|| {
            verify(
                all(),
                Ecosystem::Pypi,
                Duration::from_millis(100),
                |i| pypi_name(i).into_iter().collect(),
                |name| {
                    let doc = http.get_json(&format!("https://pypi.org/pypi/{name}/json"))?;
                    Ok(doc.map(|d| {
                        let mut urls = vec![d["info"]["home_page"].clone()];
                        urls.extend(
                            d["info"]["project_urls"]
                                .as_object()
                                .into_iter()
                                .flat_map(|o| o.values().cloned()),
                        );
                        urls
                    }))
                },
            )
        });
        [crates, npm, pypi].map(|h| h.join().expect("resolver thread panicked"))
    });
    found.extend(verified.into_iter().flatten());
    found
}

/// Looks up each candidate and keeps those whose registry record links back
/// to the entry's repository. A failed lookup loses one package, not the run.
fn verify<'a>(
    items: impl Iterator<Item = (usize, &'a Item)>,
    eco: Ecosystem,
    pause: Duration,
    candidates: fn(&Item) -> Vec<String>,
    registry_urls: impl Fn(&str) -> crate::Result<Option<Vec<Value>>>,
) -> Vec<Found> {
    let mut found = Vec::new();
    for (i, item) in items {
        for name in candidates(item).into_iter().filter(|n| tuiman_index::valid_package_name(n)) {
            let hit = match registry_urls(&name) {
                Ok(Some(urls)) => links_back(item, &urls),
                Ok(None) => false,
                Err(e) => {
                    eprintln!("warn: {}: {name}: {e}", eco.name());
                    false
                }
            };
            thread::sleep(pause);
            if hit {
                found.push((i, eco, name));
                break;
            }
        }
    }
    found
}

fn links_back(item: &Item, urls: &[Value]) -> bool {
    urls.iter().filter_map(Value::as_str).filter_map(github_repo).any(|repo| item.is_repo(&repo))
}

/// `cargo install` needs a binary target: `src/main.rs`, `src/bin/` or a `[[bin]]`.
fn cargo_name(item: &Item) -> Option<String> {
    let manifest: toml::Table = item.manifests.cargo_toml.as_deref()?.parse().ok()?;
    if !item.manifests.has_rust_bin && !manifest.contains_key("bin") {
        return None;
    }
    let package = manifest.get("package")?;
    if package.get("publish").and_then(toml::Value::as_bool) == Some(false) {
        return None;
    }
    package.get("name")?.as_str().map(str::to_owned)
}

/// A private root manifest is usually a monorepo whose CLI is published from
/// a subdirectory (openai/codex ships `@openai/codex`), so guess the usual
/// names; the link-back check still decides.
fn npm_names(item: &Item) -> Vec<String> {
    let Some(manifest) = item.manifests.package_json.as_deref() else { return Vec::new() };
    let manifest: Value = serde_json::from_str(manifest).unwrap_or_default();
    if manifest["private"].as_bool() != Some(true) {
        return manifest["name"].as_str().map(str::to_owned).into_iter().collect();
    }
    let Some((owner, name)) = item.repo.as_deref().and_then(|r| r.split_once('/')) else { return Vec::new() };
    vec![format!("@{owner}/{name}"), name.to_owned()]
}

/// The `bin` that `npm install -g` honours is the published one, which build
/// steps often add (carbonyl's repository `package.json` has none).
fn npm_runnable(latest: &Value) -> bool {
    latest["bin"].is_object() || latest["bin"].is_string()
}

fn pypi_name(item: &Item) -> Option<String> {
    let manifest: toml::Table = item.manifests.pyproject_toml.as_deref()?.parse().ok()?;
    let project = manifest.get("project").filter(|p| p.get("scripts").is_some());
    let poetry = manifest.get("tool").and_then(|t| t.get("poetry")).filter(|p| p.get("scripts").is_some());
    project.or(poetry)?.get("name")?.as_str().map(str::to_owned)
}

/// `go install` needs a main package; only the unambiguous layout (main.go at
/// the module root, module path inside the listed repository) is accepted.
fn go_package(item: &Item) -> Option<String> {
    if !item.manifests.has_root_main_go {
        return None;
    }
    let go_mod = item.manifests.go_mod.as_deref()?;
    let module = go_mod.lines().find_map(|l| l.trim().strip_prefix("module "))?.trim().trim_matches('"');
    let in_repo = github_repo(&format!("https://{module}")).is_some_and(|repo| item.is_repo(&repo));
    (in_repo && tuiman_index::valid_package_name(module)).then(|| module.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Manifests;
    use serde_json::json;

    fn item(repo: &str, manifests: Manifests) -> Item {
        Item { repo: Some(repo.into()), manifests, ..Item::default() }
    }

    #[test]
    fn cargo_candidates() {
        let pkg = |toml: &str| {
            cargo_name(&item(
                "a/b",
                Manifests { cargo_toml: Some(toml.into()), has_rust_bin: true, ..Manifests::default() },
            ))
        };
        assert_eq!(pkg("[package]\nname = \"bottom\"\nversion = \"1.0.0\""), Some("bottom".into()));
        assert_eq!(pkg("[package]\nname = \"x\"\npublish = false"), None);
        assert_eq!(pkg("[workspace]\nmembers = [\"a\"]"), None);
        assert_eq!(pkg("not toml ["), None);

        let lib = |toml: &str| {
            cargo_name(&item("a/b", Manifests { cargo_toml: Some(toml.into()), ..Manifests::default() }))
        };
        assert_eq!(lib("[package]\nname = \"ratatui\""), None);
        assert_eq!(lib("[package]\nname = \"x\"\n[[bin]]\nname = \"x\""), Some("x".into()));
    }

    #[test]
    fn npm_candidates_are_public() {
        let pkg = |json: &str| {
            npm_names(&item(
                "openai/codex",
                Manifests { package_json: Some(json.into()), ..Manifests::default() },
            ))
        };
        assert_eq!(pkg(r#"{"name":"@s/cli","bin":{"cli":"x.js"}}"#), ["@s/cli"]);
        assert_eq!(pkg(r#"{"name":"carbonyl"}"#), ["carbonyl"]);
        assert_eq!(pkg(r#"{"name":"codex-monorepo","private":true}"#), ["@openai/codex", "codex"]);
        assert!(npm_names(&item("a/b", Manifests::default())).is_empty());
    }

    #[test]
    fn pypi_candidates_need_scripts() {
        let pkg = |toml: &str| {
            pypi_name(&item("a/b", Manifests { pyproject_toml: Some(toml.into()), ..Manifests::default() }))
        };
        assert_eq!(
            pkg("[project]\nname = \"posting\"\n[project.scripts]\nposting = \"p:main\""),
            Some("posting".into())
        );
        assert_eq!(
            pkg("[tool.poetry]\nname = \"dolphie\"\n[tool.poetry.scripts]\ndolphie = \"d:main\""),
            Some("dolphie".into())
        );
        assert_eq!(pkg("[project]\nname = \"justalib\""), None);
    }

    #[test]
    fn npm_bin_comes_from_the_published_latest() {
        assert!(npm_runnable(&json!({ "bin": { "carbonyl": "index.sh" } })));
        assert!(npm_runnable(&json!({ "bin": "cli.js" })));
        assert!(!npm_runnable(&json!({ "bin": null })));
        assert!(!npm_runnable(&json!({ "name": "lib" })));
    }

    #[test]
    fn go_candidates_must_live_in_the_repo() {
        let go = |repo: &str, module: &str, main: bool| {
            go_package(&item(
                repo,
                Manifests {
                    go_mod: Some(format!("module {module}\n\ngo 1.22\n")),
                    has_root_main_go: main,
                    ..Manifests::default()
                },
            ))
        };
        assert_eq!(
            go("jesseduffield/lazygit", "github.com/jesseduffield/lazygit", true),
            Some("github.com/jesseduffield/lazygit".into())
        );
        assert_eq!(
            go("derailed/k9s", "github.com/derailed/k9s/v2", true),
            Some("github.com/derailed/k9s/v2".into())
        );
        assert_eq!(go("a/b", "github.com/a/b", false), None);
        assert_eq!(go("a/b", "github.com/someone/else", true), None);
        assert_eq!(go("a/b", "example.com/b", true), None);
    }

    #[test]
    fn verification_requires_a_link_back() {
        let it = item(
            "clementtsang/bottom",
            Manifests {
                cargo_toml: Some("[package]\nname = \"bottom\"".into()),
                has_rust_bin: true,
                ..Manifests::default()
            },
        );
        let items = [it];
        let run = |urls: Option<Vec<Value>>| {
            verify(
                items.iter().enumerate(),
                Ecosystem::Crates,
                Duration::ZERO,
                |i| cargo_name(i).into_iter().collect(),
                |_| Ok(urls.clone()),
            )
        };
        assert_eq!(
            run(Some(vec![json!("https://github.com/ClementTsang/bottom")])),
            [(0, Ecosystem::Crates, "bottom".to_owned())]
        );
        assert!(run(Some(vec![json!("https://github.com/squatter/bottom"), json!(null)])).is_empty());
        assert!(run(None).is_empty());
    }
}
