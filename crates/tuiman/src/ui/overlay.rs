//! Modal overlays: pickers, help and the job log.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Widget};

use super::{ACCENT, DIM, SELECTED};
use crate::app::{App, Picker};

const HELP: [(&str, &str); 19] = [
    ("j k ↓ ↑", "move"),
    ("g G", "first / last"),
    ("ctrl-d ctrl-u", "half page down / up"),
    ("h l ← → tab", "previous / next category"),
    ("/", "fuzzy search (enter keeps, esc clears)"),
    ("s", "cycle sort: stars, name, last push"),
    ("S", "minimum stars"),
    ("L", "language"),
    ("t", "installed only"),
    ("a", "installable on this machine only"),
    ("A", "show archived projects"),
    ("c", "clear all filters"),
    ("i enter", "install"),
    ("x", "uninstall"),
    ("o", "open the project page"),
    ("r", "refresh the index"),
    ("v", "view job output"),
    ("?", "this help"),
    ("q", "quit"),
];

/// A `width` x `height` rectangle centred in `area`, clipped to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (w, h) = (width.min(area.width), height.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn frame(buf: &mut Buffer, rect: Rect, title: &str, footer: &'static str) -> Rect {
    let block = Block::bordered()
        .border_style(ACCENT)
        .title_top(format!(" {title} "))
        .title_bottom(Line::from(footer).style(DIM).right_aligned());
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    block.render(rect, buf);
    inner
}

pub fn picker(buf: &mut Buffer, area: Rect, picker: &Picker) {
    let widest =
        picker.items.iter().map(|i| i.chars().count()).max().unwrap_or(0).max(picker.title.len() + 2);
    let rect = centered(area, widest as u16 + 6, picker.items.len() as u16 + 2);
    let inner = frame(buf, rect, &picker.title, " enter ok · esc cancel ");

    // Keep the selection visible when the list is taller than the screen.
    let first = picker.selected.saturating_sub(inner.height.saturating_sub(1) as usize);
    for ((i, item), y) in picker.items.iter().enumerate().skip(first).zip(inner.y..inner.bottom()) {
        let style = if i == picker.selected { SELECTED } else { Style::new() };
        buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
        buf.set_stringn(inner.x + 1, y, item, inner.width.saturating_sub(2) as usize, style);
    }
}

pub fn help(buf: &mut Buffer, area: Rect) {
    let inner = frame(buf, centered(area, 62, HELP.len() as u16 + 2), "Keys", " any key closes ");
    for ((keys, what), y) in HELP.iter().zip(inner.y..inner.bottom()) {
        buf.set_stringn(inner.x + 1, y, keys, 16, ACCENT);
        buf.set_stringn(inner.x + 18, y, what, inner.width.saturating_sub(19) as usize, Style::new());
    }
}

/// The tail of the job log; older lines scroll off the top.
pub fn log(buf: &mut Buffer, area: Rect, app: &App) {
    let rect = centered(area, area.width.saturating_sub(4), area.height.saturating_sub(2));
    let inner = frame(buf, rect, "Job output", " any key closes ");
    if app.log.is_empty() {
        buf.set_stringn(
            inner.x + 1,
            inner.y,
            "No jobs have run yet.",
            inner.width.saturating_sub(1) as usize,
            DIM,
        );
        return;
    }
    let skip = app.log.len().saturating_sub(inner.height as usize);
    for (line, y) in app.log.iter().skip(skip).zip(inner.y..inner.bottom()) {
        let style = if line.starts_with("$ ") { ACCENT } else { Style::new() };
        buf.set_stringn(inner.x + 1, y, line, inner.width.saturating_sub(1) as usize, style);
    }
}
