//! Terminal ownership: raw mode and the alternate screen, the input thread,
//! and handing the terminal over to a child process and taking it back.

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, Thread};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyEventKind};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{cursor, execute};
use ratatui::Terminal;

use crate::app::Event;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

pub fn enter() -> io::Result<Term> {
    enable_raw_mode()?;
    // The cursor only shows in a search box; a blinking block there is hard to miss.
    // The terminal does the blinking, so it costs no redraws.
    execute!(io::stdout(), EnterAlternateScreen, cursor::SetCursorStyle::BlinkingBlock)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    Ok(terminal)
}

/// Safe to call more than once, and from the panic hook.
pub fn leave() {
    let _ =
        execute!(io::stdout(), LeaveAlternateScreen, cursor::SetCursorStyle::DefaultUserShape, cursor::Show);
    let _ = disable_raw_mode();
}

/// Leaves the terminal when dropped.
pub struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        leave();
    }
}

/// With `panic = "abort"` no destructor runs, so the hook is the only chance
/// to give the user their terminal back before the message prints.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave();
        default(info);
    }));
}

/// The thread that turns terminal input into [`Event`]s. It must not read the
/// tty while a child process owns it (it would eat the user's sudo password).
/// So it blocks in a plain read, which costs nothing while idle, and after
/// each key it parks until the main loop has carried out that key's effects
/// (`ack`): a pause, which only ever follows a key, then finds the tty free.
pub struct Input {
    paused: Arc<AtomicBool>,
    acked: Arc<AtomicBool>,
    thread: Thread,
}

impl Input {
    pub fn spawn(tx: Sender<Event>) -> Input {
        let paused = Arc::new(AtomicBool::new(false));
        let acked = Arc::new(AtomicBool::new(false));
        let (paused_in, acked_in) = (Arc::clone(&paused), Arc::clone(&acked));
        let handle = thread::spawn(move || loop {
            // Parks are looped on a flag, so a spurious wake-up never reads the tty.
            while paused_in.load(Ordering::Acquire) {
                thread::park();
            }
            match event::read() {
                Ok(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => {
                    if tx.send(Event::Key(key)).is_err() {
                        break;
                    }
                    while !acked_in.swap(false, Ordering::AcqRel) {
                        thread::park();
                    }
                }
                Ok(TermEvent::Resize(..)) => {
                    if tx.send(Event::Resize).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        });
        Input { paused, acked, thread: handle.thread().clone() }
    }

    /// The effects of the last key have been carried out; the thread may read again.
    pub fn ack(&self) {
        self.acked.store(true, Ordering::Release);
        self.thread.unpark();
    }

    /// Keeps the thread off the tty. Only valid between a key and its `ack`,
    /// which is where every terminal job starts.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.thread.unpark();
    }
}
