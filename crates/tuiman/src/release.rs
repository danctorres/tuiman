//! Prebuilt binaries from GitHub releases, for TUIs no package manager here
//! carries. The index names one asset per platform as `owner/repo/tag/asset`;
//! tuiman downloads it, unpacks it and puts the executables in `~/.local/bin`.
//! A manifest per repository under `$XDG_DATA_HOME/tuiman/releases` records
//! the tag and the files, which is what makes upgrade and uninstall possible.
//!
//! This runs as `tuiman release install|uninstall PKG`, a subprocess like any
//! other manager, so its output streams into the job log unchanged.

use std::io::{self, Write as _};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;
use std::{env, fs};

use tuiman_index::is_archive;

use crate::managers::{data_home, dir_entries};
use crate::{fetch, paths};

const MAX_ASSET_BYTES: u64 = 256 << 20;

pub fn cli(args: &[String]) -> io::Result<ExitCode> {
    let result = match args {
        [verb, package] if verb == "install" || verb == "uninstall" => match parts(package) {
            Some((repo, tag, asset)) if verb == "install" => install(repo, tag, asset),
            Some((repo, _, _)) => uninstall(repo),
            None => Err(format!("{package} is not owner/repo/tag/asset")),
        },
        _ => Err("usage: tuiman release install|uninstall OWNER/REPO/TAG/ASSET".into()),
    };
    Ok(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("tuiman: {why}");
            ExitCode::FAILURE
        }
    })
}

/// `owner/repo` of a release package; the key its manifest is filed under.
pub fn repo(package: &str) -> &str {
    let end = package.match_indices('/').nth(1).map_or(package.len(), |(i, _)| i);
    &package[..end]
}

pub fn tag(package: &str) -> &str {
    parts(package).map_or(package, |(_, tag, _)| tag)
}

/// `(owner/repo, tag, asset)`. Tags may contain `/`; assets cannot.
fn parts(package: &str) -> Option<(&str, &str, &str)> {
    let (repo, rest) = package.split_at(repo(package).len());
    let (tag, asset) = rest.strip_prefix('/')?.rsplit_once('/')?;
    let ok = |s: &str| !s.is_empty() && s != "." && s != "..";
    (repo.split('/').count() == 2 && repo.split('/').all(ok) && ok(tag) && ok(asset))
        .then_some((repo, tag, asset))
}

/// `owner/repo` of everything installed from a release.
pub fn installed() -> Vec<String> {
    let Some(dir) = manifest_dir() else { return Vec::new() };
    let owners = dir_entries(&dir);
    owners
        .into_iter()
        .flat_map(|owner| {
            dir_entries(&dir.join(&owner)).into_iter().map(move |repo| format!("{owner}/{repo}"))
        })
        .collect()
}

fn manifest_dir() -> Option<PathBuf> {
    Some(data_home()?.join("tuiman/releases"))
}

fn bin_dir() -> Result<PathBuf, String> {
    Ok(paths::home().ok_or("HOME is not set")?.join(".local/bin"))
}

/// First line the tag, then one installed path per line.
fn read_manifest(repo: &str) -> Option<(String, Vec<PathBuf>)> {
    let text = fs::read_to_string(manifest_dir()?.join(repo)).ok()?;
    let mut lines = text.lines();
    Some((lines.next()?.to_owned(), lines.map(PathBuf::from).collect()))
}

fn install(repo: &str, tag: &str, asset: &str) -> Result<(), String> {
    if read_manifest(repo).is_some_and(|(have, _)| have == tag) {
        println!("{repo} is already at {tag}");
        return Ok(());
    }
    let work = crate::cache_dir().map_err(|e| e.to_string())?.join(format!("release.{}", std::process::id()));
    let result = install_from(repo, tag, asset, &work);
    let _ = fs::remove_dir_all(&work);
    result
}

fn install_from(repo: &str, tag: &str, asset: &str, work: &Path) -> Result<(), String> {
    // Find the unpacker before paying for the download.
    let extractor = is_archive(asset).then(|| extractor(asset)).transpose()?;
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
    println!("downloading {url}");
    let file = work.join(asset);
    let bytes = download(&url, &file)?;
    println!("{:.1} MB", bytes as f64 / 1e6);

    let bins = if let Some(extractor) = extractor {
        let unpacked = work.join("unpacked");
        extract(&extractor, &file, &unpacked)?;
        let mut bins = Vec::new();
        executables(&unpacked, &mut bins, 0);
        bins
    } else {
        // A bare binary is named after the asset, minus the version and platform.
        let named = work.join(binary_name(asset).unwrap_or_else(|| repo.rsplit('/').next().unwrap_or(repo)));
        fs::rename(&file, &named).map_err(|e| e.to_string())?;
        vec![named]
    };
    if bins.is_empty() {
        return Err(format!("no executable found in {asset}"));
    }

    let bin_dir = bin_dir()?;
    fs::create_dir_all(&bin_dir).map_err(|e| format!("{}: {e}", bin_dir.display()))?;
    let mut manifest = format!("{tag}\n");
    let mut installed = Vec::new();
    for bin in &bins {
        let dest = bin_dir.join(bin.file_name().unwrap_or_default());
        let _ = fs::remove_file(&dest);
        fs::copy(bin, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        println!("installed {}", dest.display());
        manifest.push_str(&format!("{}\n", dest.display()));
        installed.push(dest);
    }
    // Only now the old version's files, which may have had other names: a
    // failed copy above leaves the previous install intact.
    if let Some((_, old)) = read_manifest(repo) {
        old.iter().filter(|p| !installed.contains(p)).for_each(|path| drop(fs::remove_file(path)));
    }
    let manifest_path = manifest_dir().ok_or("HOME is not set")?.join(repo);
    fs::create_dir_all(manifest_path.parent().unwrap_or(&manifest_path)).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, manifest).map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    let on_path = env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d == bin_dir));
    if !on_path {
        println!("note: {} is not on your PATH", bin_dir.display());
    }
    Ok(())
}

fn uninstall(repo: &str) -> Result<(), String> {
    let Some((_, files)) = read_manifest(repo) else {
        return Err(format!("{repo} was not installed by tuiman"));
    };
    for path in files {
        match fs::remove_file(&path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    let manifest = manifest_dir().ok_or("HOME is not set")?.join(repo);
    fs::remove_file(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    // The owner directory, if this was its last repository.
    let _ = manifest.parent().map(fs::remove_dir);
    Ok(())
}

/// Streams the asset to `to`. GitHub redirects to its object store, which
/// ureq follows; the size cap keeps a wrong asset from filling the disk.
fn download(url: &str, to: &Path) -> Result<u64, String> {
    let request = fetch::agent()
        .get(url)
        .config()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build();
    let mut response = request.call().map_err(|e| e.to_string())?;
    let mut body = response.body_mut().with_config().limit(MAX_ASSET_BYTES).reader();
    let mut file = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let bytes = io::copy(&mut body, &mut file).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The program that unpacks `asset`: `tar` reads every compressed tarball;
/// zips need `unzip` or `bsdtar` (GNU tar does not read them).
fn extractor(asset: &str) -> Result<PathBuf, String> {
    let zip = asset.to_ascii_lowercase().ends_with(".zip");
    let candidates: &[&str] = if zip { &["unzip", "bsdtar"] } else { &["tar", "bsdtar"] };
    candidates
        .iter()
        .find_map(|bin| paths::which(bin))
        .ok_or_else(|| format!("{asset} needs {} to unpack, which is not installed", candidates.join(" or ")))
}

fn extract(extractor: &Path, archive: &Path, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut command = Command::new(extractor);
    match extractor.file_name().is_some_and(|n| n == "unzip") {
        true => command.arg("-qo").arg(archive).arg("-d").arg(dir),
        false => command.arg("xf").arg(archive).arg("-C").arg(dir),
    };
    println!("unpacking {}", archive.file_name().unwrap_or_default().to_string_lossy());
    let status = command
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", command.get_program().to_string_lossy()))?;
    status.success().then_some(()).ok_or_else(|| format!("unpacking {} failed", archive.display()))
}

/// Executable files without an extension, up to three directories deep;
/// symlinks count as what they point to.
/// ponytail: all-caps names (LICENSE, README) are skipped because sloppy
/// tarballs mark everything executable; a binary named in caps is lost.
fn executables(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(meta) = fs::metadata(entry.path()) else { continue };
        if meta.is_dir() {
            if depth < 3 {
                executables(&entry.path(), out, depth + 1);
            }
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let plain = !name.contains('.') && name.chars().any(|c| c.is_ascii_lowercase());
        if meta.is_file() && plain && meta.permissions().mode() & 0o111 != 0 {
            out.push(entry.path());
        }
    }
}

/// `lazygit-linux-amd64` → `lazygit`, `ctop-0.7.7-linux-amd64` → `ctop`:
/// the name before the first version or platform token.
fn binary_name(asset: &str) -> Option<&str> {
    const PLATFORM: [&str; 16] = [
        "linux",
        "darwin",
        "macos",
        "apple",
        "osx",
        "x86_64",
        "amd64",
        "x64",
        "aarch64",
        "arm64",
        "musl",
        "gnu",
        "unknown",
        "static",
        "linux64",
        "universal",
    ];
    let is_marker = |token: &str| {
        let lower = token.to_ascii_lowercase();
        let version = lower.strip_prefix('v').unwrap_or(&lower).starts_with(|c: char| c.is_ascii_digit());
        version || PLATFORM.contains(&lower.as_str())
    };
    let mut end = 0;
    for token in asset.split(['-', '_']) {
        if is_marker(token) {
            break;
        }
        end += token.len() + 1;
    }
    let name = asset[..end.min(asset.len())].trim_end_matches(['-', '_']);
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_parts() {
        assert_eq!(
            parts("o/r/v1.2/r_1.2_linux_x86_64.tar.gz"),
            Some(("o/r", "v1.2", "r_1.2_linux_x86_64.tar.gz"))
        );
        assert_eq!(parts("o/r/release/1.0/r-linux"), Some(("o/r", "release/1.0", "r-linux")));
        assert_eq!(repo("o/r/v1/a"), "o/r");
        assert_eq!(repo("o/r"), "o/r");
        assert_eq!(tag("o/r/v1/a"), "v1");
        for bad in ["o/r", "o/r/v1", "o/../v1/a", "o/r/v1/..", "o/r//a"] {
            assert_eq!(parts(bad), None, "{bad}");
        }
    }

    #[test]
    fn bare_binary_names() {
        assert_eq!(binary_name("lazygit-linux-amd64"), Some("lazygit"));
        assert_eq!(binary_name("ctop-0.7.7-linux-amd64"), Some("ctop"));
        assert_eq!(binary_name("gh-dash_v4.26.0_linux-amd64"), Some("gh-dash"));
        assert_eq!(binary_name("ec2-instance-selector-linux-amd64"), Some("ec2-instance-selector"));
        assert_eq!(binary_name("mal-cli-v0.2.1-linux-musl-x86_64"), Some("mal-cli"));
        assert_eq!(binary_name("go-life_linux_amd64"), Some("go-life"));
        assert_eq!(binary_name("grv_v0.3.2_linux64"), Some("grv"));
        assert_eq!(binary_name("plain"), Some("plain"));
        assert_eq!(binary_name("v1.0-linux"), None);
    }

    #[test]
    fn finds_executables_in_an_unpacked_archive() {
        let dir = env::temp_dir().join(format!("tuiman-release-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pkg/bin")).unwrap();
        let exe = |path: &str| {
            let p = dir.join(path);
            fs::write(&p, "").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        };
        exe("pkg/bin/tool");
        std::os::unix::fs::symlink("bin/tool", dir.join("pkg/link")).unwrap();
        exe("pkg/LICENSE");
        exe("pkg/install.sh");
        fs::write(dir.join("pkg/README.md"), "").unwrap();
        fs::write(dir.join("pkg/helper"), "").unwrap();
        let mut found = Vec::new();
        executables(&dir, &mut found, 0);
        found.sort();
        assert_eq!(found, [dir.join("pkg/bin/tool"), dir.join("pkg/link")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
