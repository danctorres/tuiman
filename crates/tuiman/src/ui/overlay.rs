//! Modal overlays: pickers, help and the job log.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};

use crate::app::{App, Help, Picker, HELP};
use crate::theme::Theme;

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// A `width` x `height` rectangle centred in `area`, clipped to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (w, h) = (width.min(area.width), height.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn frame(buf: &mut Buffer, rect: Rect, t: &Theme, title: &str, footer: &'static str) -> Rect {
    let block = Block::bordered()
        .border_style(t.accent())
        .title_top(format!(" {title} "))
        .title_bottom(Line::from(footer).style(t.dim()).right_aligned());
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    buf.set_style(rect, t.base());
    block.render(rect, buf);
    inner
}

pub fn picker(buf: &mut Buffer, area: Rect, picker: &Picker, t: &Theme) {
    const FOOTER: &str = " enter ok · esc cancel ";
    let widest = picker.items.iter().map(|i| i.chars().count()).max().unwrap_or(0);
    let widest = widest.max(picker.title.chars().count() + 2).max(FOOTER.chars().count());
    // Roomy even for one short command: a margin all round and a sensible minimum width.
    let rect = centered(area, (widest as u16 + 8).max(50), picker.items.len() as u16 + 4);
    let inner = frame(buf, rect, t, &picker.title, FOOTER);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }
        .inner(ratatui::layout::Margin::new(0, 1));

    // Keep the selection visible when the list is taller than the screen.
    let first = picker.selected.saturating_sub(inner.height.saturating_sub(1) as usize);
    for ((i, item), y) in picker.items.iter().enumerate().skip(first).zip(inner.y..inner.bottom()) {
        let style = if i == picker.selected { t.selected() } else { Style::new() };
        buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
        buf.set_stringn(inner.x + 1, y, item, inner.width.saturating_sub(2) as usize, style);
    }
}

/// A search bar over the narrowed key list. Returns where the cursor goes while typing.
pub fn help(buf: &mut Buffer, area: Rect, t: &Theme, help: &Help) -> Option<(u16, u16)> {
    let filter = help.filter.as_deref();
    let rows: Vec<_> = help.rows().collect();
    let footer = if filter.is_some() { " ↑↓ move · esc clears " } else { " ↑↓ move · esc closes " };
    let inner = frame(buf, centered(area, 62, HELP.len() as u16 + 4), t, "Keys", footer);
    if inner.height == 0 || inner.width < 4 {
        return None;
    }
    let width = inner.width.saturating_sub(1) as usize;
    let cursor = match filter {
        Some(f) => {
            let bar = Line::from(vec![Span::styled("/ ", t.accent()), Span::styled(f, BOLD)]);
            buf.set_line(inner.x + 1, inner.y, &bar, inner.width - 1);
            let x = inner.x + 3 + f.chars().count() as u16;
            Some((x.min(inner.right().saturating_sub(1)), inner.y))
        }
        None => {
            buf.set_stringn(inner.x + 1, inner.y, "Press / to search keys", width, t.dim());
            None
        }
    };
    if inner.height > 1 {
        buf.set_stringn(
            inner.x,
            inner.y + 1,
            "─".repeat(inner.width as usize),
            inner.width as usize,
            t.dim(),
        );
    }
    if rows.is_empty() {
        buf.set_stringn(inner.x + 1, inner.y + 2, "No matching keys", width, t.dim());
    }
    // Keep the selection visible when the list is taller than the box.
    let list_height = inner.height.saturating_sub(2) as usize;
    let first = help.selected.saturating_sub(list_height.saturating_sub(1));
    for ((i, (keys, what)), y) in rows.into_iter().enumerate().skip(first).zip(inner.y + 2..inner.bottom()) {
        let selected = i == help.selected;
        if selected {
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), t.selected());
        }
        let keys_style = if selected { t.selected() } else { t.accent() };
        buf.set_stringn(inner.x + 1, y, keys, 16, keys_style);
        buf.set_stringn(inner.x + 18, y, what, inner.width.saturating_sub(19) as usize, Style::new());
    }
    cursor
}

/// The tail of the job log; older lines scroll off the top.
pub fn log(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let rect = centered(area, area.width.saturating_sub(4), area.height.saturating_sub(2));
    let inner = frame(buf, rect, t, "Job output", " any key closes ");
    if app.log.is_empty() {
        buf.set_stringn(
            inner.x + 1,
            inner.y,
            "No jobs have run yet.",
            inner.width.saturating_sub(1) as usize,
            t.dim(),
        );
        return;
    }
    let skip = app.log.len().saturating_sub(inner.height as usize);
    for (line, y) in app.log.iter().skip(skip).zip(inner.y..inner.bottom()) {
        let style = if line.starts_with("$ ") { t.accent() } else { Style::new() };
        buf.set_stringn(inner.x + 1, y, line, inner.width.saturating_sub(1) as usize, style);
    }
}
