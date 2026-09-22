//! What can be installed on this machine, and what already is: two manager
//! bitmasks per catalog row, derived from the detected managers' listings.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use tuiman_index::{Catalog, Row};

use crate::managers::{Action, ManagerId, Mask, MANAGERS};

#[derive(Default)]
pub struct Installed {
    detected: Vec<(ManagerId, PathBuf)>,
    /// Latest listing per manager; `None` until its first scan (or cache) lands.
    listings: [Option<HashSet<String>>; MANAGERS.len()],
    /// Per row: managers that could install it here.
    available: Vec<Mask>,
    /// Per row: managers that report it installed.
    installed: Vec<Mask>,
}

/// One way to carry out an action on a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub manager: ManagerId,
    pub package: String,
}

impl Installed {
    pub fn new(detected: Vec<(ManagerId, PathBuf)>, catalog: &Catalog) -> Installed {
        let mut this = Installed { detected, ..Installed::default() };
        this.rebuild(catalog);
        this
    }

    pub fn detected(&self) -> &[(ManagerId, PathBuf)] {
        &self.detected
    }

    pub fn is_available(&self, row: Row) -> bool {
        self.available[row as usize] != 0
    }

    pub fn is_installed(&self, row: Row) -> bool {
        self.installed[row as usize] != 0
    }

    pub fn set_listing(&mut self, manager: ManagerId, names: Vec<String>, catalog: &Catalog) {
        self.listings[manager as usize] = Some(names.into_iter().collect());
        self.rebuild(catalog);
    }

    /// Recomputes both masks. ~700 rows x a few packages: microseconds.
    pub fn rebuild(&mut self, catalog: &Catalog) {
        self.available.clear();
        self.installed.clear();
        for row in catalog.rows() {
            let (mut available, mut installed) = (0, 0);
            for (eco, package) in catalog.packages(row) {
                for &(id, _) in self.detected.iter().filter(|(id, _)| MANAGERS[*id as usize].eco == eco) {
                    available |= 1 << id;
                    let name = MANAGERS[id as usize].installed_name(package);
                    if self.listings[id as usize].as_ref().is_some_and(|set| set.contains(name)) {
                        installed |= 1 << id;
                    }
                }
            }
            self.available.push(available);
            self.installed.push(installed);
        }
    }

    /// Ways to perform `action` on `row`, in manager-table (preference) order.
    pub fn choices(&self, catalog: &Catalog, row: Row, action: Action) -> Vec<Choice> {
        let mask = match action {
            Action::Install => self.available[row as usize] & !self.installed[row as usize],
            Action::Uninstall | Action::Upgrade => self.installed[row as usize],
        };
        let mut choices: Vec<Choice> = catalog
            .packages(row)
            .flat_map(|(eco, package)| {
                (0..MANAGERS.len() as ManagerId)
                    .filter(move |&id| mask & (1 << id) != 0 && MANAGERS[id as usize].eco == eco)
                    .map(move |id| Choice { manager: id, package: package.to_owned() })
            })
            .collect();
        choices.sort_by_key(|c| c.manager);
        choices
    }

    /// Names of the managers that report `row` installed.
    pub fn installed_via(&self, row: Row) -> impl Iterator<Item = &'static str> + '_ {
        let mask = self.installed[row as usize];
        MANAGERS.iter().enumerate().filter(move |(id, _)| mask & (1 << id) != 0).map(|(_, m)| m.name)
    }

    /// Replaces the set of detected managers, forgetting listings of managers
    /// that disappeared.
    pub fn set_detected(&mut self, detected: Vec<(ManagerId, PathBuf)>, catalog: &Catalog) {
        for (id, listing) in self.listings.iter_mut().enumerate() {
            if !detected.iter().any(|(d, _)| *d as usize == id) {
                *listing = None;
            }
        }
        self.detected = detected;
        self.rebuild(catalog);
    }

    /// Persists what the next start needs to draw a truthful first frame
    /// without touching `PATH` or any package manager: the detected managers
    /// (`@name<TAB>path`) and the installed packages the catalog knows
    /// (`name<TAB>package`).
    pub fn save_cache(&self, path: &Path, catalog: &Catalog) {
        let mut text = String::new();
        for (id, bin) in &self.detected {
            let _ = writeln!(text, "@{}\t{}", MANAGERS[*id as usize].name, bin.display());
        }
        for row in catalog.rows().filter(|&row| self.is_installed(row)) {
            for choice in self.choices(catalog, row, Action::Uninstall) {
                let manager = &MANAGERS[choice.manager as usize];
                let _ = writeln!(text, "{}\t{}", manager.name, manager.installed_name(&choice.package));
            }
        }
        let _ = fs::write(path, text);
    }

    /// State as of the last run. It is only a starting point: the shell
    /// re-detects and re-scans in the background right after the first frame.
    pub fn from_cache(path: &Path, catalog: &Catalog) -> Installed {
        let mut this = Installed::default();
        let text = fs::read_to_string(path).unwrap_or_default();
        for (key, value) in text.lines().filter_map(|l| l.split_once('\t')) {
            if let Some(id) = key.strip_prefix('@').and_then(crate::managers::by_name) {
                this.detected.push((id, PathBuf::from(value)));
                this.listings[id as usize].get_or_insert_default();
            } else if let Some(id) = crate::managers::by_name(key) {
                this.listings[id as usize].get_or_insert_default().insert(value.to_owned());
            }
        }
        this.rebuild(catalog);
        this
    }
}

/// Why `action` has no [`Installed::choices`] on `row`. `page_hint` says how
/// to reach the project page from where the message is shown.
pub fn impossible(catalog: &Catalog, row: Row, action: Action, page_hint: &str) -> String {
    let name = catalog.name(row);
    if action != Action::Install {
        return format!("{name} is not installed through a package manager tuiman knows");
    }
    let known: Vec<&str> = catalog.packages(row).map(|(eco, _)| eco.name()).collect();
    match known.is_empty() {
        true => format!("no known package for {name} ({page_hint})"),
        false => format!("{name} is packaged for {}, none of which is on this machine", known.join(", ")),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::managers::by_name;
    use tuiman_index::{Builder, Ecosystem, Entry, FLAG_LIBRARY};

    pub fn catalog() -> Catalog {
        let mut b = Builder::new(0);
        let mut push =
            |name: &str, category: &str, language: &str, stars, pushed_days, flags, packages: &[_]| {
                let url = format!("https://github.com/o/{name}");
                let desc = format!("{name} description");
                b.push(&Entry {
                    name,
                    desc: &desc,
                    url: &url,
                    category,
                    language,
                    stars,
                    pushed_days,
                    flags,
                    packages,
                    ..Entry::default()
                })
                .unwrap();
            };
        push(
            "btop",
            "Dashboards",
            "C++",
            Some(20_000),
            Some(20_700),
            0,
            &[(Ecosystem::Brew, "btop"), (Ecosystem::Apt, "btop")],
        );
        push("bottom", "Dashboards", "Rust", Some(10_000), Some(20_600), 0, &[(Ecosystem::Crates, "bottom")]);
        push(
            "lazygit",
            "Development",
            "Go",
            Some(50_000),
            Some(20_710),
            0,
            &[(Ecosystem::Brew, "lazygit"), (Ecosystem::Go, "github.com/jesseduffield/lazygit")],
        );
        push("oldtool", "Development", "C", Some(300), Some(15_000), tuiman_index::FLAG_ARCHIVED, &[]);
        push(
            "ratatui",
            "Libraries",
            "Rust",
            Some(9_000),
            Some(20_715),
            FLAG_LIBRARY,
            &[(Ecosystem::Crates, "ratatui")],
        );
        push("mystery", "Web", "", None, None, 0, &[(Ecosystem::Nix, "mystery")]);
        b.finish()
    }

    pub fn detected(names: &[&str]) -> Vec<(ManagerId, PathBuf)> {
        names.iter().map(|n| (by_name(n).unwrap(), PathBuf::from(n))).collect()
    }

    #[test]
    fn availability_follows_detected_managers() {
        let c = catalog();
        let inst = Installed::new(detected(&["brew", "cargo"]), &c);
        let available: Vec<bool> = c.rows().map(|r| inst.is_available(r)).collect();
        assert_eq!(
            available,
            [true, true, true, false, true, false],
            "libraries install like anything else; nix-only rows need nix"
        );
        assert!(c.rows().all(|r| !inst.is_installed(r)));
    }

    #[test]
    fn listings_mark_rows_and_drive_choices() {
        let c = catalog();
        let mut inst = Installed::new(detected(&["brew", "go", "apt"]), &c);
        inst.set_listing(by_name("go").unwrap(), vec!["lazygit".into(), "unrelated".into()], &c);
        inst.set_listing(by_name("apt").unwrap(), vec!["btop".into()], &c);

        assert!(inst.is_installed(0) && inst.is_installed(2) && !inst.is_installed(1));
        assert_eq!(inst.installed_via(2).collect::<Vec<_>>(), ["go"]);

        let install = inst.choices(&c, 0, Action::Install);
        assert_eq!(
            install,
            [Choice { manager: by_name("brew").unwrap(), package: "btop".into() }],
            "apt already has it"
        );
        let uninstall = inst.choices(&c, 2, Action::Uninstall);
        assert_eq!(
            uninstall,
            [Choice { manager: by_name("go").unwrap(), package: "github.com/jesseduffield/lazygit".into() }]
        );
        assert!(inst.choices(&c, 1, Action::Uninstall).is_empty());
    }

    #[test]
    fn cache_roundtrip() {
        let c = catalog();
        let path = std::env::temp_dir().join(format!("tuiman-installed-test-{}", std::process::id()));
        let mut inst = Installed::new(detected(&["brew", "go"]), &c);
        inst.set_listing(by_name("brew").unwrap(), vec!["btop".into(), "wget".into()], &c);
        inst.set_listing(by_name("go").unwrap(), vec!["lazygit".into()], &c);
        inst.save_cache(&path, &c);

        let restored = Installed::from_cache(&path, &c);
        let _ = fs::remove_file(&path);
        assert_eq!(restored.detected, inst.detected);
        assert_eq!((&restored.installed, &restored.available), (&inst.installed, &inst.available));

        let nothing = Installed::from_cache(&path, &c);
        assert!(nothing.detected.is_empty() && c.rows().all(|r| !nothing.is_available(r)));
    }

    #[test]
    fn losing_a_manager_forgets_its_listing() {
        let c = catalog();
        let mut inst = Installed::new(detected(&["brew", "go"]), &c);
        inst.set_listing(by_name("go").unwrap(), vec!["lazygit".into()], &c);
        inst.set_detected(detected(&["brew"]), &c);
        assert!(!inst.is_installed(2) && inst.is_available(2));
        inst.set_detected(detected(&["brew", "go"]), &c);
        assert!(!inst.is_installed(2), "a stale listing must not come back");
    }
}
