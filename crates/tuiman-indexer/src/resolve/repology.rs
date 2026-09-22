//! Distribution packages (apt, dnf, pacman) via Repology.
//!
//! Distributions do not publish "package → upstream repository" in bulk, and
//! Repology's API does not expose URLs either, so a lookup is never trusted on
//! its name alone. It is *anchored*: a Repology project is accepted for an
//! entry only if it also contains a package this run already matched to the
//! entry by URL in another ecosystem (Homebrew, AUR, nixpkgs). Same Repology
//! project + same verified package = same software.

use std::thread;
use std::time::Duration;

use serde_json::Value;
use tuiman_index::Ecosystem;

use super::Found;
use crate::http::Http;
use crate::model::Item;

/// Repology asks API users to stay at or below one request per second.
const PAUSE: Duration = Duration::from_millis(1100);

/// Repology repositories whose packages were matched by URL elsewhere.
const ANCHORS: [(&str, Ecosystem); 3] =
    [("homebrew", Ecosystem::Brew), ("aur", Ecosystem::Aur), ("nix_unstable", Ecosystem::Nix)];

/// Maps a Repology repository name to the ecosystem it fills in and a rank,
/// so the newest stable release can be preferred. Debian and Ubuntu both feed
/// apt but number releases differently (`13` vs `24_04`), so any Debian
/// release outranks any Ubuntu one: Debian is upstream of Ubuntu's names.
/// Rolling and pre-release repositories (`debian_unstable`, `fedora_rawhide`)
/// are skipped: a package that only exists there cannot be installed by most.
fn target(repo: &str) -> Option<(Ecosystem, u32)> {
    if repo == "arch" {
        return Some((Ecosystem::Pacman, 0));
    }
    let (eco, base, release) = [
        ("debian_", Ecosystem::Apt, 1 << 20),
        ("ubuntu_", Ecosystem::Apt, 0),
        ("fedora_", Ecosystem::Dnf, 0),
    ]
    .iter()
    .find_map(|&(prefix, eco, base)| Some((eco, base, repo.strip_prefix(prefix)?)))?;
    let digits: String = release.chars().filter(|c| *c != '_').collect();
    Some((eco, base + digits.parse::<u32>().ok()?))
}

pub fn resolve(http: &Http, items: &[Item]) -> Vec<Found> {
    let mut found = Vec::new();
    let mut failures = 0;
    // Distro packages of a library are the library itself, not a binary (see bulk).
    for (i, item) in items.iter().enumerate().filter(|(_, item)| !item.library) {
        // Repology project names are usually the most common package name.
        let Some(project) = ANCHORS.iter().find_map(|(_, eco)| item.packages.get(eco.name())) else {
            continue;
        };
        let project = project.to_ascii_lowercase();
        if !project.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b)) {
            continue;
        }
        match http.get_json(&format!("https://repology.org/api/v1/project/{project}")) {
            Ok(Some(packages)) => {
                failures = 0;
                found.extend(distro_packages(item, &packages).into_iter().map(|(e, p)| (i, e, p)))
            }
            Ok(None) => failures = 0,
            Err(e) => {
                eprintln!("warn: repology: {project}: {e}");
                failures += 1;
                if failures >= 5 {
                    eprintln!("warn: repology: giving up after 5 failures in a row");
                    break;
                }
            }
        }
        thread::sleep(PAUSE);
    }
    found
}

fn package_name(package: &Value) -> Option<&str> {
    ["binname", "srcname", "visiblename"].iter().find_map(|key| package[key].as_str())
}

fn distro_packages(item: &Item, packages: &Value) -> Vec<(Ecosystem, String)> {
    let packages = packages.as_array().map(Vec::as_slice).unwrap_or_default();

    let anchored = ANCHORS.iter().any(|(repo, eco)| {
        let ours = item.packages.get(eco.name());
        packages
            .iter()
            .any(|p| p["repo"].as_str() == Some(repo) && ours.is_some_and(|o| package_name(p) == Some(o)))
    });
    if !anchored {
        return Vec::new();
    }

    // Newest release of each ecosystem wins.
    let mut best: Vec<(Ecosystem, u32, &str)> = Vec::new();
    for p in packages.iter().filter(|p| p["status"].as_str() != Some("legacy")) {
        let Some((eco, release)) = p["repo"].as_str().and_then(target) else { continue };
        let Some(name) = package_name(p).filter(|n| tuiman_index::valid_package_name(n)) else { continue };
        match best.iter_mut().find(|(have, _, _)| *have == eco) {
            Some(entry) if entry.1 < release => *entry = (eco, release, name),
            Some(_) => {}
            None => best.push((eco, release, name)),
        }
    }
    best.into_iter().map(|(eco, _, name)| (eco, name.to_owned())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(brew: &str) -> Item {
        let mut item = Item::default();
        item.packages.insert("brew", brew.to_owned());
        item
    }

    #[test]
    fn anchored_projects_yield_distro_packages() {
        let project = json!([
            { "repo": "homebrew", "srcname": "btop", "visiblename": "btop" },
            { "repo": "debian_9", "srcname": "btop", "binname": "btop-ancient" },
            { "repo": "debian_13", "srcname": "btop", "binname": "btop" },
            { "repo": "debian_12", "srcname": "btop", "binname": "btop-old" },
            { "repo": "ubuntu_24_04", "srcname": "btop", "binname": "btop-ubuntu" },
            { "repo": "debian_unstable", "srcname": "btop", "binname": "btop-sid" },
            { "repo": "archpower", "binname": "not-arch" },
            { "repo": "homebrew_casks", "srcname": "btop" },
            { "repo": "fedora_42", "srcname": "btop", "visiblename": "btop" },
            { "repo": "arch", "subrepo": "extra", "srcname": "btop", "binname": "btop" },
            { "repo": "gentoo", "srcname": "sys-process/btop" }
        ]);
        let mut got = distro_packages(&item("btop"), &project);
        got.sort();
        assert_eq!(
            got,
            [
                (Ecosystem::Apt, "btop".to_owned()),
                (Ecosystem::Dnf, "btop".to_owned()),
                (Ecosystem::Pacman, "btop".to_owned())
            ]
        );
    }

    #[test]
    fn unanchored_projects_are_ignored() {
        // A different program that merely shares the name: its Homebrew
        // package is not the one we matched by URL.
        let project = json!([
            { "repo": "homebrew", "srcname": "something-else" },
            { "repo": "homebrew_casks", "srcname": "sc-im" },
            { "repo": "debian_13", "binname": "sc" }
        ]);
        assert!(distro_packages(&item("sc-im"), &project).is_empty());
        assert!(distro_packages(&item("sc-im"), &json!({"error": "nope"})).is_empty());
    }
}
