//! Prebuilt binaries from the latest GitHub release: for each supported
//! platform, the one asset whose file name says it is for that platform.
//! Nothing is downloaded; the file name is the whole signal.

use tuiman_index::{is_archive, Ecosystem};

use super::Found;
use crate::model::Item;

/// (ecosystem, OS tokens, architecture tokens), matched against the
/// lower-cased asset name.
const PLATFORMS: [(Ecosystem, &[&str], &[&str]); 4] = [
    (Ecosystem::ReleaseLinuxX64, &["linux"], &["x86_64", "amd64", "x64"]),
    (Ecosystem::ReleaseLinuxArm64, &["linux"], &["aarch64", "arm64"]),
    (Ecosystem::ReleaseMacosX64, &["darwin", "macos", "apple", "osx"], &["x86_64", "amd64", "x64"]),
    (Ecosystem::ReleaseMacosArm64, &["darwin", "macos", "apple", "osx"], &["aarch64", "arm64"]),
];

pub fn resolve(items: &[Item]) -> Vec<Found> {
    let mut found = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let (Some(repo), Some((tag, assets))) = (&item.repo, &item.release) else { continue };
        for (eco, os, arch) in PLATFORMS {
            if let Some(asset) = pick(assets, os, arch) {
                found.push((i, eco, format!("{repo}/{tag}/{asset}")));
            }
        }
    }
    found
}

/// Whether `name` is an archive or a bare executable, rather than a package,
/// checksum or signature. A bare binary has no extension: its last
/// dot-separated part is not an alphanumeric suffix like `deb`, `sha256` or
/// `sha256sum` (a version such as `2.0.1_linux_amd64` has separators in it).
fn is_binary(name: &str) -> bool {
    is_archive(name)
        || match name.rsplit_once('.') {
            Some((_, ext)) => !ext.bytes().all(|b| b.is_ascii_alphanumeric()),
            None => true,
        }
}

/// The best asset for one platform: static (musl) builds first, since they
/// run on any distribution, then anything over a zip, which needs `unzip`;
/// universal macOS binaries count for both arches.
fn pick<'a>(assets: &'a [String], os: &[&str], arch: &[&str]) -> Option<&'a str> {
    let mut best: Option<(u8, &str)> = None;
    for asset in assets {
        let lower = asset.to_ascii_lowercase();
        let has = |tokens: &[&str]| tokens.iter().any(|t| lower.contains(t));
        let arch_ok = has(arch) || (os[0] != "linux" && lower.contains("universal"));
        if !is_binary(&lower)
            || !has(os)
            || !arch_ok
            || lower.contains("android")
            || lower.contains("debug")
            || lower.ends_with("-update")
        {
            continue;
        }
        let score = u8::from(lower.contains("musl")) * 4
            + u8::from(lower.contains("static")) * 2
            + u8::from(!lower.ends_with(".zip"));
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, asset));
        }
    }
    best.map(|(_, asset)| asset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn resolve_one(assets: &[&str]) -> Vec<(Ecosystem, String)> {
        let item =
            Item { repo: Some("o/r".into()), release: Some(("v1".into(), names(assets))), ..Item::default() };
        resolve(&[item]).into_iter().map(|(_, eco, pkg)| (eco, pkg)).collect()
    }

    #[test]
    fn picks_one_asset_per_platform_from_real_release_listings() {
        let lazygit = resolve_one(&[
            "lazygit_0.65.1_linux_32-bit.tar.gz",
            "lazygit_0.65.1_freebsd_x86_64.tar.gz",
            "lazygit_0.65.1_windows_x86_64.zip",
            "lazygit_0.65.1_darwin_arm64.tar.gz",
            "lazygit_0.65.1_linux_x86_64.tar.gz",
            "lazygit_0.65.1_linux_arm64.tar.gz",
            "checksums.txt",
        ]);
        assert_eq!(
            lazygit,
            [
                (Ecosystem::ReleaseLinuxX64, "o/r/v1/lazygit_0.65.1_linux_x86_64.tar.gz".into()),
                (Ecosystem::ReleaseLinuxArm64, "o/r/v1/lazygit_0.65.1_linux_arm64.tar.gz".into()),
                (Ecosystem::ReleaseMacosArm64, "o/r/v1/lazygit_0.65.1_darwin_arm64.tar.gz".into()),
            ]
        );
        let bottom = resolve_one(&[
            "bottom-musl_0.14.9-1_amd64.deb",
            "bottom_0.14.9-1_amd64.deb",
            "bottom_aarch64-linux-android.tar.gz",
            "bottom_aarch64-unknown-linux-gnu.tar.gz",
            "bottom_x86_64-apple-darwin.tar.gz",
            "bottom_x86_64-unknown-linux-gnu.tar.gz",
            "bottom_x86_64-unknown-linux-musl.tar.gz",
            "bottom_x86_64_installer.msi",
            "bottom.desktop",
        ]);
        assert_eq!(
            bottom,
            [
                (Ecosystem::ReleaseLinuxX64, "o/r/v1/bottom_x86_64-unknown-linux-musl.tar.gz".into()),
                (Ecosystem::ReleaseLinuxArm64, "o/r/v1/bottom_aarch64-unknown-linux-gnu.tar.gz".into()),
                (Ecosystem::ReleaseMacosX64, "o/r/v1/bottom_x86_64-apple-darwin.tar.gz".into()),
            ]
        );
        let k9s = resolve_one(&[
            "k9s_Linux_amd64.tar.gz",
            "k9s_Linux_amd64.tar.gz.sbom.json",
            "k9s_linux_amd64.apk",
        ]);
        assert_eq!(k9s, [(Ecosystem::ReleaseLinuxX64, "o/r/v1/k9s_Linux_amd64.tar.gz".into())]);
    }

    #[test]
    fn bare_binaries_and_universal_builds() {
        let got = resolve_one(&[
            "gh_2.0.1_linux_amd64",
            "app-macos-universal.zip",
            "app-linux-x64.sha256",
            "app-linux-x64.tar.gz.sha256sum",
            "app-linux-x64.provenance",
            "app-x86_64-unknown-linux-musl-update",
        ]);
        assert_eq!(
            got,
            [
                (Ecosystem::ReleaseLinuxX64, "o/r/v1/gh_2.0.1_linux_amd64".into()),
                (Ecosystem::ReleaseMacosX64, "o/r/v1/app-macos-universal.zip".into()),
                (Ecosystem::ReleaseMacosArm64, "o/r/v1/app-macos-universal.zip".into()),
            ]
        );
        assert!(resolve_one(&["src.tar.gz", "windows-x64.zip"]).is_empty());
        let tar_over_zip = resolve_one(&["x-linux-x64.zip", "x-linux-x64.tar.gz"]);
        assert_eq!(tar_over_zip, [(Ecosystem::ReleaseLinuxX64, "o/r/v1/x-linux-x64.tar.gz".into())]);
        let no_debug = resolve_one(&["x-linux-x64-debug.tar.gz", "x-linux-x64.tar.gz"]);
        assert_eq!(no_debug, [(Ecosystem::ReleaseLinuxX64, "o/r/v1/x-linux-x64.tar.gz".into())]);
    }
}
