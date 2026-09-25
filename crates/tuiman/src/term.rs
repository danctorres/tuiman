//! Terminal ownership: raw mode and the alternate screen, the input thread,
//! and handing the terminal over to a child process and taking it back.

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::crossterm::style::Print;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{cursor, execute};
use ratatui::Terminal;

use crate::app::{Event, Press};

/// Two left clicks this close together are a double click; the core checks
/// that they landed on the same thing.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Mouse reporting for clicks and the wheel, in SGR encoding. Not crossterm's
/// `EnableMouseCapture`, which also turns on motion tracking (modes 1002 and
/// 1003): that would wake the input thread on every pointer movement.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1000l";

pub fn enter() -> io::Result<Term> {
    enable_raw_mode()?;
    // The cursor only shows in a search box; a blinking block there is hard to miss.
    // The terminal does the blinking, so it costs no redraws.
    execute!(io::stdout(), EnterAlternateScreen, cursor::SetCursorStyle::BlinkingBlock, Print(MOUSE_ON))?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    Ok(terminal)
}

/// Safe to call more than once, and from the panic hook.
pub fn leave() {
    let _ = execute!(
        io::stdout(),
        Print(MOUSE_OFF),
        LeaveAlternateScreen,
        cursor::SetCursorStyle::DefaultUserShape,
        cursor::Show
    );
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
/// each key or click it parks until the main loop has carried out its effects
/// (`ack`): a pause, which only ever follows one, then finds the tty free.
/// Wheel steps only move a selection, so they flow through unacknowledged and
/// a fast wheel is drained into one frame.
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
        let handle = thread::spawn(move || {
            // When the last single left click was.
            let mut last_click: Option<Instant> = None;
            loop {
                // Parks are looped on a flag, so a spurious wake-up never reads the tty.
                while paused_in.load(Ordering::Acquire) {
                    thread::park();
                }
                let event = match event::read() {
                    Ok(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => Event::Key(key),
                    Ok(TermEvent::Mouse(mouse)) => {
                        let press = match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                let now = Instant::now();
                                let double =
                                    last_click.is_some_and(|at| now.duration_since(at) < DOUBLE_CLICK);
                                // A third click starts over rather than acting twice.
                                last_click = if double { None } else { Some(now) };
                                if double {
                                    Press::Act
                                } else {
                                    Press::Select
                                }
                            }
                            MouseEventKind::Down(MouseButton::Right) => Press::Open,
                            MouseEventKind::ScrollDown => Press::Wheel(1),
                            MouseEventKind::ScrollUp => Press::Wheel(-1),
                            _ => continue,
                        };
                        Event::Mouse { x: mouse.column, y: mouse.row, press }
                    }
                    Ok(TermEvent::Resize(..)) => Event::Resize,
                    Ok(_) => continue,
                    Err(_) => break,
                };
                let ack = event.needs_ack();
                if tx.send(event).is_err() {
                    break;
                }
                while ack && !acked_in.swap(false, Ordering::AcqRel) {
                    thread::park();
                }
            }
        });
        Input { paused, acked, thread: handle.thread().clone() }
    }

    /// The effects of the last key or click have been carried out; the thread may read again.
    pub fn ack(&self) {
        self.acked.store(true, Ordering::Release);
        self.thread.unpark();
    }

    /// Keeps the thread off the tty. Only valid between a key or click and its `ack`,
    /// which is where every terminal job starts.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.thread.unpark();
    }
}
