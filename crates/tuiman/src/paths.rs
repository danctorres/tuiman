//! Where tuiman keeps its cache. Everything in it can be deleted at any time.

use std::env;
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

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
