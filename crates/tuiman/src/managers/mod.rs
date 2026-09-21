//! Package managers tuiman can drive. They are data: one static table row
//! each, with command templates and a function that lists what is installed.
//!
//! Listing reads the manager's own database from disk where that is possible
//! (Homebrew's Cellar, Cargo's `.crates.toml`, dpkg's status file, the npm,
//! uv and pipx tool directories, Go's bin directory) and only spawns the
//! manager where it is not: starting `npm` alone costs more than everything
//! else tuiman does at startup combined.

mod parse;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs, thread};

use tuiman_index::Ecosystem;

use crate::paths;

/// Index into [`MANAGERS`]; also the bit position in per-row manager masks.
pub type ManagerId = u8;
/// One bit per manager.
pub type Mask = u16;

pub struct Manager {
    pub name: &'static str,
    pub bin: &'static str,
    /// The ecosystem whose package names this manager understands.
    pub eco: Ecosystem,
    /// Argument templates; `{pkg}` is replaced by the package name.
    install: &'static [&'static str],
    uninstall: &'static [&'static str],
    /// Prefix with `sudo` when not already root.
    pub sudo: bool,
    /// Needs a real terminal (password or confirmation prompts), so the TUI
    /// steps aside instead of capturing the output.
    pub tty: bool,
    /// Names of everything this manager reports as installed.
    pub list_installed: fn(&Path) -> Vec<String>,
}

pub const MANAGERS: [Manager; 12] = [
    Manager {
        name: "brew",
        bin: "brew",
        eco: Ecosystem::Brew,
        install: &["install", "{pkg}"],
        uninstall: &["uninstall", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: list_brew,
    },
    Manager {
        name: "cargo",
        bin: "cargo",
        eco: Ecosystem::Crates,
        install: &["install", "--locked", "{pkg}"],
        uninstall: &["uninstall", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: list_cargo,
    },
    Manager {
        name: "go",
        bin: "go",
        eco: Ecosystem::Go,
        install: &["install", "{pkg}@latest"],
        uninstall: &[], // Go has no uninstall; see `Manager::argv`.
        sudo: false,
        tty: false,
        list_installed: list_go,
    },
    Manager {
        name: "npm",
        bin: "npm",
        eco: Ecosystem::Npm,
        install: &["install", "--global", "{pkg}"],
        uninstall: &["uninstall", "--global", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: list_npm,
    },
    Manager {
        name: "uv",
        bin: "uv",
        eco: Ecosystem::Pypi,
        install: &["tool", "install", "{pkg}"],
        uninstall: &["tool", "uninstall", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: list_uv,
    },
    Manager {
        name: "pipx",
        bin: "pipx",
        eco: Ecosystem::Pypi,
        install: &["install", "{pkg}"],
        uninstall: &["uninstall", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: list_pipx,
    },
    Manager {
        name: "apt",
        bin: "apt-get",
        eco: Ecosystem::Apt,
        install: &["install", "{pkg}"],
        uninstall: &["remove", "{pkg}"],
        sudo: true,
        tty: true,
        list_installed: |_| {
            parse::dpkg_status(&fs::read_to_string("/var/lib/dpkg/status").unwrap_or_default())
        },
    },
    Manager {
        name: "dnf",
        bin: "dnf",
        eco: Ecosystem::Dnf,
        install: &["install", "{pkg}"],
        uninstall: &["remove", "{pkg}"],
        sudo: true,
        tty: true,
        list_installed: |_| parse::lines(&run(Path::new("rpm"), &["-qa", "--qf", "%{NAME}\\n"])),
    },
    Manager {
        name: "pacman",
        bin: "pacman",
        eco: Ecosystem::Pacman,
        install: &["-S", "{pkg}"],
        uninstall: &["-Rs", "{pkg}"],
        sudo: true,
        tty: true,
        list_installed: |bin| parse::lines(&run(bin, &["-Qqn"])),
    },
    Manager {
        name: "paru",
        bin: "paru",
        eco: Ecosystem::Aur,
        install: &["-S", "{pkg}"],
        uninstall: &["-Rs", "{pkg}"],
        sudo: false,
        tty: true,
        list_installed: |_| parse::lines(&run(Path::new("pacman"), &["-Qqm"])),
    },
    Manager {
        name: "yay",
        bin: "yay",
        eco: Ecosystem::Aur,
        install: &["-S", "{pkg}"],
        uninstall: &["-Rs", "{pkg}"],
        sudo: false,
        tty: true,
        list_installed: |_| parse::lines(&run(Path::new("pacman"), &["-Qqm"])),
    },
    Manager {
        name: "nix",
        bin: "nix-env",
        eco: Ecosystem::Nix,
        install: &["--install", "--attr", "nixpkgs.{pkg}"],
        uninstall: &["--uninstall", "{pkg}"],
        sudo: false,
        tty: false,
        list_installed: |bin| parse::nix_env_query(&run(bin, &["--query"])),
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Uninstall,
}

impl Action {
    pub fn verb(self) -> &'static str {
        match self {
            Action::Install => "install",
            Action::Uninstall => "uninstall",
        }
    }
}

pub fn by_name(name: &str) -> Option<ManagerId> {
    MANAGERS.iter().position(|m| m.name == name).map(|i| i as ManagerId)
}

impl Manager {
    /// The name this manager's listing uses for `package`. Go only leaves a
    /// binary behind, named after the last path element of the module
    /// (ignoring a `/vN` major-version suffix).
    pub fn installed_name<'a>(&self, package: &'a str) -> &'a str {
        if self.eco != Ecosystem::Go {
            return package;
        }
        let mut parts = package.rsplit('/');
        let last = parts.next().unwrap_or(package);
        let is_major =
            last.len() > 1 && last.starts_with('v') && last[1..].bytes().all(|b| b.is_ascii_digit());
        if is_major {
            parts.next().unwrap_or(last)
        } else {
            last
        }
    }

    /// Full argument vector for `action`. Never goes through a shell, and
    /// `package` has already passed `tuiman_index::valid_package_name`.
    pub fn argv(&self, action: Action, package: &str) -> Vec<String> {
        debug_assert!(tuiman_index::valid_package_name(package));
        if self.eco == Ecosystem::Go && action == Action::Uninstall {
            let binary = go_bin_dir().join(self.installed_name(package));
            return vec!["rm".into(), "--".into(), binary.to_string_lossy().into_owned()];
        }
        let template = match action {
            Action::Install => self.install,
            Action::Uninstall => self.uninstall,
        };
        let mut argv = Vec::with_capacity(template.len() + 2);
        if self.sudo && !is_root() {
            argv.push("sudo".to_owned());
        }
        argv.push(self.bin.to_owned());
        argv.extend(template.iter().map(|arg| arg.replace("{pkg}", package)));
        argv
    }
}

/// Managers present on this machine: a `PATH` walk per binary, no
/// subprocesses. The walks run in parallel because `PATH` can contain slow
/// directories (network mounts, WSL's view of the Windows drive), and a
/// manager that is absent has to be looked for in every one of them.
pub fn detect() -> Vec<(ManagerId, PathBuf)> {
    thread::scope(|s| {
        let walks: Vec<_> = MANAGERS.iter().map(|m| s.spawn(|| paths::which(m.bin))).collect();
        let found = walks.into_iter().enumerate();
        found.filter_map(|(id, walk)| Some((id as ManagerId, walk.join().ok()??))).collect()
    })
}

fn is_root() -> bool {
    env::var("USER").is_ok_and(|u| u == "root")
}

fn run(bin: &Path, args: &[&str]) -> String {
    let output = Command::new(bin).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output();
    output.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn dir_entries(dir: &Path) -> Vec<String> {
    let entries = fs::read_dir(dir).into_iter().flatten().flatten();
    entries.filter_map(|e| e.file_name().into_string().ok()).filter(|name| !name.starts_with('.')).collect()
}

/// Installed formulae are the directories of `<prefix>/Cellar`; `brew` lives
/// in `<prefix>/bin`. Reading the directory is ~1000x faster than `brew list`.
fn list_brew(bin: &Path) -> Vec<String> {
    let prefix = env::var_os("HOMEBREW_PREFIX")
        .map(PathBuf::from)
        .or_else(|| Some(bin.parent()?.parent()?.to_owned()))
        .unwrap_or_default();
    dir_entries(&prefix.join("Cellar"))
}

fn list_cargo(_bin: &Path) -> Vec<String> {
    let cargo_home =
        env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| Some(paths::home()?.join(".cargo")));
    let manifest = cargo_home.map(|home| fs::read_to_string(home.join(".crates.toml")).unwrap_or_default());
    parse::cargo_crates_toml(&manifest.unwrap_or_default())
}

/// Global packages live in `<prefix>/lib/node_modules`; scoped ones sit one
/// level down in `@scope/`. The prefix is wherever `npm` itself is installed
/// unless the user moved it (commonly to avoid `sudo npm install -g`).
fn list_npm(bin: &Path) -> Vec<String> {
    let npmrc = paths::home().map(|home| fs::read_to_string(home.join(".npmrc")).unwrap_or_default());
    let prefix = env::var_os("NPM_CONFIG_PREFIX")
        .map(PathBuf::from)
        .or_else(|| parse::npmrc_prefix(&npmrc.unwrap_or_default(), paths::home().as_deref()))
        .or_else(|| Some(bin.parent()?.parent()?.to_owned()))
        .unwrap_or_default();
    let modules = prefix.join("lib/node_modules");
    let scoped = |name: String| match name.starts_with('@') {
        true => {
            dir_entries(&modules.join(&name)).into_iter().map(|package| format!("{name}/{package}")).collect()
        }
        false => vec![name],
    };
    dir_entries(&modules).into_iter().flat_map(scoped).collect()
}

fn data_home() -> Option<PathBuf> {
    let xdg = env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.or_else(|| Some(paths::home()?.join(".local/share")))
}

/// One directory per installed tool.
fn list_uv(_bin: &Path) -> Vec<String> {
    let tools = env::var_os("UV_TOOL_DIR").map(PathBuf::from).or_else(|| Some(data_home()?.join("uv/tools")));
    dir_entries(&tools.unwrap_or_default())
}

/// One virtualenv per installed application; pipx moved its home once.
fn list_pipx(_bin: &Path) -> Vec<String> {
    let homes = [
        env::var_os("PIPX_HOME").map(PathBuf::from),
        data_home().map(|data| data.join("pipx")),
        paths::home().map(|home| home.join(".local/pipx")),
    ];
    homes.into_iter().flatten().flat_map(|home| dir_entries(&home.join("venvs"))).collect()
}

fn go_bin_dir() -> PathBuf {
    let absolute = |var: &str| env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    absolute("GOBIN")
        .or_else(|| Some(env::split_paths(&env::var_os("GOPATH")?).next()?.join("bin")))
        .or_else(|| Some(paths::home()?.join("go/bin")))
        .unwrap_or_default()
}

fn list_go(_bin: &Path) -> Vec<String> {
    dir_entries(&go_bin_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(name: &str) -> &'static Manager {
        &MANAGERS[by_name(name).unwrap() as usize]
    }

    #[test]
    fn table_fits_the_mask_and_names_are_unique() {
        assert!(MANAGERS.len() <= Mask::BITS as usize);
        for (i, m) in MANAGERS.iter().enumerate() {
            assert_eq!(by_name(m.name), Some(i as ManagerId), "duplicate name {}", m.name);
        }
    }

    #[test]
    fn argv_substitutes_the_package_as_one_argument() {
        assert_eq!(manager("brew").argv(Action::Install, "btop"), ["brew", "install", "btop"]);
        assert_eq!(manager("cargo").argv(Action::Uninstall, "bottom"), ["cargo", "uninstall", "bottom"]);
        assert_eq!(
            manager("nix").argv(Action::Install, "btop"),
            ["nix-env", "--install", "--attr", "nixpkgs.btop"]
        );
        let go = manager("go").argv(Action::Install, "github.com/jesseduffield/lazygit");
        assert_eq!(go, ["go", "install", "github.com/jesseduffield/lazygit@latest"]);
    }

    #[test]
    fn go_uninstall_removes_the_binary() {
        let argv = manager("go").argv(Action::Uninstall, "github.com/derailed/k9s/v2");
        assert_eq!(&argv[..2], ["rm", "--"]);
        assert!(argv[2].ends_with("/bin/k9s"), "{argv:?}");
    }

    #[test]
    fn go_installed_names() {
        let go = manager("go");
        assert_eq!(go.installed_name("github.com/jesseduffield/lazygit"), "lazygit");
        assert_eq!(go.installed_name("github.com/derailed/k9s/v2"), "k9s");
        assert_eq!(go.installed_name("github.com/a/vim"), "vim");
        assert_eq!(manager("brew").installed_name("a/b"), "a/b");
    }

    #[test]
    fn privileged_managers_need_a_terminal() {
        assert!(MANAGERS.iter().filter(|m| m.sudo).all(|m| m.tty));
    }
}
