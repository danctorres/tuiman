//! Builds the tuiman index: awesome-tuis README → GitHub metadata → package
//! names → `index.bin` (client format) + `index.json` (for humans and tools).
//!
//! Runs in CI on a schedule; see `.github/workflows/index.yml`.

mod github;
mod http;
mod model;
mod overrides;
mod readme;
mod resolve;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process, thread};

use tuiman_index::{Builder, Ecosystem, Entry, FLAG_ARCHIVED, FLAG_LIBRARY};

use crate::http::Http;
use crate::model::Item;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const README_URL: &str = "https://raw.githubusercontent.com/rothgar/awesome-tuis/master/README.md";
/// A parse that yields fewer entries than this means the README layout
/// changed; publishing such an index would wipe the catalog for every user.
const MIN_ENTRIES: usize = 400;

const USAGE: &str = "\
usage: tuiman-indexer [--out DIR] [--readme FILE] [--overrides FILE] [--no-registries]

  --out DIR         output directory (default: dist)
  --readme FILE     read the awesome-tuis README from FILE instead of the network
  --overrides FILE  hand-curated corrections (default: overrides.toml)
  --no-registries   skip the rate-limited per-package lookups
                    (crates.io, npm, PyPI, Repology)

GITHUB_TOKEN must be set for stars, languages and registry candidates.";

struct Args {
    out: PathBuf,
    readme: Option<PathBuf>,
    overrides: PathBuf,
    registries: bool,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut args = Args {
        out: PathBuf::from("dist"),
        readme: None,
        overrides: PathBuf::from("overrides.toml"),
        registries: true,
    };
    let mut argv = env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--out" => args.out = argv.next().ok_or("--out needs a value")?.into(),
            "--readme" => args.readme = Some(argv.next().ok_or("--readme needs a value")?.into()),
            "--overrides" => args.overrides = argv.next().ok_or("--overrides needs a value")?.into(),
            "--no-registries" => args.registries = false,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(args)
}

fn main() {
    let args = parse_args().unwrap_or_else(|msg| {
        eprintln!("{msg}\n{USAGE}");
        process::exit(if msg.is_empty() { 0 } else { 2 });
    });
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(args: &Args) -> Result<()> {
    let http = Http::new();
    let overrides = overrides::Overrides::load(&args.overrides)?;

    let readme = match &args.readme {
        Some(path) => fs::read_to_string(path)?,
        None => http.get_text(README_URL)?,
    };
    let mut items: Vec<Item> = readme::parse(&readme).into_iter().map(Item::from_listing).collect();
    eprintln!("readme: {} entries", items.len());
    if items.len() < MIN_ENTRIES {
        return Err(format!("only {} entries parsed; refusing to publish", items.len()).into());
    }

    match env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()) {
        Some(token) => {
            let dead = github::enrich(&http, &token, &mut items)?;
            for &i in dead.iter().rev() {
                eprintln!("github: dropping {} ({}): repository not found", items[i].name, items[i].url);
                items.remove(i);
            }
        }
        None => eprintln!("warn: GITHUB_TOKEN not set: no stars, languages or registry packages"),
    }

    // Resolvers only read `items`; their findings are merged afterwards.
    // One thread per data source, since each is bound by its own host.
    let found = thread::scope(|s| {
        let bulk = [
            ("brew", s.spawn(|| resolve::brew::resolve(&http, &items))),
            ("aur", s.spawn(|| resolve::bulk::resolve_aur(&http, &items))),
            ("nix", s.spawn(|| resolve::bulk::resolve_nix(&http, &items))),
        ];
        let registries = args.registries.then(|| s.spawn(|| resolve::registries::resolve(&http, &items)));
        let mut found = Vec::new();
        for (name, handle) in bulk {
            match handle.join().expect("resolver panicked") {
                Ok(matches) => found.extend(matches),
                // One source being down must not stop the daily index.
                Err(e) => eprintln!("warn: {name} resolver failed: {e}"),
            }
        }
        found.extend(registries.into_iter().flat_map(|h| h.join().expect("resolver panicked")));
        found
    });
    for (i, eco, package) in found {
        items[i].set_package(eco, &package);
    }

    // Repology lookups are anchored on the URL-verified matches above.
    if args.registries {
        for (i, eco, package) in resolve::repology::resolve(&http, &items) {
            items[i].set_package(eco, &package);
        }
    }
    overrides.apply(&mut items);

    report(&items);
    write(args, &items)
}

fn report(items: &[Item]) {
    let apps = items.iter().filter(|i| !i.library).count();
    let installable = items.iter().filter(|i| !i.packages.is_empty()).count();
    eprintln!("index: {} entries, {apps} applications, {installable} installable", items.len());
    let mut per_eco: BTreeMap<&str, usize> = BTreeMap::new();
    for eco in items.iter().flat_map(|i| i.packages.keys()) {
        *per_eco.entry(eco).or_default() += 1;
    }
    for (eco, n) in per_eco {
        eprintln!("  {eco:<8} {n}");
    }
}

fn write(args: &Args, items: &[Item]) -> Result<()> {
    let generated = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut builder = Builder::new(generated);
    for item in items {
        let packages: Vec<(Ecosystem, &str)> = item
            .packages
            .iter()
            .filter_map(|(eco, name)| Some((Ecosystem::from_name(eco)?, name.as_str())))
            .collect();
        let mut flags = 0;
        if item.archived {
            flags |= FLAG_ARCHIVED;
        }
        if item.library {
            flags |= FLAG_LIBRARY;
        }
        builder.push(&Entry {
            name: &item.name,
            desc: &item.description,
            url: &item.url,
            category: &item.category,
            language: &item.language,
            license: &item.license,
            stars: item.stars,
            pushed_days: item.pushed_at.as_deref().and_then(tuiman_index::date::days_from_iso),
            flags,
            packages: &packages,
        })?;
    }
    let bytes = tuiman_index::encode(&builder.finish());
    // Never publish something the client would refuse.
    tuiman_index::decode(&bytes)?;

    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("index.bin"), &bytes)?;
    let json = serde_json::json!({ "generated": generated, "source": README_URL, "entries": items });
    fs::write(args.out.join("index.json"), serde_json::to_vec_pretty(&json)?)?;
    eprintln!("wrote {} ({} bytes) and index.json", args.out.join("index.bin").display(), bytes.len());
    Ok(())
}
