//! The tuiman catalog: every TUI we know about, stored as struct-of-arrays
//! columns over a single string arena.
//!
//! This crate is shared by the client (which only ever decodes) and the CI
//! indexer (which only ever builds and encodes). It has no dependencies.

pub mod date;
mod format;

pub use format::{decode, encode, DecodeError, MAX_INDEX_BYTES};

/// Row id into a [`Catalog`].
pub type Row = u32;

/// Entry is flagged as archived upstream.
pub const FLAG_ARCHIVED: u8 = 1 << 0;
/// Entry is a TUI *library*, not an application: browsable, never installable.
pub const FLAG_LIBRARY: u8 = 1 << 1;

const STARS_UNKNOWN: u32 = u32::MAX;

/// Where a package lives. This is index-side vocabulary: the client maps each
/// ecosystem onto whichever package managers on the machine can consume it
/// (e.g. `Pypi` is served by both `pipx` and `uv`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Ecosystem {
    Brew = 0,
    Crates = 1,
    Go = 2,
    Npm = 3,
    Pypi = 4,
    Apt = 5,
    Dnf = 6,
    Pacman = 7,
    Aur = 8,
    Nix = 9,
}

impl Ecosystem {
    pub const ALL: [Ecosystem; 10] = [
        Ecosystem::Brew,
        Ecosystem::Crates,
        Ecosystem::Go,
        Ecosystem::Npm,
        Ecosystem::Pypi,
        Ecosystem::Apt,
        Ecosystem::Dnf,
        Ecosystem::Pacman,
        Ecosystem::Aur,
        Ecosystem::Nix,
    ];

    /// Unknown ids are not an error: an older client must keep working when
    /// the indexer learns a new ecosystem.
    pub fn from_u8(id: u8) -> Option<Ecosystem> {
        Self::ALL.get(id as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Ecosystem::Brew => "brew",
            Ecosystem::Crates => "crates",
            Ecosystem::Go => "go",
            Ecosystem::Npm => "npm",
            Ecosystem::Pypi => "pypi",
            Ecosystem::Apt => "apt",
            Ecosystem::Dnf => "dnf",
            Ecosystem::Pacman => "pacman",
            Ecosystem::Aur => "aur",
            Ecosystem::Nix => "nix",
        }
    }

    pub fn from_name(name: &str) -> Option<Ecosystem> {
        Self::ALL.iter().copied().find(|e| e.name() == name)
    }
}

/// A string slice inside the catalog arena.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Str {
    off: u32,
    len: u32,
}

/// Package names end up as arguments to package managers, so the alphabet is
/// deliberately tiny and a leading `-` (option injection) is refused.
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+/-".contains(&b))
}

/// Emoji, dingbats, variation selectors and joiners. Terminals disagree about
/// how wide these are, which corrupts any cell-based renderer, so they are
/// kept out of everything tuiman draws (catalog text and job output alike).
pub fn is_emoji(c: char) -> bool {
    matches!(c,
        '\u{200d}' | '\u{20e3}' | '\u{fe00}'..='\u{fe0f}'
        | '\u{2190}'..='\u{21ff}' | '\u{2300}'..='\u{23ff}' | '\u{2600}'..='\u{27bf}'
        | '\u{2b00}'..='\u{2bff}' | '\u{1f000}'..='\u{1faff}' | '\u{e0020}'..='\u{e007f}')
}

/// URLs are handed to the system browser opener, so only plain web URLs pass.
pub fn valid_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 500
        && !url.bytes().any(|b| b.is_ascii_whitespace())
}

/// Read-only catalog. Construct with [`Builder`] or [`decode`]; both uphold
/// the invariants the accessors rely on (ranges in bounds, ids in tables).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// Unix seconds at which the indexer produced this catalog.
    generated: u64,

    name: Vec<Str>,
    desc: Vec<Str>,
    url: Vec<Str>,
    stars: Vec<u32>,
    /// Last push, in days since the Unix epoch. 0 = unknown.
    pushed: Vec<u32>,
    language: Vec<u16>,
    license: Vec<u16>,
    category: Vec<u8>,
    flags: Vec<u8>,
    /// `pkg_start[row]..pkg_start[row + 1]` indexes the package columns.
    pkg_start: Vec<u32>,

    categories: Vec<Str>,
    languages: Vec<Str>,
    licenses: Vec<Str>,

    pkg_eco: Vec<u8>,
    pkg_name: Vec<Str>,

    arena: String,
}

impl Catalog {
    /// A valid catalog with no rows.
    pub fn empty() -> Catalog {
        Builder::new(0).finish()
    }

    pub fn len(&self) -> usize {
        self.name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }

    pub fn rows(&self) -> std::ops::Range<Row> {
        0..self.len() as Row
    }

    fn str(&self, s: Str) -> &str {
        &self.arena[s.off as usize..(s.off + s.len) as usize]
    }

    pub fn name(&self, row: Row) -> &str {
        self.str(self.name[row as usize])
    }

    pub fn desc(&self, row: Row) -> &str {
        self.str(self.desc[row as usize])
    }

    pub fn url(&self, row: Row) -> &str {
        self.str(self.url[row as usize])
    }

    pub fn stars(&self, row: Row) -> Option<u32> {
        let s = self.stars[row as usize];
        (s != STARS_UNKNOWN).then_some(s)
    }

    /// Days since the Unix epoch of the last push, if known.
    pub fn pushed_days(&self, row: Row) -> Option<u32> {
        let d = self.pushed[row as usize];
        (d != 0).then_some(d)
    }

    pub fn category_id(&self, row: Row) -> u8 {
        self.category[row as usize]
    }

    pub fn language_id(&self, row: Row) -> u16 {
        self.language[row as usize]
    }

    pub fn language(&self, row: Row) -> &str {
        self.str(self.languages[self.language[row as usize] as usize])
    }

    pub fn license(&self, row: Row) -> &str {
        self.str(self.licenses[self.license[row as usize] as usize])
    }

    pub fn flags(&self, row: Row) -> u8 {
        self.flags[row as usize]
    }

    pub fn is_archived(&self, row: Row) -> bool {
        self.flags(row) & FLAG_ARCHIVED != 0
    }

    pub fn is_library(&self, row: Row) -> bool {
        self.flags(row) & FLAG_LIBRARY != 0
    }

    pub fn category_count(&self) -> usize {
        self.categories.len()
    }

    pub fn category_name(&self, id: u8) -> &str {
        self.str(self.categories[id as usize])
    }

    pub fn language_count(&self) -> usize {
        self.languages.len()
    }

    pub fn language_name(&self, id: u16) -> &str {
        self.str(self.languages[id as usize])
    }

    /// Packages that provide `row`, skipping ecosystems this build predates.
    pub fn packages(&self, row: Row) -> impl Iterator<Item = (Ecosystem, &str)> + '_ {
        let range = self.pkg_start[row as usize] as usize..self.pkg_start[row as usize + 1] as usize;
        range.filter_map(|i| Some((Ecosystem::from_u8(self.pkg_eco[i])?, self.str(self.pkg_name[i]))))
    }
}

/// One catalog entry as the indexer sees it.
#[derive(Clone, Debug, Default)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub desc: &'a str,
    pub url: &'a str,
    pub category: &'a str,
    pub language: &'a str,
    pub license: &'a str,
    pub stars: Option<u32>,
    pub pushed_days: Option<u32>,
    pub flags: u8,
    pub packages: &'a [(Ecosystem, &'a str)],
}

#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    InvalidUrl(String),
    InvalidPackage(String),
    TooManyCategories,
    TooManyTableEntries,
    ArenaFull,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::InvalidUrl(u) => write!(f, "invalid url {u:?}"),
            BuildError::InvalidPackage(p) => write!(f, "invalid package name {p:?}"),
            BuildError::TooManyCategories => f.write_str("more than 256 categories"),
            BuildError::TooManyTableEntries => f.write_str("more than 65536 table entries"),
            BuildError::ArenaFull => f.write_str("string arena exceeds 4 GiB"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Append-only catalog builder.
#[derive(Debug)]
pub struct Builder {
    cat: Catalog,
}

impl Builder {
    pub fn new(generated: u64) -> Builder {
        Builder { cat: Catalog { generated, pkg_start: vec![0], ..Catalog::default() } }
    }

    pub fn push(&mut self, e: &Entry<'_>) -> Result<(), BuildError> {
        if !valid_url(e.url) {
            return Err(BuildError::InvalidUrl(e.url.to_owned()));
        }
        if let Some((_, bad)) = e.packages.iter().find(|(_, p)| !valid_package_name(p)) {
            return Err(BuildError::InvalidPackage((*bad).to_owned()));
        }

        let category = intern(&mut self.cat.arena, &mut self.cat.categories, e.category)?;
        let category = u8::try_from(category).map_err(|_| BuildError::TooManyCategories)?;
        let language = intern(&mut self.cat.arena, &mut self.cat.languages, e.language)?;
        let license = intern(&mut self.cat.arena, &mut self.cat.licenses, e.license)?;

        let c = &mut self.cat;
        c.name.push(alloc(&mut c.arena, e.name)?);
        c.desc.push(alloc(&mut c.arena, e.desc)?);
        c.url.push(alloc(&mut c.arena, e.url)?);
        c.stars.push(e.stars.map_or(STARS_UNKNOWN, |s| s.min(STARS_UNKNOWN - 1)));
        c.pushed.push(e.pushed_days.unwrap_or(0));
        c.language.push(language);
        c.license.push(license);
        c.category.push(category);
        c.flags.push(e.flags);
        for (eco, pkg) in e.packages {
            c.pkg_eco.push(*eco as u8);
            c.pkg_name.push(alloc(&mut c.arena, pkg)?);
        }
        c.pkg_start.push(c.pkg_eco.len() as u32);
        Ok(())
    }

    pub fn finish(self) -> Catalog {
        self.cat
    }
}

/// Copies `s` into the arena. Control characters never enter a catalog: the
/// text is drawn straight to a terminal.
fn alloc(arena: &mut String, s: &str) -> Result<Str, BuildError> {
    let off = arena.len();
    arena.extend(s.chars().map(|c| if c.is_control() { ' ' } else { c }));
    let end = u32::try_from(arena.len()).map_err(|_| BuildError::ArenaFull)?;
    Ok(Str { off: off as u32, len: end - off as u32 })
}

fn intern(arena: &mut String, table: &mut Vec<Str>, s: &str) -> Result<u16, BuildError> {
    let found = table.iter().position(|&t| &arena[t.off as usize..(t.off + t.len) as usize] == s);
    let id = match found {
        Some(id) => id,
        None => {
            table.push(alloc(arena, s)?);
            table.len() - 1
        }
    };
    u16::try_from(id).map_err(|_| BuildError::TooManyTableEntries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        for ok in ["btop", "lazygit", "@scope/pkg", "github.com/a/b/cmd/x", "python3Packages.foo", "g++"] {
            assert!(valid_package_name(ok), "{ok}");
        }
        for bad in ["", "-rf", "--force", "a;b", "a b", "a$(b)", "a\nb", "a`b`", "ä"] {
            assert!(!valid_package_name(bad), "{bad}");
        }
    }

    #[test]
    fn urls() {
        assert!(valid_url("https://github.com/a/b"));
        assert!(!valid_url("-h"));
        assert!(!valid_url("file:///etc/passwd"));
        assert!(!valid_url("https://a b"));
    }

    #[test]
    fn builder_rejects_bad_input() {
        let mut b = Builder::new(0);
        let bad_pkg = Entry { url: "https://x.y", packages: &[(Ecosystem::Brew, "-x")], ..Entry::default() };
        assert_eq!(b.push(&bad_pkg), Err(BuildError::InvalidPackage("-x".into())));
        let bad_url = Entry { url: "javascript:1", ..Entry::default() };
        assert!(matches!(b.push(&bad_url), Err(BuildError::InvalidUrl(_))));
        assert_eq!(b.finish().len(), 0);
    }

    #[test]
    fn builder_roundtrip_through_accessors() {
        let mut b = Builder::new(42);
        b.push(&Entry {
            name: "btop",
            desc: "Resource\u{1b}[31m monitor",
            url: "https://github.com/aristocratos/btop",
            category: "Dashboards",
            language: "C++",
            license: "Apache-2.0",
            stars: Some(20_000),
            pushed_days: Some(20_000),
            flags: FLAG_ARCHIVED,
            packages: &[(Ecosystem::Brew, "btop"), (Ecosystem::Apt, "btop")],
        })
        .unwrap();
        b.push(&Entry { name: "x", url: "https://x.y", category: "Dashboards", ..Entry::default() }).unwrap();
        let c = b.finish();

        assert_eq!(c.len(), 2);
        assert_eq!(c.name(0), "btop");
        assert_eq!(c.desc(0), "Resource [31m monitor", "control characters are neutralised");
        assert_eq!(c.stars(0), Some(20_000));
        assert_eq!(c.stars(1), None);
        assert_eq!(c.pushed_days(1), None);
        assert_eq!(c.language(0), "C++");
        assert_eq!(c.license(0), "Apache-2.0");
        assert!(c.is_archived(0) && !c.is_library(0));
        assert_eq!(c.category_id(0), c.category_id(1));
        assert_eq!(c.category_count(), 1);
        assert_eq!(c.packages(0).collect::<Vec<_>>(), [(Ecosystem::Brew, "btop"), (Ecosystem::Apt, "btop")]);
        assert_eq!(c.packages(1).count(), 0);
    }
}
