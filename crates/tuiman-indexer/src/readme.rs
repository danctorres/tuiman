//! Parser for the awesome-tuis README.
//!
//! The list is regular enough that a line scanner beats a markdown crate:
//! `<h2>` inside a `<summary>` opens a category, `<h3>` opens a subcategory
//! (only used under *Libraries*), and `- [name](url) description` is an entry.

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub url: String,
    pub desc: String,
    pub category: String,
    pub subcategory: Option<String>,
}

pub fn parse(readme: &str) -> Vec<Listing> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut category: Option<String> = None;
    let mut subcategory: Option<String> = None;

    for line in readme.lines() {
        let line = line.trim();
        if let Some(title) = between(line, "<h2>", "</h2>") {
            category = Some(unescape(title));
            subcategory = None;
        } else if let Some(title) = between(line, "<h3>", "</h3>") {
            subcategory = Some(unescape(title));
        } else if let (Some(category), Some((name, url, desc))) = (&category, entry(line)) {
            // The same project is occasionally listed under two categories.
            if seen.insert(url.to_ascii_lowercase()) {
                out.push(Listing {
                    name: name.to_owned(),
                    url: url.to_owned(),
                    desc: plain_text(desc),
                    category: category.clone(),
                    subcategory: subcategory.clone(),
                });
            }
        }
    }
    out
}

fn between<'a>(line: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = line.find(open)? + open.len();
    let end = start + line[start..].find(close)?;
    Some(line[start..end].trim())
}

fn entry(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix("- [").or_else(|| line.strip_prefix("* ["))?;
    let (name, rest) = rest.split_once("](")?;
    let (url, desc) = rest.split_once(')')?;
    let url = url.trim().trim_end_matches('/');
    // The catalog refuses anything else, and one refused entry would fail the whole run.
    if name.is_empty() || !tuiman_index::valid_url(url) {
        return None;
    }
    let desc = desc.trim().trim_start_matches(['-', ':', '–', '—']).trim();
    Some((name.trim(), url, desc))
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

/// Descriptions are rendered as plain text in a terminal: inline links become
/// their label and emphasis markers are dropped.
fn plain_text(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(open) = rest.find('[') {
        let link = rest[open + 1..]
            .split_once("](")
            .and_then(|(label, tail)| Some((label, tail.split_once(')')?.1)))
            .filter(|(label, _)| !label.contains('['));
        match link {
            Some((label, tail)) => {
                out.push_str(&rest[..open]);
                out.push_str(label);
                rest = tail;
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    let text: String = out.chars().filter(|&c| c != '`' && c != '*' && !tuiman_index::is_emoji(c)).collect();
    unescape(text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/awesome-tuis.md");

    #[test]
    fn parses_the_real_list() {
        let list = parse(FIXTURE);
        assert!(list.len() > 600, "only {} entries", list.len());

        let categories: HashSet<&str> = list.iter().map(|l| l.category.as_str()).collect();
        assert_eq!(categories.len(), 13);
        assert!(categories.contains("Docker/LXC/K8s") && categories.contains("File Managers"));

        let btop = list.iter().find(|l| l.name == "btop++").unwrap();
        assert_eq!(btop.url, "https://github.com/aristocratos/btop");
        assert_eq!(btop.category, "Dashboards");
        assert_eq!(btop.desc, "Resource monitor with extras");
        assert_eq!(btop.subcategory, None);

        // Table-of-contents bullets are anchors, not entries.
        assert!(list.iter().all(|l| l.url.starts_with("http")));
        assert!(list.iter().all(|l| !l.name.is_empty() && !l.desc.contains('`')));
    }

    #[test]
    fn libraries_carry_their_language_heading() {
        let list = parse(FIXTURE);
        let libs: Vec<_> = list.iter().filter(|l| l.category == "Libraries").collect();
        assert!(libs.len() > 30);
        assert!(libs.iter().all(|l| l.subcategory.is_some()));
        assert!(list.iter().filter(|l| l.category != "Libraries").all(|l| l.subcategory.is_none()));
    }

    #[test]
    fn keeps_non_github_links() {
        let list = parse(FIXTURE);
        assert!(list.iter().any(|l| l.url.starts_with("https://codeberg.org/")));
    }

    #[test]
    fn entry_shapes() {
        let md = "<details open><summary><h2>Tools &amp; Toys</h2></summary>\n\
                  - [a](https://github.com/o/a/) - does [things](https://x.y) with `code`\n\
                  - [dup](https://GitHub.com/o/A) again\n\
                  - [anchor](#nope) skipped\n\
                  - [titled](https://github.com/o/t \"a title\") skipped, not a valid url\n\
                  - not an entry\n";
        let list = parse(md);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].category, "Tools & Toys");
        assert_eq!(list[0].url, "https://github.com/o/a");
        assert_eq!(list[0].desc, "does things with code");
    }

    #[test]
    fn entries_before_any_category_are_ignored() {
        assert!(parse("- [a](https://github.com/o/a) orphan").is_empty());
    }

    #[test]
    fn plain_text_survives_stray_brackets() {
        assert_eq!(plain_text("array [0] and [x](u) ok"), "array [0] and x ok");
        assert_eq!(plain_text("dangling [bracket"), "dangling [bracket");
        assert_eq!(
            plain_text("\u{2702}\u{fe0f} snips \u{1f680}, caf\u{e9} \u{4e2d}\u{6587}"),
            "snips , caf\u{e9} \u{4e2d}\u{6587}"
        );
    }
}
