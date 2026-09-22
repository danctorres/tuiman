//! Ecosystems that publish their whole package set as one file: the AUR and
//! nixpkgs. Both record an upstream URL per package, so matching is exact.
//! The dumps are large, so they are deserialized straight from the
//! decompression stream into the two or three fields that matter.

use std::collections::HashMap;
use std::io::{BufReader, Read};

use serde::Deserialize;
use tuiman_index::Ecosystem;

use super::Found;
use crate::http::Http;
use crate::model::{github_repo, Item};
use crate::Result;

const AUR_DUMP: &str = "https://aur.archlinux.org/packages-meta-v1.json.gz";
const NIX_DUMP: &str = "https://channels.nixos.org/nixos-unstable/packages.json.br";

/// Lower is better. Candidates for one repository are ranked, and the best
/// one wins; ties go to the smaller name so the pick is order-independent.
pub(super) type Rank = (u8, usize);

pub fn resolve_aur(http: &Http, items: &[Item]) -> Result<Vec<Found>> {
    let dump = flate2::read::GzDecoder::new(http.get_reader(AUR_DUMP)?);
    Ok(assign(items, Ecosystem::Aur, &aur_by_repo(BufReader::new(dump))?))
}

pub fn resolve_nix(http: &Http, items: &[Item]) -> Result<Vec<Found>> {
    let dump = brotli_decompressor::Decompressor::new(http.get_reader(NIX_DUMP)?, 64 << 10);
    Ok(assign(items, Ecosystem::Nix, &nix_by_repo(BufReader::new(dump))?))
}

pub(super) fn assign(
    items: &[Item],
    eco: Ecosystem,
    by_repo: &HashMap<String, (Rank, String)>,
) -> Vec<Found> {
    // A library's distro package is the library itself (brew's libuv), which
    // is what installing a library entry means.
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let keys = [&item.repo, &item.former_repo];
            let (_, name) = keys.into_iter().flatten().find_map(|key| by_repo.get(key))?;
            Some((i, eco, name.clone()))
        })
        .collect()
}

pub(super) fn offer(by_repo: &mut HashMap<String, (Rank, String)>, repo: String, rank: Rank, name: &str) {
    match by_repo.get_mut(&repo) {
        Some(best) if (best.0, best.1.as_str()) <= (rank, name) => {}
        Some(best) => *best = (rank, name.to_owned()),
        None => drop(by_repo.insert(repo, (rank, name.to_owned()))),
    }
}

#[derive(Deserialize)]
struct AurPackage {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "OutOfDate")]
    out_of_date: Option<u64>,
}

/// Prefers the plain package, then `-bin` (no compile), then `-git`.
fn aur_by_repo(dump: impl Read) -> Result<HashMap<String, (Rank, String)>> {
    let packages: Vec<AurPackage> = serde_json::from_reader(dump)?;
    let mut by_repo = HashMap::new();
    for p in packages.iter().filter(|p| p.out_of_date.is_none() && tuiman_index::valid_package_name(&p.name))
    {
        let Some(repo) = p.url.as_deref().and_then(github_repo) else { continue };
        let flavour = match p.name.rsplit_once('-').map(|(_, suffix)| suffix) {
            Some("bin") => 1,
            Some("git" | "nightly" | "beta" | "debug") => 2,
            _ => 0,
        };
        offer(&mut by_repo, repo, (flavour, p.name.len()), &p.name);
    }
    Ok(by_repo)
}

#[derive(Deserialize)]
struct NixDump {
    packages: HashMap<String, NixPackage>,
}

#[derive(Deserialize)]
struct NixPackage {
    #[serde(default)]
    meta: NixMeta,
}

#[derive(Default, Deserialize)]
struct NixMeta {
    #[serde(default)]
    homepage: Homepage,
}

/// nixpkgs allows one homepage or a list of them.
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Homepage {
    One(String),
    Many(Vec<String>),
    #[default]
    None,
}

/// Keys are attribute paths, which is what `nix-env -iA nixpkgs.<attr>` takes.
/// Top-level attributes beat nested sets (`python3Packages.x`), which exist
/// for a project's language bindings more often than for the project itself.
fn nix_by_repo(dump: impl Read) -> Result<HashMap<String, (Rank, String)>> {
    let dump: NixDump = serde_json::from_reader(dump)?;
    let mut by_repo = HashMap::new();
    for (attr, package) in dump.packages.iter().filter(|(attr, _)| tuiman_index::valid_package_name(attr)) {
        let homepages = match &package.meta.homepage {
            Homepage::One(url) => std::slice::from_ref(url),
            Homepage::Many(urls) => urls.as_slice(),
            Homepage::None => &[],
        };
        let nesting = attr.matches('.').count().min(255) as u8;
        for repo in homepages.iter().filter_map(|url| github_repo(url)) {
            offer(&mut by_repo, repo, (nesting, attr.len()), attr);
        }
    }
    Ok(by_repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aur_prefers_release_packages_and_skips_flagged_ones() {
        let dump = r#"[
            {"Name":"yazi-git","URL":"https://github.com/sxyazi/yazi","OutOfDate":null,"Popularity":1.0},
            {"Name":"yazi-bin","URL":"https://github.com/sxyazi/yazi","OutOfDate":null},
            {"Name":"yazi","URL":"https://github.com/sxyazi/yazi/","OutOfDate":null},
            {"Name":"stale","URL":"https://github.com/o/stale","OutOfDate":1700000000},
            {"Name":"nourl","URL":null,"OutOfDate":null},
            {"Name":"-evil","URL":"https://github.com/o/evil","OutOfDate":null},
            {"Name":"onlygit-git","URL":"https://github.com/o/onlygit.git","OutOfDate":null}
        ]"#;
        let map = aur_by_repo(dump.as_bytes()).unwrap();
        assert_eq!(map["sxyazi/yazi"].1, "yazi");
        assert_eq!(map["o/onlygit"].1, "onlygit-git");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn nix_prefers_top_level_attributes() {
        let dump = r#"{"version":2,"packages":{
            "python3Packages.btop":{"pname":"btop","meta":{"homepage":"https://github.com/aristocratos/btop"}},
            "btop":{"pname":"btop","meta":{"homepage":"https://github.com/aristocratos/btop#readme"}},
            "btop-rocm":{"meta":{"homepage":["https://example.com","https://github.com/aristocratos/btop"]}},
            "nometa":{},
            "nohome":{"meta":{"description":"x"}}
        }}"#;
        let map = nix_by_repo(dump.as_bytes()).unwrap();
        assert_eq!(map["aristocratos/btop"].1, "btop");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn assignment_follows_renames_and_includes_libraries() {
        let by_repo = HashMap::from([
            ("old/app".to_owned(), ((0, 3), "app".to_owned())),
            ("o/lib".to_owned(), ((0, 3), "lib".to_owned())),
        ]);
        let items = [
            Item { repo: Some("new/app".into()), former_repo: Some("old/app".into()), ..Item::default() },
            Item { repo: Some("o/lib".into()), library: true, ..Item::default() },
            Item { repo: None, ..Item::default() },
        ];
        assert_eq!(
            assign(&items, Ecosystem::Aur, &by_repo),
            [(0, Ecosystem::Aur, "app".to_owned()), (1, Ecosystem::Aur, "lib".to_owned())]
        );
    }

    #[test]
    fn rank_ties_do_not_depend_on_order() {
        for names in [["b.amd", "b.v3d"], ["b.v3d", "b.amd"]] {
            let mut by_repo = HashMap::new();
            for name in names {
                offer(&mut by_repo, "o/b".into(), (1, 5), name);
            }
            assert_eq!(by_repo["o/b"].1, "b.amd");
        }
    }
}
