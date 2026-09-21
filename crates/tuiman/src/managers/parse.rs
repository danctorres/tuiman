//! Pure parsers for "what is installed" listings, one per source format.

use std::path::{Path, PathBuf};

/// One package name per line (`pacman -Qq`, `rpm -qa --qf '%{NAME}\n'`).
pub fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// `~/.cargo/.crates.toml`: keys look like `"bottom 0.10.2 (registry+https://…)"`.
pub fn cargo_crates_toml(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| Some(l.trim().strip_prefix('"')?.split(' ').next()?.to_owned())).collect()
}

/// The `prefix` setting of an `.npmrc`, with `~` and `${HOME}` expanded.
pub fn npmrc_prefix(npmrc: &str, home: Option<&Path>) -> Option<PathBuf> {
    let value = npmrc.lines().find_map(|l| {
        let (key, value) = l.split_once('=')?;
        (key.trim() == "prefix").then(|| value.trim().trim_matches(['"', '\'']))
    })?;
    let relative = value.strip_prefix("~/").or_else(|| value.strip_prefix("${HOME}/"));
    match relative {
        Some(rest) => Some(home?.join(rest)),
        None => Some(PathBuf::from(value)).filter(|p| p.is_absolute()),
    }
}

/// `/var/lib/dpkg/status`: stanzas with `Package:` and `Status:` fields. Only
/// fully installed packages count (not `deinstall ok config-files`).
pub fn dpkg_status(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut package = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("Package: ") {
            package = Some(name.trim());
        } else if line.strip_prefix("Status: ").is_some_and(|s| s.trim() == "install ok installed") {
            out.extend(package.take().map(str::to_owned));
        } else if line.is_empty() {
            package = None;
        }
    }
    out
}

/// `nix-env -q`: `name-version`. The version starts at the first `-` that is
/// followed by a digit, which is how Nix itself splits derivation names.
pub fn nix_env_query(text: &str) -> Vec<String> {
    let names = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|line| {
        let bytes = line.as_bytes();
        let cut =
            (0..bytes.len().saturating_sub(1)).find(|&i| bytes[i] == b'-' && bytes[i + 1].is_ascii_digit());
        line[..cut.unwrap_or(line.len())].to_owned()
    });
    names.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines() {
        assert_eq!(lines("btop\n  lazygit \n\nyazi\n"), ["btop", "lazygit", "yazi"]);
    }

    #[test]
    fn cargo() {
        let toml =
            "[v1]\n\"bottom 0.10.2 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"btm\"]\n\
                    \"gitui 0.26.3 (git+https://github.com/extrawurst/gitui#abc)\" = [\"gitui\"]\n";
        assert_eq!(cargo_crates_toml(toml), ["bottom", "gitui"]);
    }

    #[test]
    fn npmrc() {
        let home = Path::new("/home/u");
        assert_eq!(
            npmrc_prefix("a=b\nprefix=~/.npm-global\n", Some(home)),
            Some("/home/u/.npm-global".into())
        );
        assert_eq!(npmrc_prefix("prefix = \"${HOME}/n\"", Some(home)), Some("/home/u/n".into()));
        assert_eq!(npmrc_prefix("prefix=/opt/npm", None), Some("/opt/npm".into()));
        assert_eq!(npmrc_prefix("prefix=relative/dir", Some(home)), None);
        assert_eq!(npmrc_prefix("registry=https://r", Some(home)), None);
    }

    #[test]
    fn dpkg() {
        let status = "Package: htop\nStatus: install ok installed\nPriority: optional\n\n\
                      Package: removed\nStatus: deinstall ok config-files\n\n\
                      Package: btop\nArchitecture: amd64\nStatus: install ok installed\n";
        assert_eq!(dpkg_status(status), ["htop", "btop"]);
    }

    #[test]
    fn nix() {
        assert_eq!(
            nix_env_query("btop-1.3.2\nlazygit-0.44.1\ngit-crypt-0.7.0\nnoversion\n"),
            ["btop", "lazygit", "git-crypt", "noversion"]
        );
    }
}
