//! Modal overlays: pickers, help and the job log.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Widget};

use crate::app::{App, Help, Picker, PickerKind, HELP};
use crate::theme::{Theme, THEMES};

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// A `width` x `height` rectangle centred in `area`, clipped to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (w, h) = (width.min(area.width), height.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

/// A modal box holds its contents `PAD` columns and a line clear of the
/// border, which is what the `centered` sizes below are measured against.
const PAD: u16 = 1;

fn frame(buf: &mut Buffer, rect: Rect, t: &Theme, title: &str, footer: &'static str) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(t.accent())
        .padding(Padding::new(PAD, PAD, 1, 1))
        .title_top(format!(" {title} "));
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    buf.set_style(rect, t.base());
    block.render(rect, buf);
    super::gradient(buf, rect, t);
    // The footer goes on after the sweep, which would otherwise light it up
    // to the same brightness as the border it is meant to sit quietly inside.
    if !rect.is_empty() {
        let room = rect.width.saturating_sub(2);
        let len = (footer.chars().count() as u16).min(room);
        let x = rect.right().saturating_sub(len + 1);
        buf.set_stringn(x, rect.bottom() - 1, footer, len as usize, t.dim());
    }
    inner
}

/// Returns where the cursor goes while typing a theme search.
pub fn picker(buf: &mut Buffer, area: Rect, picker: &Picker, t: &Theme) -> Option<(u16, u16)> {
    let search = match &picker.kind {
        PickerKind::Theme { filter, .. } => Some(filter.as_deref()),
        _ => None,
    };
    let footer = match search {
        Some(Some(_)) => " ↑↓ move · enter ok · esc clears ",
        Some(None) => " ↑↓ move · enter ok · esc cancels ",
        None => " enter ok · esc cancel ",
    };
    // A searchable list keeps its full height so the box doesn't jump while typing.
    let (rows, bar) = match search {
        Some(_) => (THEMES.len(), 2),
        None => (picker.items.len(), 0),
    };
    let widest = picker.items.iter().map(|i| i.chars().count()).max().unwrap_or(0);
    let widest = widest.max(picker.title.chars().count() + 2).max(footer.chars().count());
    // Roomy even for one short command, and a sensible minimum width.
    let rect = centered(area, (widest as u16 + 2 * PAD + 4).max(54), rows as u16 + bar + 4);
    let inner = frame(buf, rect, t, &picker.title, footer);
    let mut cursor = None;
    let inner = match search {
        Some(filter) => {
            cursor = search_bar(buf, inner, t, filter, "Press / to search themes");
            if picker.items.is_empty() && inner.height > 2 {
                let width = inner.width.saturating_sub(1) as usize;
                buf.set_stringn(inner.x, inner.y + 2, "No matching themes", width, t.dim());
            }
            Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner }
        }
        None => inner,
    };

    // ✓ marks the saved theme, which may differ from the one being previewed.
    let active = match &picker.kind {
        PickerKind::Theme { original, ids, .. } => ids.iter().position(|i| i == original),
        _ => None,
    };
    let indent = if search.is_some() { 2 } else { 0 };
    // Keep the selection visible when the list is taller than the screen.
    let first = picker.selected.saturating_sub(inner.height.saturating_sub(1) as usize);
    for ((i, item), y) in picker.items.iter().enumerate().skip(first).zip(inner.y..inner.bottom()) {
        let style = if i == picker.selected { t.selected() } else { Style::new() };
        buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
        if active == Some(i) {
            let mark = if i == picker.selected { style } else { Style::new().fg(t.installed) };
            buf.set_stringn(inner.x, y, "✓", 1, mark);
        }
        let width = inner.width.saturating_sub(indent) as usize;
        buf.set_stringn(inner.x + indent, y, item, width, style);
        // Theme names are lowercase, and the filter matches them lowercased.
        if let Some(Some(filter)) = search {
            let col = (inner.x + indent, width as u16);
            super::lit_sub(buf, y, col, item, &filter.to_lowercase(), i == picker.selected, t);
        }
    }
    cursor
}

/// The `/ text` line and a rule under it, or `hint` until `/` is pressed.
/// Returns where the cursor goes while typing.
fn search_bar(
    buf: &mut Buffer,
    inner: Rect,
    t: &Theme,
    filter: Option<&str>,
    hint: &str,
) -> Option<(u16, u16)> {
    if inner.height == 0 || inner.width < 4 {
        return None;
    }
    let cursor = match filter {
        Some(f) => {
            let bar = Line::from(vec![Span::styled("/ ", t.accent()), Span::styled(f, BOLD)]);
            buf.set_line(inner.x, inner.y, &bar, inner.width);
            let x = inner.x + 2 + f.chars().count() as u16;
            Some((x.min(inner.right().saturating_sub(1)), inner.y))
        }
        None => {
            buf.set_stringn(inner.x, inner.y, hint, inner.width as usize, t.dim());
            None
        }
    };
    if inner.height > 1 {
        let rule = "─".repeat(inner.width as usize);
        buf.set_stringn(inner.x, inner.y + 1, rule, inner.width as usize, t.dim());
    }
    cursor
}

/// A search bar over the narrowed key list. Returns where the cursor goes while typing.
pub fn help(buf: &mut Buffer, area: Rect, t: &Theme, help: &Help) -> Option<(u16, u16)> {
    let filter = help.filter.as_deref();
    let rows: Vec<_> = help.rows().collect();
    let footer = if filter.is_some() { " ↑↓ move · esc clears " } else { " ↑↓ move · esc closes " };
    let inner = frame(buf, centered(area, 62 + 2 * PAD, HELP.len() as u16 + 6), t, "Keys", footer);
    if inner.height == 0 || inner.width < 4 {
        return None;
    }
    let width = inner.width.saturating_sub(1) as usize;
    let cursor = search_bar(buf, inner, t, filter, "Press / to search keys");
    if rows.is_empty() && inner.height > 2 {
        buf.set_stringn(inner.x, inner.y + 2, "No matching keys", width, t.dim());
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
        buf.set_stringn(inner.x, y, keys, 16, keys_style);
        let what_w = inner.width.saturating_sub(17);
        buf.set_stringn(inner.x + 17, y, what, what_w as usize, Style::new());
        // The filter matches keys or description, by case, so both are worth lighting.
        if let Some(filter) = filter {
            super::lit_sub(buf, y, (inner.x, 16), keys, filter, selected, t);
            super::lit_sub(buf, y, (inner.x + 17, what_w), what, filter, selected, t);
        }
    }
    cursor
}

/// The tail of the job log; older lines scroll off the top.
pub fn log(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let rect = centered(area, area.width.saturating_sub(4), area.height.saturating_sub(4));
    let inner = frame(buf, rect, t, "Job output", " any key closes ");
    // Too short for the padding, which leaves the inner area below the screen.
    if inner.is_empty() {
        return;
    }
    if app.log.is_empty() {
        buf.set_stringn(inner.x, inner.y, "No jobs have run yet.", inner.width as usize, t.dim());
        return;
    }
    let skip = app.log.len().saturating_sub(inner.height as usize);
    for (line, y) in app.log.iter().skip(skip).zip(inner.y..inner.bottom()) {
        let style = if line.starts_with("$ ") { t.accent() } else { Style::new() };
        buf.set_stringn(inner.x, y, line, inner.width as usize, style);
    }
}
