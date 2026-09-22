//! Non-interactive commands over the same core as the TUI, for scripts and
//! for people who already know what they want.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;
use std::thread;

use tuiman_index::{Catalog, Row};

use crate::app::{IndexUpdate, Job};
use crate::installed::{self, Installed};
use crate::managers::{self, Action, MANAGERS};
use crate::query::{Query, Sort, View};
use crate::{fetch, worker};

const USAGE: &str = "\
tuiman: discover, install and manage TUIs

usage:
  tuiman                              open the interface
  tuiman list [FILTERS] [TEXT]        list TUIs, optionally fuzzy-matching TEXT
  tuiman install NAME [--via MANAGER] install a TUI
  tuiman uninstall NAME [--via MANAGER]
  tuiman upgrade NAME [--via MANAGER]   upgrade an installed TUI
  tuiman refresh                      download the latest index
  tuiman managers                     show the package managers tuiman found

filters:
  --installed            only TUIs that are installed
  --installable          only TUIs this machine has a package manager for
  --archived             include archived projects
  --min-stars N
  --category NAME
  --language NAME
  --sort stars|name|updated

environment:
  TUIMAN_INDEX_URL       https URL or local path of index.bin
  TUIMAN_CACHE_DIR       where the index and scan results are cached
  TUIMAN_NERD_FONT=1     Nerd Font icons and status bar separators
  TUIMAN_TRACE=1         print startup and frame timings on exit";

type CliResult = io::Result<ExitCode>;

fn fail(message: impl std::fmt::Display) -> CliResult {
    eprintln!("tuiman: {message}");
    Ok(ExitCode::from(2))
}

pub fn run(args: &[String]) -> CliResult {
    let rest = &args[1..];
    match args[0].as_str() {
        "list" | "ls" | "search" => list(rest),
        "install" | "i" => change(Action::Install, rest),
        "uninstall" | "remove" | "rm" => change(Action::Uninstall, rest),
        "upgrade" | "up" => change(Action::Upgrade, rest),
        "refresh" | "update" => refresh(),
        "managers" => {
            for (id, bin) in managers::detect() {
                println!("{:<8} {}", MANAGERS[id as usize].name, bin.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "-V" | "--version" => {
            println!("tuiman {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        other => fail(format!("unknown command {other:?}; try tuiman --help")),
    }
}

fn refresh() -> CliResult {
    match fetch::refresh(&crate::cache_dir()?) {
        IndexUpdate::Fresh(catalog) => println!("index updated: {} TUIs", catalog.len()),
        IndexUpdate::NotModified => println!("index is up to date"),
        IndexUpdate::Failed(why) => return fail(format!("index refresh failed: {why}")),
    }
    Ok(ExitCode::SUCCESS)
}

/// The cached catalog, downloading it first if this is the very first run.
fn catalog() -> io::Result<Catalog> {
    let cache_dir = crate::cache_dir()?;
    if let Some(catalog) = fetch::load_cached(&cache_dir) {
        return Ok(catalog);
    }
    match fetch::refresh(&cache_dir) {
        IndexUpdate::Fresh(catalog) => Ok(*catalog),
        IndexUpdate::Failed(why) => Err(io::Error::other(format!("cannot download the index: {why}"))),
        IndexUpdate::NotModified => Err(io::Error::other("index cache is missing")),
    }
}

/// Scans every detected manager, in parallel, and waits for all of them.
fn scan_installed(catalog: &Catalog) -> Installed {
    let mut installed = Installed::new(managers::detect(), catalog);
    let listings: Vec<_> = thread::scope(|s| {
        let scans: Vec<_> = installed
            .detected()
            .iter()
            .map(|(id, bin)| s.spawn(move || (*id, (MANAGERS[*id as usize].list_installed)(bin))))
            .collect();
        scans.into_iter().filter_map(|scan| scan.join().ok()).collect()
    });
    for (id, names) in listings {
        installed.set_listing(id, names, catalog);
    }
    installed
}

fn parse_query(args: &[String], catalog: &Catalog) -> Result<Query, String> {
    let mut query = Query::default();
    let mut words: Vec<&str> = Vec::new();
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg {
            "--installed" => query.installed_only = true,
            "--installable" => query.installable_only = true,
            "--archived" => query.show_archived = true,
            "--min-stars" => {
                query.min_stars = value()?.parse().map_err(|_| "--min-stars needs a number".to_owned())?;
            }
            "--category" => {
                let name = value()?;
                let id = (0..catalog.category_count() as u8)
                    .find(|&id| catalog.category_name(id).eq_ignore_ascii_case(name));
                query.category = Some(id.ok_or_else(|| format!("no category named {name:?}"))?);
            }
            "--language" => {
                let name = value()?;
                let id = (0..catalog.language_count() as u16)
                    .find(|&id| catalog.language_name(id).eq_ignore_ascii_case(name));
                query.language = Some(id.ok_or_else(|| format!("no TUI is written in {name:?}"))?);
            }
            "--sort" => {
                query.sort = match value()? {
                    "stars" => Sort::Stars,
                    "name" => Sort::Name,
                    "updated" => Sort::Updated,
                    other => return Err(format!("cannot sort by {other:?}")),
                };
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            word => words.push(word),
        }
    }
    query.text = words.join(" ");
    Ok(query)
}

fn list(args: &[String]) -> CliResult {
    let catalog = catalog()?;
    let query = match parse_query(args, &catalog) {
        Ok(query) => query,
        Err(why) => return fail(why),
    };
    let installed = scan_installed(&catalog);
    let mut view = View::default();
    view.run(&catalog, &installed, &query);

    // `tuiman list | head` closes the pipe early; that is not an error.
    let mut out = BufWriter::new(io::stdout().lock());
    for &row in &view.rows {
        let mark = if installed.is_installed(row) { '✓' } else { ' ' };
        let stars = catalog.stars(row).map_or_else(|| "-".to_owned(), |s| s.to_string());
        let line = writeln!(
            out,
            "{mark} {:<26} {stars:>7}  {:<12} {}",
            catalog.name(row),
            catalog.language(row),
            catalog.desc(row)
        );
        if line.is_err() {
            break;
        }
    }
    let _ = out.flush();
    Ok(ExitCode::SUCCESS)
}

/// Exact (case-insensitive) name match; otherwise the closest suggestions.
fn find(catalog: &Catalog, installed: &Installed, name: &str) -> Result<Row, Vec<String>> {
    if let Some(row) = catalog.rows().find(|&row| catalog.name(row).eq_ignore_ascii_case(name)) {
        return Ok(row);
    }
    let mut view = View::default();
    view.run(catalog, installed, &Query { text: name.to_owned(), show_archived: true, ..Query::default() });
    Err(view.rows.iter().take(5).map(|&row| catalog.name(row).to_owned()).collect())
}

fn change(action: Action, args: &[String]) -> CliResult {
    let (name, via) = match args {
        [name] => (name, None),
        [name, flag, manager] if flag == "--via" => (name, Some(manager.as_str())),
        _ => return fail(format!("usage: tuiman {} NAME [--via MANAGER]", action.verb())),
    };
    let catalog = catalog()?;
    let installed = scan_installed(&catalog);
    let row = match find(&catalog, &installed, name) {
        Ok(row) => row,
        Err(similar) if similar.is_empty() => return fail(format!("no TUI named {name:?}")),
        Err(similar) => return fail(format!("no TUI named {name:?}; did you mean {}?", similar.join(", "))),
    };

    // Without --via, an install through a second manager is never what was meant.
    if action == Action::Install && via.is_none() && installed.is_installed(row) {
        let managers: Vec<&str> = installed.installed_via(row).collect();
        return fail(format!(
            "{} is already installed via {}; pass --via MANAGER to install another copy",
            catalog.name(row),
            managers.join(", ")
        ));
    }
    let choices = installed.choices(&catalog, row, action);
    let choice = match via {
        None => choices.first(),
        Some(via) => choices.iter().find(|c| MANAGERS[c.manager as usize].name == via),
    };
    let Some(choice) = choice else {
        return fail(installed::impossible(&catalog, row, action, &format!("see {}", catalog.url(row))));
    };

    let manager = &MANAGERS[choice.manager as usize];
    let job = Job {
        action,
        manager: choice.manager,
        row,
        title: String::new(),
        argv: manager.argv(action, &choice.package),
    };
    eprintln!("$ {}", job.argv.join(" "));
    let status = worker::command(&job).status()?;
    Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::tests::catalog as sample;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn parses_filters_and_free_text() {
        let c = sample();
        let q = parse_query(
            &args(&["git", "--min-stars", "500", "--category", "development", "ui", "--sort", "name"]),
            &c,
        )
        .unwrap();
        assert_eq!((q.text.as_str(), q.min_stars, q.category, q.sort), ("git ui", 500, Some(1), Sort::Name));
        let q = parse_query(&args(&["--language", "rust", "--installed", "--archived"]), &c).unwrap();
        assert!(q.language.is_some() && q.installed_only && q.show_archived);
    }

    #[test]
    fn rejects_bad_options() {
        let c = sample();
        for bad in [
            &["--min-stars"][..],
            &["--min-stars", "many"],
            &["--category", "nope"],
            &["--sort", "size"],
            &["--wat"],
        ] {
            assert!(parse_query(&args(bad), &c).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn finds_by_name_or_suggests() {
        let c = sample();
        let inst = Installed::new(Vec::new(), &c);
        assert_eq!(find(&c, &inst, "LazyGit"), Ok(2));
        assert_eq!(find(&c, &inst, "lazy"), Err(vec!["lazygit".to_owned()]));
        assert_eq!(find(&c, &inst, "zzzzzz"), Err(Vec::new()));
    }
}
