//! tuiman: a TUI to discover, install and manage TUIs.
//!
//! This file is the imperative shell around the pure core in `app.rs`: it
//! owns the terminal, the event channel and the worker threads, feeds events
//! into `App::update`, and carries out the effects that come back.
//!
//! The loop blocks on the channel and draws only when state changed, so an
//! idle tuiman does no work at all.

mod app;
mod cli;
mod fetch;
mod installed;
mod managers;
mod paths;
mod query;
mod term;
mod theme;
mod trace;
mod ui;
mod worker;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tuiman_index::Catalog;

use crate::app::{App, Effect, Event, Job};
use crate::installed::Installed;
use crate::trace::Trace;

/// Spinner frame time; the loop only wakes on it while something is running.
const TICK: Duration = Duration::from_millis(80);
const INSTALLED_CACHE: &str = "installed";
const THEME_FILE: &str = "theme";

fn main() -> ExitCode {
    let trace = Trace::start();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.is_empty() { tui(trace) } else { cli::run(&args) };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tuiman: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn today_days() -> u32 {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    (secs / 86_400) as u32
}

pub fn cache_dir() -> io::Result<PathBuf> {
    let dir = paths::cache_dir()
        .ok_or_else(|| io::Error::other("cannot locate a cache directory: HOME is not set"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn tui(mut trace: Trace) -> io::Result<ExitCode> {
    let cache_dir = cache_dir()?;
    let catalog = fetch::load_cached(&cache_dir).unwrap_or_else(Catalog::empty);
    trace.mark("index decoded");
    let installed = Installed::from_cache(&cache_dir.join(INSTALLED_CACHE), &catalog);
    trace.mark("installed cache read");
    let mut app = App::new(catalog, installed, today_days());
    app.theme = theme::by_name(&std::fs::read_to_string(cache_dir.join(THEME_FILE)).unwrap_or_default());
    trace.mark("first query");

    term::install_panic_hook();
    let mut terminal = term::enter()?;
    // Also restores the terminal when an error returns early through `?`.
    let restore = term::Restore;
    let (tx, rx) = mpsc::channel();

    // Nothing slow has happened yet: everything below the first frame runs on
    // worker threads and patches the UI when it lands.
    let mut pending = VecDeque::new();
    app.detecting = true;
    worker::detect(tx.clone());
    if app.catalog.is_empty() || !fetch::is_fresh(&cache_dir) {
        app.begin_quiet_refresh();
        pending.push_back(Effect::RefreshIndex);
    }

    let input = term::Input::spawn(tx.clone());
    let mut first_frame = true;
    'main: loop {
        if app.dirty {
            let started = Instant::now();
            let (page, sidebar) = ui::layout(terminal.size()?.into());
            app.set_layout(page, sidebar);
            terminal.draw(|frame| ui::draw(frame, &app))?;
            app.dirty = false;
            trace.frame(started.elapsed());
            if std::mem::take(&mut first_frame) {
                trace.mark("first frame");
            }
        }

        while let Some(effect) = pending.pop_front() {
            match effect {
                Effect::Quit => break 'main,
                Effect::RefreshIndex => worker::refresh_index(tx.clone(), cache_dir.clone()),
                Effect::Scan(only) => scan(&tx, &app, only),
                Effect::Spawn(job) => worker::spawn_job(tx.clone(), &job),
                Effect::SaveInstalled => {
                    app.installed.save_cache(&cache_dir.join(INSTALLED_CACHE), &app.catalog)
                }
                Effect::SaveTheme(name) => {
                    let _ = std::fs::write(cache_dir.join(THEME_FILE), name);
                }
                Effect::OpenUrl(url) => {
                    app.status = match open_url(&url) {
                        true => format!("Opened {url}"),
                        false => format!("No browser opener found: {url}"),
                    };
                    app.dirty = true;
                }
                Effect::Copy(text) => {
                    app.status = match copy(&text) {
                        true => format!("Copied {text}"),
                        false => "No clipboard tool found (pbcopy, clip.exe, wl-copy, xclip, xsel)".into(),
                    };
                    app.dirty = true;
                }
                Effect::RunInTerminal(job) => {
                    let guard = input.pause();
                    let ok = run_in_terminal(&job);
                    terminal = term::enter()?;
                    input.resume(guard);
                    pending.extend(app.terminal_job_finished(&job, ok));
                }
            }
        }
        if app.dirty {
            continue;
        }

        let event = if app.busy() {
            match rx.recv_timeout(TICK) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => Event::Tick,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(event) => event,
                Err(_) => break,
            }
        };
        pending.extend(app.update(event));
        // Drain whatever else is queued (key repeat, bursts of job output) so
        // a backlog costs one frame, not one frame per event.
        while let Ok(event) = rx.try_recv() {
            pending.extend(app.update(event));
        }
    }

    drop(restore);
    trace.report();
    Ok(ExitCode::SUCCESS)
}

fn scan(tx: &Sender<Event>, app: &App, only: Option<managers::ManagerId>) {
    for (id, bin) in app.installed.detected().iter().filter(|(id, _)| only.is_none_or(|o| o == *id)) {
        worker::scan(tx.clone(), *id, bin.clone());
    }
}

/// Steps out of the TUI so the command owns the terminal (sudo password
/// prompts, pacman confirmations), then waits for the user before returning.
fn run_in_terminal(job: &Job) -> bool {
    term::leave();
    println!("\n$ {}\n", job.argv.join(" "));
    let ok = match worker::command(job).status() {
        Ok(status) => status.success(),
        Err(e) => {
            println!("cannot start {}: {e}", job.argv[0]);
            false
        }
    };
    print!("\n[{}] press enter to return to tuiman ", if ok { "done" } else { "failed" });
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut String::new());
    ok
}

/// `url` was validated as a plain http(s) URL when the index was decoded.
fn open_url(url: &str) -> bool {
    // Only spawning is checked: explorer.exe exits 1 on success and xdg-open
    // may block until the browser closes. So under WSL, where xdg-open often
    // exists but has no browser, the Windows openers go first.
    let openers: &[&str] = if cfg!(target_os = "macos") {
        &["open"]
    } else if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        &["wslview", "explorer.exe", "xdg-open"]
    } else {
        &["xdg-open"]
    };
    openers.iter().filter_map(|bin| paths::which(bin)).any(|bin| {
        let mut opener = Command::new(bin);
        opener.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        opener.spawn().is_ok()
    })
}

fn copy(text: &str) -> bool {
    let tools: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else {
        &[
            &["clip.exe"],
            &["wl-copy"],
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
        ]
    };
    tools.iter().any(|argv| {
        let Some(bin) = paths::which(argv[0]) else { return false };
        let mut tool = Command::new(bin);
        tool.args(&argv[1..]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        let Ok(mut child) = tool.spawn() else { return false };
        // Dropping stdin closes it; wl-copy and xclip then fork to keep the selection.
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|status| status.success()) && written
    })
}
