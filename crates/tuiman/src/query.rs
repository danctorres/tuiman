//! Filtering, searching and sorting: turns a [`Query`] into a view, which is
//! just a vector of row ids. Buffers are reused, so re-running a query does
//! not allocate once they have grown to catalog size.

use std::cmp::Reverse;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tuiman_index::{Catalog, Row};

use crate::installed::Installed;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Stars,
    Name,
    Updated,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Stars => Sort::Name,
            Sort::Name => Sort::Updated,
            Sort::Updated => Sort::Stars,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Stars => "stars",
            Sort::Name => "name",
            Sort::Updated => "updated",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub category: Option<u8>,
    pub language: Option<u16>,
    pub min_stars: u32,
    pub installed_only: bool,
    pub installable_only: bool,
    pub show_archived: bool,
    pub sort: Sort,
}

/// Result of running a query, plus the scratch space to run the next one.
pub struct View {
    pub rows: Vec<Row>,
    /// Matches per category under every filter except the category one, so
    /// the sidebar can show where the current search has hits.
    pub category_counts: Vec<u32>,
    scores: Vec<u32>,
    matcher: Matcher,
    utf32: Vec<char>,
}

impl Default for View {
    fn default() -> View {
        View {
            rows: Vec::new(),
            category_counts: Vec::new(),
            scores: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT),
            utf32: Vec::new(),
        }
    }
}

impl View {
    pub fn run(&mut self, catalog: &Catalog, installed: &Installed, query: &Query) {
        self.rows.clear();
        self.category_counts.clear();
        self.category_counts.resize(catalog.category_count(), 0);
        self.scores.clear();
        self.scores.resize(catalog.len(), 0);

        let text = query.text.trim();
        let pattern =
            (!text.is_empty()).then(|| Pattern::parse(text, CaseMatching::Ignore, Normalization::Smart));

        for row in catalog.rows() {
            let keep = (query.show_archived || !catalog.is_archived(row))
                && catalog.stars(row).unwrap_or(0) >= query.min_stars
                && query.language.is_none_or(|l| l == catalog.language_id(row))
                && (!query.installed_only || installed.is_installed(row))
                && (!query.installable_only || installed.is_available(row));
            if !keep {
                continue;
            }
            if let Some(pattern) = &pattern {
                // A hit in the name always outranks a hit in the description.
                let name =
                    pattern.score(Utf32Str::new(catalog.name(row), &mut self.utf32), &mut self.matcher);
                let score = match name {
                    Some(score) => score.saturating_add(1 << 20),
                    None => {
                        let desc = Utf32Str::new(catalog.desc(row), &mut self.utf32);
                        match pattern.score(desc, &mut self.matcher) {
                            Some(score) => score,
                            None => continue,
                        }
                    }
                };
                self.scores[row as usize] = score;
            }
            self.category_counts[catalog.category_id(row) as usize] += 1;
            if query.category.is_none_or(|c| c == catalog.category_id(row)) {
                self.rows.push(row);
            }
        }

        // While searching, name hits stay above description hits; within a
        // tier the chosen sort applies and the fuzzy score only breaks ties.
        let scores = &self.scores;
        let tier = |r: Row| Reverse(scores[r as usize] >> 20);
        let relevance = |r: Row| Reverse(scores[r as usize]);
        let stars = |r: Row| Reverse(catalog.stars(r).unwrap_or(0));
        let pushed = |r: Row| Reverse(catalog.pushed_days(r).unwrap_or(0));
        let name = |r: Row| catalog.name(r).bytes().map(|c| c.to_ascii_lowercase());
        self.rows.sort_unstable_by(|&a, &b| {
            let chosen = match query.sort {
                Sort::Stars => stars(a).cmp(&stars(b)),
                Sort::Name => name(a).cmp(name(b)),
                Sort::Updated => pushed(a).cmp(&pushed(b)),
            };
            tier(a).cmp(&tier(b)).then(chosen).then(relevance(a).cmp(&relevance(b))).then(a.cmp(&b))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::tests::{catalog, detected};
    use crate::managers::by_name;

    fn names(catalog: &Catalog, query: &Query, installed: &Installed) -> Vec<String> {
        let mut view = View::default();
        view.run(catalog, installed, query);
        view.rows.iter().map(|&r| catalog.name(r).to_owned()).collect()
    }

    #[test]
    fn default_query_sorts_by_stars_and_hides_archived() {
        let c = catalog();
        let inst = Installed::new(Vec::new(), &c);
        assert_eq!(names(&c, &Query::default(), &inst), ["lazygit", "btop", "bottom", "ratatui", "mystery"]);
        let all = Query { show_archived: true, sort: Sort::Name, ..Query::default() };
        assert_eq!(names(&c, &all, &inst), ["bottom", "btop", "lazygit", "mystery", "oldtool", "ratatui"]);
        let recent = Query { sort: Sort::Updated, ..Query::default() };
        assert_eq!(names(&c, &recent, &inst)[..2], ["ratatui", "lazygit"]);
    }

    #[test]
    fn filters_compose() {
        let c = catalog();
        let mut inst = Installed::new(detected(&["brew", "cargo"]), &c);
        inst.set_listing(by_name("brew").unwrap(), vec!["btop".into()], &c);

        let q = |f: fn(&mut Query)| {
            let mut q = Query::default();
            f(&mut q);
            q
        };
        assert_eq!(names(&c, &q(|q| q.min_stars = 10_000), &inst), ["lazygit", "btop", "bottom"]);
        assert_eq!(names(&c, &q(|q| q.installed_only = true), &inst), ["btop"]);
        assert_eq!(
            names(&c, &q(|q| q.installable_only = true), &inst),
            ["lazygit", "btop", "bottom", "ratatui"]
        );
        assert_eq!(names(&c, &q(|q| q.category = Some(0)), &inst), ["btop", "bottom"]);
        let rust = c.rows().find(|&r| c.language(r) == "Rust").map(|r| c.language_id(r));
        let mut rust_apps = Query { language: rust, min_stars: 9_500, ..Query::default() };
        assert_eq!(names(&c, &rust_apps, &inst), ["bottom"]);
        rust_apps.category = Some(1);
        assert!(names(&c, &rust_apps, &inst).is_empty());
    }

    #[test]
    fn search_ranks_name_hits_first_and_counts_per_category() {
        let c = catalog();
        let inst = Installed::new(Vec::new(), &c);
        let mut view = View::default();
        let query = Query { text: "bto".into(), category: Some(1), ..Query::default() };
        view.run(&c, &inst, &query);
        assert!(view.rows.is_empty(), "no development tool matches");
        assert_eq!(view.category_counts[0], 2, "btop and bottom both fuzzy-match in Dashboards");
        assert_eq!(view.category_counts.iter().sum::<u32>(), 2);

        let by_desc = Query { text: "lazygit desc".into(), ..Query::default() };
        assert_eq!(names(&c, &by_desc, &inst), ["lazygit"]);
        let fuzzy = Query { text: "BTM".into(), ..Query::default() };
        assert_eq!(names(&c, &fuzzy, &inst), ["bottom"]);
    }

    #[test]
    fn sort_still_applies_while_searching() {
        let c = catalog();
        let inst = Installed::new(Vec::new(), &c);
        let by_stars = Query { text: "b".into(), ..Query::default() };
        assert_eq!(names(&c, &by_stars, &inst), ["btop", "bottom"]);
        let by_name = Query { text: "b".into(), sort: Sort::Name, ..Query::default() };
        assert_eq!(names(&c, &by_name, &inst), ["bottom", "btop"]);
    }

    #[test]
    fn rerunning_reuses_buffers() {
        let c = catalog();
        let inst = Installed::new(Vec::new(), &c);
        let mut view = View::default();
        view.run(&c, &inst, &Query::default());
        let capacity = (view.rows.capacity(), view.scores.capacity());
        view.run(&c, &inst, &Query { text: "b".into(), ..Query::default() });
        assert_eq!((view.rows.capacity(), view.scores.capacity()), capacity);
    }
}
