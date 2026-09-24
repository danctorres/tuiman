//! Everything slow runs here, on short-lived threads that report back to the
//! main loop as [`Event`]s: installed scans, index refreshes and jobs.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::app::{Event, Job};
use crate::fetch;
use crate::managers::{self, ManagerId, MANAGERS};

pub fn detect(tx: Sender<Event>) {
    thread::spawn(move || {
        let _ = tx.send(Event::Detected(managers::detect()));
    });
}

pub fn scan_path(tx: Sender<Event>) {
    thread::spawn(move || {
        let _ = tx.send(Event::OnPath(crate::paths::names_on_path()));
    });
}

pub fn scan(tx: Sender<Event>, manager: ManagerId, bin: PathBuf) {
    thread::spawn(move || {
        let names = (MANAGERS[manager as usize].list_installed)(&bin);
        let _ = tx.send(Event::Listing(manager, names));
    });
}

pub fn refresh_index(tx: Sender<Event>, cache_dir: PathBuf) {
    thread::spawn(move || {
        let _ = tx.send(Event::Index(fetch::refresh(&cache_dir)));
    });
}

/// Builds the process for a job. Always an argument vector, never a shell.
/// Release installs are `tuiman` itself, which need not be on `PATH`.
pub fn command(job: &Job) -> Command {
    let mut command = match MANAGERS[job.manager as usize].eco.is_release() {
        true => Command::new(std::env::current_exe().unwrap_or_else(|_| job.argv[0].clone().into())),
        false => Command::new(&job.argv[0]),
    };
    command.args(&job.argv[1..]);
    command
}

/// Runs a job with captured output, streamed line by line into the log.
pub fn spawn_job(tx: Sender<Event>, job: &Job) {
    let mut command = command(job);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).env("NO_COLOR", "1");
    thread::spawn(move || {
        let ok = match command.spawn() {
            Ok(mut child) => {
                let pipes = [
                    child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
                    child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
                ];
                let readers: Vec<_> = pipes
                    .into_iter()
                    .flatten()
                    .map(|pipe| {
                        let tx = tx.clone();
                        thread::spawn(move || stream_lines(pipe, &tx))
                    })
                    .collect();
                let status = child.wait();
                readers.into_iter().for_each(|r| drop(r.join()));
                status.is_ok_and(|s| s.success())
            }
            Err(e) => {
                let _ = tx.send(Event::JobOutput(format!("cannot start: {e}")));
                false
            }
        };
        let _ = tx.send(Event::JobDone { ok });
    });
}

/// Progress bars rewrite their line with `\r`; each rewrite becomes a line.
fn stream_lines(mut pipe: impl Read, tx: &Sender<Event>) {
    let mut pending = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n @ 1..) = pipe.read(&mut chunk) {
        for &byte in &chunk[..n] {
            if byte != b'\n' && byte != b'\r' {
                pending.push(byte);
            } else if !pending.is_empty() {
                let _ = tx.send(Event::JobOutput(printable(&String::from_utf8_lossy(&pending))));
                pending.clear();
            }
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(Event::JobOutput(printable(&String::from_utf8_lossy(&pending))));
    }
}

/// Strips escape sequences, control characters and emoji: job output is
/// untrusted text that gets drawn inside our own screen.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then one final byte in `@`..=`~`.
                Some('[') => drop(chars.by_ref().find(|c| ('@'..='~').contains(c))),
                // OSC: runs to BEL or to the `ESC \` terminator.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next().is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => out.push_str("    "),
            c if c.is_control() || tuiman_index::is_emoji(c) => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::Action;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn strips_escapes() {
        assert_eq!(printable("\x1b[1;32m==>\x1b[0m Pouring btop"), "==> Pouring btop");
        assert_eq!(printable("\x1b]0;title\x07text\x1b]8;;x\x1b\\!"), "text!");
        assert_eq!(printable("a\tb\x08\x00c"), "a    bc");
        assert_eq!(printable("dangling \x1b"), "dangling ");
        assert_eq!(printable("\u{2714}\u{fe0e} Bottle \u{1f37a}  done"), " Bottle   done");
    }

    #[test]
    fn splits_on_newlines_and_carriage_returns() {
        let (tx, rx) = mpsc::channel();
        stream_lines(&b"10%\r50%\r100%\ndone\n\ntail"[..], &tx);
        drop(tx);
        let lines: Vec<String> =
            rx.iter().map(|e| if let Event::JobOutput(l) = e { l } else { unreachable!() }).collect();
        assert_eq!(lines, ["10%", "50%", "100%", "done", "tail"]);
    }

    fn run(argv: &[&str]) -> (Vec<String>, bool) {
        let (tx, rx) = mpsc::channel();
        let job = Job {
            action: Action::Install,
            manager: 0,
            row: 0,
            title: "test".into(),
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        };
        spawn_job(tx, &job);
        let mut lines = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("job never finished") {
                Event::JobOutput(line) => lines.push(line),
                Event::JobDone { ok } => return (lines, ok),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn jobs_stream_output_and_report_status() {
        let (mut lines, ok) = run(&["sh", "-c", "echo out; echo err >&2"]);
        lines.sort();
        assert_eq!((lines, ok), (vec!["err".to_owned(), "out".to_owned()], true));
        assert!(!run(&["sh", "-c", "exit 3"]).1);
        let (lines, ok) = run(&["/nonexistent/tuiman-test-binary"]);
        assert!(!ok && lines[0].starts_with("cannot start"));
    }

    #[test]
    fn arguments_are_never_interpreted_by_a_shell() {
        let (lines, _) = run(&["echo", "$(id); `id` && x"]);
        assert_eq!(lines, ["$(id); `id` && x"]);
    }
}
