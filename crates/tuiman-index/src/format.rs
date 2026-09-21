//! On-disk / on-wire index format. Little-endian, fixed column order:
//!
//! ```text
//! "TUIM" version:u16 reserved:u16 generated:u64
//! rows:u32 categories:u32 languages:u32 licenses:u32 packages:u32 arena:u32
//! name[rows]:Str desc[rows]:Str url[rows]:Str
//! stars[rows]:u32 pushed[rows]:u32 language[rows]:u16 license[rows]:u16
//! category[rows]:u8 flags[rows]:u8 pkg_start[rows + 1]:u32
//! categories[]:Str languages[]:Str licenses[]:Str
//! pkg_eco[packages]:u8 pkg_name[packages]:Str
//! arena[arena]:utf8
//! ```
//!
//! The file arrives over the network, so `decode` trusts nothing: the total
//! size is derived from the header and checked before anything is allocated,
//! and every range, id and string is validated once here so the accessors on
//! `Catalog` never have to.

use crate::{valid_package_name, valid_url, Catalog, Str};

const MAGIC: &[u8; 4] = b"TUIM";
const VERSION: u16 = 1;
const HEADER_BYTES: u64 = 4 + 2 + 2 + 8 + 6 * 4;
const STR_BYTES: u64 = 8;

/// Upper bound the client enforces on a downloaded index.
pub const MAX_INDEX_BYTES: usize = 16 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    UnsupportedVersion(u16),
    /// Length does not match what the header promises.
    BadLength,
    /// A range, id or string failed validation.
    Corrupt(&'static str),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadMagic => f.write_str("not a tuiman index"),
            DecodeError::UnsupportedVersion(v) => {
                write!(f, "index format version {v} is not supported, please update tuiman")
            }
            DecodeError::BadLength => f.write_str("index is truncated or has trailing data"),
            DecodeError::Corrupt(what) => write!(f, "index is corrupt: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn encode(c: &Catalog) -> Vec<u8> {
    let mut out = Vec::with_capacity(c.arena.len() + c.len() * 48 + 256);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&c.generated.to_le_bytes());
    for n in
        [c.len(), c.categories.len(), c.languages.len(), c.licenses.len(), c.pkg_eco.len(), c.arena.len()]
    {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }

    put_strs(&mut out, &c.name);
    put_strs(&mut out, &c.desc);
    put_strs(&mut out, &c.url);
    put_u32s(&mut out, &c.stars);
    put_u32s(&mut out, &c.pushed);
    put_u16s(&mut out, &c.language);
    put_u16s(&mut out, &c.license);
    out.extend_from_slice(&c.category);
    out.extend_from_slice(&c.flags);
    put_u32s(&mut out, &c.pkg_start);
    put_strs(&mut out, &c.categories);
    put_strs(&mut out, &c.languages);
    put_strs(&mut out, &c.licenses);
    out.extend_from_slice(&c.pkg_eco);
    put_strs(&mut out, &c.pkg_name);
    out.extend_from_slice(c.arena.as_bytes());
    out
}

fn put_u16s(out: &mut Vec<u8>, xs: &[u16]) {
    xs.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
}

fn put_u32s(out: &mut Vec<u8>, xs: &[u32]) {
    xs.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
}

fn put_strs(out: &mut Vec<u8>, xs: &[Str]) {
    for s in xs {
        out.extend_from_slice(&s.off.to_le_bytes());
        out.extend_from_slice(&s.len.to_le_bytes());
    }
}

pub fn decode(bytes: &[u8]) -> Result<Catalog, DecodeError> {
    if bytes.len() < HEADER_BYTES as usize {
        return Err(if bytes.starts_with(MAGIC) { DecodeError::BadLength } else { DecodeError::BadMagic });
    }
    let mut r = Reader { bytes, pos: 0 };
    if r.take(4) != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = r.u16();
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let _reserved = r.u16();
    let generated = r.u64();
    let [rows, n_cat, n_lang, n_lic, n_pkg, n_arena] = [(); 6].map(|()| u64::from(r.u32()));

    // All counts are < 2^32, so this cannot overflow u64. Checking it against
    // the real length bounds every allocation below by the input size.
    let expected = HEADER_BYTES
        + rows * (3 * STR_BYTES + 4 + 4 + 2 + 2 + 1 + 1)
        + (rows + 1) * 4
        + (n_cat + n_lang + n_lic) * STR_BYTES
        + n_pkg * (1 + STR_BYTES)
        + n_arena;
    if expected != bytes.len() as u64 {
        return Err(DecodeError::BadLength);
    }
    if n_cat > 256 || n_lang > 65536 || n_lic > 65536 {
        return Err(DecodeError::Corrupt("table too large"));
    }
    let [rows, n_cat, n_lang, n_lic, n_pkg, n_arena] =
        [rows, n_cat, n_lang, n_lic, n_pkg, n_arena].map(|n| n as usize);

    let c = Catalog {
        generated,
        name: r.strs(rows),
        desc: r.strs(rows),
        url: r.strs(rows),
        stars: r.u32s(rows),
        pushed: r.u32s(rows),
        language: r.u16s(rows),
        license: r.u16s(rows),
        category: r.take(rows).to_vec(),
        flags: r.take(rows).to_vec(),
        pkg_start: r.u32s(rows + 1),
        categories: r.strs(n_cat),
        languages: r.strs(n_lang),
        licenses: r.strs(n_lic),
        pkg_eco: r.take(n_pkg).to_vec(),
        pkg_name: r.strs(n_pkg),
        arena: match std::str::from_utf8(r.take(n_arena)) {
            Ok(s) => s.to_owned(),
            Err(_) => return Err(DecodeError::Corrupt("arena is not utf-8")),
        },
    };
    debug_assert_eq!(r.pos, bytes.len());
    validate(&c)?;
    Ok(c)
}

fn validate(c: &Catalog) -> Result<(), DecodeError> {
    if c.arena.chars().any(char::is_control) {
        return Err(DecodeError::Corrupt("control character in text"));
    }
    let columns = [&c.name, &c.desc, &c.url, &c.categories, &c.languages, &c.licenses, &c.pkg_name];
    for s in columns.into_iter().flatten() {
        let (start, end) = (s.off as usize, s.off as usize + s.len as usize);
        if end > c.arena.len() || !c.arena.is_char_boundary(start) || !c.arena.is_char_boundary(end) {
            return Err(DecodeError::Corrupt("string out of bounds"));
        }
    }
    if c.category.iter().any(|&id| id as usize >= c.categories.len()) {
        return Err(DecodeError::Corrupt("category id"));
    }
    if c.language.iter().any(|&id| id as usize >= c.languages.len()) {
        return Err(DecodeError::Corrupt("language id"));
    }
    if c.license.iter().any(|&id| id as usize >= c.licenses.len()) {
        return Err(DecodeError::Corrupt("license id"));
    }
    let monotonic = c.pkg_start.windows(2).all(|w| w[0] <= w[1]);
    if c.pkg_start[0] != 0 || !monotonic || c.pkg_start[c.len()] as usize != c.pkg_eco.len() {
        return Err(DecodeError::Corrupt("package ranges"));
    }
    if !c.url.iter().all(|&s| valid_url(c.str(s))) {
        return Err(DecodeError::Corrupt("url"));
    }
    if !c.pkg_name.iter().all(|&s| valid_package_name(c.str(s))) {
        return Err(DecodeError::Corrupt("package name"));
    }
    Ok(())
}

/// Cursor over a buffer whose length was already checked against the header,
/// so reads cannot run past the end.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        s
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }

    fn u16s(&mut self, n: usize) -> Vec<u16> {
        self.take(n * 2).chunks_exact(2).map(|b| u16::from_le_bytes(b.try_into().unwrap())).collect()
    }

    fn u32s(&mut self, n: usize) -> Vec<u32> {
        self.take(n * 4).chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect()
    }

    fn strs(&mut self, n: usize) -> Vec<Str> {
        self.take(n * 8)
            .chunks_exact(8)
            .map(|b| Str {
                off: u32::from_le_bytes(b[..4].try_into().unwrap()),
                len: u32::from_le_bytes(b[4..].try_into().unwrap()),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Builder, Ecosystem, Entry, FLAG_LIBRARY};

    fn sample() -> Catalog {
        let mut b = Builder::new(1_790_000_000);
        b.push(&Entry {
            name: "lazygit",
            desc: "simple terminal UI for git commands",
            url: "https://github.com/jesseduffield/lazygit",
            category: "Development",
            language: "Go",
            license: "MIT",
            stars: Some(60_000),
            pushed_days: Some(20_700),
            flags: 0,
            packages: &[(Ecosystem::Brew, "lazygit"), (Ecosystem::Go, "github.com/jesseduffield/lazygit")],
        })
        .unwrap();
        b.push(&Entry {
            name: "ratatui",
            desc: "Rust library to build rich TUIs — with ünïcödé",
            url: "https://github.com/ratatui/ratatui",
            category: "Libraries",
            language: "Rust",
            license: "MIT",
            flags: FLAG_LIBRARY,
            ..Entry::default()
        })
        .unwrap();
        b.finish()
    }

    #[test]
    fn roundtrip() {
        let c = sample();
        assert_eq!(decode(&encode(&c)).unwrap(), c);
        let empty = Builder::new(0).finish();
        assert_eq!(decode(&encode(&empty)).unwrap(), empty);
    }

    #[test]
    fn rejects_wrong_magic_and_version() {
        let mut bytes = encode(&sample());
        assert_eq!(decode(b"nope"), Err(DecodeError::BadMagic));
        assert_eq!(
            decode(b"<!DOCTYPE html><html>...........................</html>"),
            Err(DecodeError::BadMagic)
        );
        bytes[4] = 9;
        assert_eq!(decode(&bytes), Err(DecodeError::UnsupportedVersion(9)));
    }

    #[test]
    fn rejects_every_truncation_and_trailing_data() {
        let bytes = encode(&sample());
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len]).is_err(), "accepted truncation to {len}");
        }
        let mut longer = bytes;
        longer.push(0);
        assert_eq!(decode(&longer), Err(DecodeError::BadLength));
    }

    #[test]
    fn huge_counts_do_not_allocate() {
        let mut bytes = encode(&Builder::new(0).finish());
        bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(DecodeError::BadLength));
    }

    /// Flip every byte in turn: decode must never panic, and whatever it
    /// accepts must be safe to read through the accessors.
    #[test]
    fn single_byte_corruption_never_panics() {
        let bytes = encode(&sample());
        for i in 0..bytes.len() {
            for delta in [1u8, 0x80, 0xff] {
                let mut bad = bytes.clone();
                bad[i] = bad[i].wrapping_add(delta);
                if let Ok(c) = decode(&bad) {
                    for row in c.rows() {
                        let _ = (c.name(row), c.desc(row), c.url(row), c.language(row), c.license(row));
                        let _ = c.category_name(c.category_id(row));
                        assert!(c.packages(row).all(|(_, p)| valid_package_name(p)));
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_hostile_strings() {
        let c = sample();
        let bytes = encode(&c);
        let arena_at = bytes.len() - c.arena.len();

        // The brew package name sits right before the go module path.
        let pkg_at = arena_at + c.arena.find("lazygitgithub.com").unwrap();
        let mut bad = bytes.clone();
        bad[pkg_at] = b'-';
        assert!(decode(&bad).is_err(), "package name starting with '-'");

        let mut bad = bytes;
        bad[arena_at + 1] = 0x1b;
        assert_eq!(decode(&bad), Err(DecodeError::Corrupt("control character in text")));
    }
}
