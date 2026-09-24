//! Where tuiman keeps its cache. Everything in it can be deleted at any time.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

/// `$XDG_CACHE_HOME/tuiman` (or `~/.cache/tuiman`) on Linux,
/// `~/Library/Caches/tuiman` on macOS.
pub fn cache_dir() -> Option<PathBuf> {
    let absolute = |var: &str| env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(dir) = absolute("TUIMAN_CACHE_DIR") {
        return Some(dir);
    }
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Caches")
    } else {
        absolute("XDG_CACHE_HOME").or_else(|| Some(home()?.join(".cache")))?
    };
    Some(base.join("tuiman"))
}

pub fn home() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Resolves `bin` against `PATH` without spawning anything.
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).map(|dir| dir.join(bin)).find(|candidate| is_executable(candidate))
}

/// Every file name in the `PATH` directories. Lists directories only, with no
/// per-file stat: what sits in a bin directory is an executable in practice.
pub fn names_on_path() -> HashSet<String> {
    let path = env::var_os("PATH").unwrap_or_default();
    let mut seen = HashSet::new();
    env::split_paths(&path)
        .filter(|dir| seen.insert(dir.clone()))
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten().filter_map(|e| e.file_name().into_string().ok()))
        .collect()
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
