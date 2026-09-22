# tuiman

<!--toc:start-->
- [tuiman](#tuiman)
  - [Install](#install)
    - [From source](#from-source)
  - [Use](#use)
  - [Catalog](#catalog)
  - [License](#license)
<!--toc:end-->

A fast TUI manager.

tuiman browses the ~700 terminal applications curated in
[awesome-tuis](https://github.com/rothgar/awesome-tuis), lets you search and
filter them (stars, category, language, installed, installable here), and
installs or uninstalls them through the package managers you already have.

## Install

```sh
cargo install --git https://github.com/danctorres/tuiman tuiman
```

Linux and macOS. Requires Rust 1.85+.

### From source

```sh
git clone https://github.com/danctorres/tuiman
cd tuiman
cargo build --release -p tuiman   # binary at target/release/tuiman
cargo install --path crates/tuiman  # or put it on your PATH
```

Before sending a change, run what CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

To rebuild the index locally (optional; the app downloads it), set
`GITHUB_TOKEN` and run:

```sh
cargo run --profile indexer -p tuiman-indexer -- --out dist
```

## Use

Run `tuiman`. Press `?` for every key; the important ones:

| Key | Action |
| --- | --- |
| `/` | fuzzy search names and descriptions |
| `h` `l` / `←` `→` | focus categories / list; `j` `k` / `↑` `↓` then move in that panel |
| `tab` `shift-tab` | next / previous category |
| `s` | cycle sort: stars, last push, name |
| `*` `L` | minimum stars, language |
| `i` `a` `A` | installed only, installable here only, show archived |
| `c` | clear filters |
| `enter` | install, or uninstall if installed (always shows the exact command first) |
| `o` | open the project page |
| `y` | copy the name |
| `v` | view job output |
| `t` | pick a colour theme (default, gruvbox, nord, catppuccin, tokyonight, dracula) |
| `?` | key help (`/` searches it) |
| `r` | refresh the index |

The same core is scriptable:

```sh
tuiman list git --min-stars 1000 --installable
tuiman list --installed
tuiman install lazygit            # picks the first available manager
tuiman install lazygit --via go
tuiman uninstall lazygit
tuiman managers                   # which package managers were found
```

## Catalog

The catalog comes from a small index file rebuilt daily from awesome-tuis,
GitHub and the package registries. tuiman checks for a new one at most every
six hours and otherwise starts from its cache, so it works offline and makes no
API calls while you browse. Press `r` to refresh now.

The cache lives in `~/.cache/tuiman` (`~/Library/Caches/tuiman` on macOS);
set `TUIMAN_CACHE_DIR` to move it.

A wrong or missing package for some app? Fix it in
[`overrides.toml`](overrides.toml) and open a pull request.

## License

MIT
