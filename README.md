# tuiman

A fast TUI manager with vim keybindings.

![tuiman searching, filtering and installing a TUI](https://github.com/user-attachments/assets/24266ed8-fa2a-483e-80b7-6db994d09dd7)

tuiman browses the ~700 terminal applications curated in
[awesome-tuis](https://github.com/rothgar/awesome-tuis), lets you search and
filter them (stars, category, language, installed, installable here), and
installs or uninstalls them through the package managers you already have.

## Install

Homebrew (macOS and Linux):

```sh
brew install danctorres/tap/tuiman
```

With Cargo:

```sh
cargo install tuiman
```

Or the latest from `main`:

```sh
cargo install --git https://github.com/danctorres/tuiman tuiman
```

Linux and macOS. Requires Rust 1.85+.

### Prebuilt binaries

Skip compiling with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall tuiman
```

Or download the file for your system from the
[releases page](https://github.com/danctorres/tuiman/releases) and put `tuiman` on your PATH:

```sh
tar xzf tuiman-*.tar.gz
mv tuiman-*/tuiman ~/.local/bin/
```

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
| `(` `)` | half a page up / down, in any list |
| `ctrl-d` `ctrl-u` | half a page down / up in the TUI list |
| `gg` `G` / `3gg` | top / bottom, or row 3 (rows are numbered) |
| `3j` | vim counts: a number before a move repeats it |
| `tab` `shift-tab` | next / previous category |
| `s` | cycle sort: stars, last push, name |
| `*` `L` | minimum stars, language |
| `i` `a` `A` | installed only, installable here only, show archived |
| `enter` | install, or uninstall if installed (always shows the exact command first) |
| `u` | upgrade an installed TUI to its latest version |
| `o` | open the project page |
| `y` | copy the selected item: name, category, theme, help line or job output |
| `v` | view job output |
| `t` | pick a colour theme, `/` searches (default, catppuccin, dracula, everforest, github, gruvbox, kanagawa, monokai, nord, rose-pine, solarized-dark, synthwave, tokyonight, bubblegum, catppuccin-latte, github-light, gruvbox-light, paper, sepia, solarized-light, amber, c64, cga, gameboy, phosphor, high-contrast) |
| `?` | key help (`/` searches it) |
| `r` | refresh the index |
| `c` | clear filters |

In the list, `✓` means a package manager installed it, `•` means its binary is
on your `PATH` but came from somewhere else (a downloaded release, a script, a
build from source), and `○` means it can be installed here. tuiman cannot
uninstall or upgrade a `•` TUI; `enter` on it installs a managed copy.

The same core is scriptable:

```sh
tuiman list git --min-stars 1000 --installable
tuiman list --installed
tuiman install lazygit            # picks the first available manager
tuiman install lazygit --via go
tuiman uninstall lazygit
tuiman upgrade lazygit
tuiman managers                   # which package managers were found
```

## Catalog

The catalog comes from a small index file rebuilt daily from awesome-tuis,
GitHub and the package registries. tuiman checks for a new one at most every
six hours and otherwise starts from its cache, so it works offline and makes no
API calls while you browse. Press `r` to refresh now.

If your terminal uses a Nerd Font, set `TUIMAN_NERD_FONT=1` for icons beside
each category and language and sharper separators in the bottom bar. Without
that font the icons show up as empty boxes, so it stays off by default.

The cache lives in `~/.cache/tuiman` (`~/Library/Caches/tuiman` on macOS);
set `TUIMAN_CACHE_DIR` to move it.

A wrong or missing package for some app? Fix it in
[`overrides.toml`](overrides.toml) and open a pull request.

## License

MIT
