//! Terminal ownership: raw mode and the alternate screen, the input thread,
//! and handing the terminal over to a child process and taking it back.

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, Thread};
use std::time::Duration;

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

/// How long the input thread waits for a key before checking whether it has
/// been asked to pause. Bounds the delay before a sudo prompt can appear.
const POLL: Duration = Duration::from_millis(250);

/// The thread that turns terminal input into [`Event`]s. It must not read the
/// tty while a child process owns it (it would eat the user's sudo password),
/// so it can be paused: `pause` returns only once the thread has let go.
pub struct Input {
    paused: Arc<AtomicBool>,
    tty: Arc<Mutex<()>>,
    thread: Thread,
}

impl Input {
    pub fn spawn(tx: Sender<Event>) -> Input {
        let paused = Arc::new(AtomicBool::new(false));
        let tty = Arc::new(Mutex::new(()));
        let (paused_in, tty_in) = (Arc::clone(&paused), Arc::clone(&tty));
        let handle = thread::spawn(move || loop {
            if paused_in.load(Ordering::Acquire) {
                thread::park();
                continue;
            }
            let _tty = tty_in.lock().unwrap_or_else(|e| e.into_inner());
            let event = match event::poll(POLL) {
                Ok(true) => event::read(),
                Ok(false) => continue,
                Err(e) => Err(e),
            };
            let sent = match event {
                Ok(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => tx.send(Event::Key(key)),
                Ok(TermEvent::Resize(..)) => tx.send(Event::Resize),
                Ok(_) => Ok(()),
                Err(_) => break,
            };
            if sent.is_err() {
                break;
            }
        });
        Input { paused, tty, thread: handle.thread().clone() }
    }

    pub fn pause(&self) -> MutexGuard<'_, ()> {
        self.paused.store(true, Ordering::Release);
        self.tty.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn resume(&self, guard: MutexGuard<'_, ()>) {
        drop(guard);
        self.paused.store(false, Ordering::Release);
        self.thread.unpark();
    }
}
