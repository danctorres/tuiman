//! Rendering. Pure functions from `&App` to a frame: nothing here mutates
//! application state, and nothing outside here knows about widgets.
//!
//! The table, the only part whose cost scales with the catalog, writes the
//! visible rows straight into the cell buffer instead of building widget
//! trees, so a frame costs the same for 700 entries as for 70,000.

mod format;
mod overlay;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tuiman_index::Row;

use crate::app::{App, Mode, FLASH_TICKS};
use crate::managers::MANAGERS;
use crate::query::Sort;
use crate::theme::Theme;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// Nerd Font glyphs: the left cap of a solid badge and a thin group separator.
const POWER_CAP: &str = "\u{e0b2}";
const POWER_SEP: &str = " \u{e0b1} ";

/// Whether to draw Nerd Font glyphs in the status bar. Opt-in, since a
/// terminal without a patched font shows them as boxes.
fn powerline() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TUIMAN_POWERLINE").is_some_and(|v| v != "0"))
}

pub struct Areas {
    pub sidebar: Rect,
    pub table: Rect,
    pub details: Rect,
    pub status: Rect,
}

/// Hand-rolled layout: four rectangles do not need a constraint solver.
/// Tiles float with a gap around and between them, as a tiling compositor
/// lays out windows. The gap comes out of the table's share, so the sidebar
/// and details appear at the same sizes as without it; only a terminal too
/// small to spare a column goes without.
pub fn areas(area: Rect) -> Areas {
    let status_h = area.height.min(1);
    let full_h = area.height - status_h;
    let gap = u16::from(area.width >= 40 && full_h >= 10);
    let body_h = full_h - gap;
    let (x, y, width) = (area.x + gap, area.y + gap, area.width.saturating_sub(2 * gap));
    let details_h = if full_h >= 18 { 8 } else { 0 };
    let sidebar_w = if area.width >= 90 { 24 } else { 0 };
    let top_h = body_h - details_h - if details_h > 0 { gap } else { 0 };
    let table_x = if sidebar_w > 0 { sidebar_w + gap } else { 0 };
    Areas {
        sidebar: Rect::new(x, y, sidebar_w, top_h),
        table: Rect::new(x + table_x, y, width - table_x, top_h),
        details: Rect::new(x, y + top_h + gap, width, details_h),
        status: Rect::new(area.x, area.y + full_h, area.width, status_h),
    }
}

/// What the app needs to know about a terminal of this size: the number of
/// table body lines (borders and header excluded) and whether the sidebar fits.
pub fn layout(area: Rect) -> (usize, bool) {
    let areas = areas(area);
    (areas.table.height.saturating_sub(3) as usize, !areas.sidebar.is_empty())
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let areas = areas(area);
    let buf = frame.buffer_mut();
    buf.set_style(area, app.theme().base());

    if !areas.sidebar.is_empty() {
        sidebar(buf, areas.sidebar, app);
    }
    table(buf, areas.table, app);
    if !areas.details.is_empty() {
        details(buf, areas.details, app);
    }
    status(buf, areas.status, app);

    match &app.mode {
        Mode::Picker(picker) => {
            if let Some(pos) = overlay::picker(buf, area, picker, app.theme()) {
                frame.set_cursor_position(pos);
            }
        }
        Mode::Help(help) => {
            if let Some(pos) = overlay::help(buf, area, app.theme(), help) {
                frame.set_cursor_position(pos);
            }
        }
        Mode::Log => overlay::log(buf, area, app),
        Mode::Normal | Mode::Search | Mode::CategorySearch { .. } => {}
    }
    let typing = match &app.mode {
        Mode::Search => Some((areas.table, app.query.text.as_str())),
        Mode::CategorySearch { text, .. } if !areas.sidebar.is_empty() => {
            Some((areas.sidebar, text.as_str()))
        }
        _ => None,
    };
    if let Some((rect, text)) = typing {
        // Title starts after the corner and " /".
        let x = rect.x + 3 + text.chars().count() as u16;
        frame.set_cursor_position((x.min(rect.right().saturating_sub(2)), rect.y));
    }
}

fn sidebar(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let title = match &app.mode {
        Mode::CategorySearch { text, .. } => {
            Line::from(vec![" /".into(), Span::styled(text.as_str(), BOLD), " ".into()])
        }
        _ => Line::from(" Categories "),
    };
    let block = panel(t, app.sidebar_focused).title(title);
    let inner = block.inner(area);
    block.render(area, buf);
    if app.sidebar_focused {
        gradient(buf, area, t);
    }

    let entries = std::iter::once((None, "All", app.view.category_counts.iter().sum())).chain(
        (0..app.catalog.category_count()).map(|id| {
            (
                Some(id as u8),
                app.catalog.category_name(id as u8),
                app.view.category_counts.get(id).copied().unwrap_or(0),
            )
        }),
    );
    for ((id, name, count), y) in entries.zip(inner.y..inner.bottom()) {
        let style = match (id == app.query.category, count) {
            (true, _) => cursor(t, app.sidebar_focused),
            (false, 0) => t.dim(),
            (false, _) => Style::new(),
        };
        let line = Rect::new(inner.x, y, inner.width, 1);
        buf.set_style(line, style);
        buf.set_stringn(inner.x + 1, y, name, inner.width.saturating_sub(7) as usize, style);
        let count = count.to_string();
        let x = inner.right().saturating_sub(count.len() as u16 + 1);
        buf.set_stringn(x, y, &count, count.len(), if id == app.query.category { style } else { t.dim() });
        // After the text, which would otherwise paint over the sweep.
        if id == app.query.category {
            cursor_bar(buf, line, t, app.sidebar_focused);
        }
    }
}

/// Column x-offsets and widths for a given inner table width.
struct Columns {
    number: Option<(u16, u16)>,
    name: (u16, u16),
    stars: (u16, u16),
    language: Option<(u16, u16)>,
    age: Option<(u16, u16)>,
    desc: (u16, u16),
}

impl Columns {
    /// `rows` sizes the line number column to the largest number.
    fn new(width: u16, rows: usize) -> Columns {
        let wide = width >= 72;
        let name_w = if wide { 24 } else { 18 }.min(width.saturating_sub(10));
        let mut x = 2;
        let mut next = |w: u16| {
            let col = (x, w);
            x += w + 1;
            col
        };
        let number = wide.then(|| next(rows.to_string().len() as u16));
        let name = next(name_w);
        let stars = next(6);
        let language = wide.then(|| next(11));
        let age = wide.then(|| next(6));
        let desc = (x, width.saturating_sub(x));
        Columns { number, name, stars, language, age, desc }
    }
}

fn table(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let title = match (&app.mode, app.query.text.is_empty()) {
        (Mode::Search, _) | (_, false) => {
            Line::from(vec![" /".into(), Span::styled(app.query.text.as_str(), BOLD), " ".into()])
        }
        _ => Line::from(" TUIs "),
    };
    let block = panel(t, !app.sidebar_focused).title_top(title).title_top(filters(app).right_aligned());
    let inner = block.inner(area);
    block.render(area, buf);
    if !app.sidebar_focused {
        gradient(buf, area, t);
    }
    if inner.height < 2 || inner.width < 12 {
        return;
    }

    let cols = Columns::new(inner.width, app.view.rows.len());
    // The sorted column's header is the one bright thing in the header row.
    let header = |label: &str, sort: Sort| match sort == app.query.sort {
        true => (format!("{label} ▾"), t.accent().patch(BOLD)),
        false => (label.to_owned(), t.dim()),
    };
    let put = |buf: &mut Buffer, y, (x, w): (u16, u16), text: &str, style| {
        buf.set_stringn(inner.x + x, y, text, w as usize, style);
    };
    let put_right = |buf: &mut Buffer, y, (x, w): (u16, u16), text: &str, style| {
        let pad = w.saturating_sub(text.chars().count() as u16);
        buf.set_stringn(inner.x + x + pad, y, text, (w - pad) as usize, style);
    };

    let (name, style) = header("NAME", Sort::Name);
    put(buf, inner.y, cols.name, &name, style);
    let (stars, style) = header("★", Sort::Stars);
    put_right(buf, inner.y, cols.stars, &stars, style);
    if let (Some(language), Some(age)) = (cols.language, cols.age) {
        put(buf, inner.y, language, "LANGUAGE", t.dim());
        let (push, style) = header("PUSH", Sort::Updated);
        put_right(buf, inner.y, age, &push, style);
    }
    put(buf, inner.y, cols.desc, "DESCRIPTION", t.dim());

    if app.view.rows.is_empty() {
        let message = match (app.catalog.is_empty(), app.refreshing) {
            (true, true) => "Downloading the index…",
            (true, false) => "No index yet (press r to download it)",
            (false, _) => "Nothing matches (press c to clear filters)",
        };
        put(buf, inner.y + 2, (2, inner.width.saturating_sub(2)), message, t.dim());
        return;
    }

    let stripe = t.stripe();
    let body = (inner.y + 1..inner.bottom()).zip(app.view.rows.iter().enumerate().skip(app.offset));
    for (y, (i, &row)) in body {
        let cat = &app.catalog;
        let selected = i == app.selected;
        let faded = cat.is_archived(row);
        let text = if faded { t.dim() } else { Style::new() };
        // Alternating rows sit on a shade of the background, so a wide table
        // still reads across. Themes on the terminal palette get no stripe.
        if i % 2 == 1 {
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), stripe);
        }

        // A job on this row outranks the installed mark: spinning while it runs, ⋯ while queued.
        if app.running.as_ref().is_some_and(|job| job.row == row) {
            put(buf, y, (0, 1), SPINNER[app.spinner % SPINNER.len()], t.accent().patch(BOLD));
        } else if app.queue.iter().any(|job| job.row == row) {
            put(buf, y, (0, 1), "⋯", t.accent());
        } else if app.installed.is_installed(row) {
            put(buf, y, (0, 1), "✓", Style::new().fg(t.installed));
        }
        // Numbered from 1, so the number is what 3gg takes.
        if let Some(number) = cols.number {
            put_right(buf, y, number, &(i + 1).to_string(), t.dim());
        }
        put(buf, y, cols.name, cat.name(row), text.patch(BOLD));
        put_right(
            buf,
            y,
            cols.stars,
            &format::stars(cat.stars(row)),
            if faded { t.dim() } else { t.heat(cat.stars(row)) },
        );
        if let (Some(language), Some(age)) = (cols.language, cols.age) {
            put(buf, y, language, cat.language(row), t.dim());
            put_right(buf, y, age, &format::age(cat.pushed_days(row), app.today_days), t.dim());
        }
        put(buf, y, cols.desc, cat.desc(row), text);
        if selected {
            cursor_bar(buf, Rect::new(inner.x, y, inner.width, 1), t, !app.sidebar_focused);
        }
    }
}

/// A focusable panel: rounded throughout, the focused one lit by a diagonal
/// accent-to-link gradient the way a tiling compositor marks the active window.
fn panel(t: &Theme, focused: bool) -> Block<'static> {
    let block = Block::bordered().border_type(BorderType::Rounded);
    match focused {
        true => block.border_style(t.accent()).title_style(t.accent().patch(BOLD)),
        false => block.border_style(t.dim()).title_style(t.dim()),
    }
}

/// Repaints an already-drawn border with a top-left to bottom-right sweep.
/// Walks the perimeter only, so it costs the border, not the panel.
pub fn gradient(buf: &mut Buffer, area: Rect, t: &Theme) {
    if area.is_empty() || t.sweep(0.0).is_none() {
        return;
    }
    let span = (area.width + area.height).saturating_sub(2).max(1) as f32;
    let (top, bottom, left, right) = (area.y, area.bottom() - 1, area.x, area.right() - 1);
    let mut paint = |x: u16, y: u16| {
        let level = ((x - area.x) + (y - area.y)) as f32 / span;
        if let Some(colour) = t.sweep(level) {
            buf[(x, y)].set_fg(colour);
        }
    };
    for x in left..=right {
        paint(x, top);
        paint(x, bottom);
    }
    for y in top + 1..bottom {
        paint(left, y);
        paint(right, y);
    }
}

/// The selection bar: the same sweep as the border, so the eye reads the row
/// and the panel as one lit thing. Flat where the theme cannot blend.
fn cursor_bar(buf: &mut Buffer, rect: Rect, t: &Theme, focused: bool) {
    buf.set_style(rect, cursor(t, focused));
    if !focused || t.sweep(0.0).is_none() {
        return;
    }
    for (i, x) in (rect.x..rect.right()).enumerate() {
        if let Some(colour) = t.sweep(i as f32 / rect.width.max(1) as f32) {
            buf[(x, rect.y)].set_bg(colour);
        }
    }
}

/// The selection bar: full accent in the focused panel, a muted bar in the other.
fn cursor(t: &Theme, focused: bool) -> Style {
    match focused {
        true => t.selected(),
        // Forces the fg too, so dim cells stay readable on the dim bar.
        false => t.selected().bg(t.dim),
    }
}

/// Active filters, shown right-aligned in the table border.
fn filters(app: &App) -> Line<'static> {
    let q = &app.query;
    let mut parts =
        vec![format!("{}/{}", app.view.rows.len(), app.catalog.len()), format!("sort:{}", q.sort.label())];
    if q.min_stars > 0 {
        parts.push(format!("★≥{}", q.min_stars));
    }
    // Tab still steps categories without the sidebar, so say where it went.
    if let (Some(id), false) = (q.category, app.sidebar_visible) {
        parts.push(format!("cat:{}", app.catalog.category_name(id)));
    }
    if let Some(id) = q.language {
        parts.push(format!("lang:{}", app.catalog.language_name(id)));
    }
    parts.extend(q.installed_only.then(|| "installed".to_owned()));
    parts.extend(q.installable_only.then(|| "installable".to_owned()));
    parts.extend(q.show_archived.then(|| "+archived".to_owned()));
    Line::from(format!(" {} ", parts.join(" · ")))
}

fn details(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(t.dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    block.render(area, buf);
    let Some(row) = app.selected_row() else { return };
    Paragraph::new(detail_lines(app, row)).wrap(Wrap { trim: true }).render(inner, buf);
}

fn detail_lines(app: &App, row: Row) -> Vec<Line<'_>> {
    let t = app.theme();
    let cat = &app.catalog;
    let mut head = vec![Span::styled(cat.name(row), BOLD.fg(t.accent))];
    head.push(Span::styled(format!("  ★ {}", format::stars(cat.stars(row))), Style::new().fg(t.stars)));
    for field in [cat.language(row), cat.license(row), cat.category_name(cat.category_id(row))] {
        if !field.is_empty() {
            head.push(Span::styled(format!(" · {field}"), t.dim()));
        }
    }
    if let Some(days) = cat.pushed_days(row) {
        head.push(Span::styled(format!(" · pushed {}", format::date(days)), t.dim()));
    }
    if cat.is_archived(row) {
        head.push(Span::styled(" · archived", Style::new().fg(t.archived)));
    }
    if cat.is_library(row) {
        head.push(Span::styled(" · library", Style::new().fg(t.library)));
    }

    let mut packages = vec![Span::styled("Packages: ", t.dim())];
    let installed_via: Vec<&str> = app.installed.installed_via(row).collect();
    for (eco, package) in cat.packages(row) {
        let mut here =
            app.installed.detected().iter().map(|(id, _)| &MANAGERS[*id as usize]).filter(|m| m.eco == eco);
        let available = here.clone().next().is_some();
        let is_installed = here.any(|m| installed_via.contains(&m.name));
        let (mark, style) = match (is_installed, available) {
            (true, _) => ("✓ ", Style::new().fg(t.installed)),
            (false, true) => ("", Style::new()),
            (false, false) => ("", t.dim()),
        };
        packages.push(Span::styled(format!("{mark}{}:{package}  ", eco.name()), style));
    }
    if packages.len() == 1 {
        packages.push(Span::styled("none known (o opens the project page)", t.dim()));
    }

    vec![
        Line::from(head),
        Line::from(Span::styled(cat.url(row), Style::new().fg(t.link).add_modifier(Modifier::UNDERLINED))),
        Line::from(cat.desc(row)),
        Line::default(),
        Line::from(packages),
    ]
}

fn status(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    if area.is_empty() {
        return;
    }
    let scanning = if app.detecting { 1 } else { app.scans_pending };
    let activity = match (&app.running, app.refreshing, scanning) {
        (Some(job), _, _) if app.queue.is_empty() => Some(job.title.clone()),
        (Some(job), _, _) => Some(format!("{} (+{} queued)", job.title, app.queue.len())),
        (None, true, _) => Some("refreshing index".to_owned()),
        (None, false, 1..) => Some("scanning installed".to_owned()),
        (None, false, 0) => None,
    };
    let mut right = area.right();
    if let Some(activity) = activity {
        let text = format!(" {} {activity} ", SPINNER[app.spinner % SPINNER.len()]);
        // A running job gets a solid badge; background scans stay a quiet spinner.
        let solid = app.running.is_some();
        let style = if solid { t.selected().patch(BOLD) } else { t.accent() };
        let cap = powerline() && solid;
        let width = (text.chars().count() as u16 + u16::from(cap)).min(area.width);
        right -= width;
        let x = match cap {
            true => buf.set_stringn(right, area.y, POWER_CAP, 1, Style::new().fg(t.accent)).0,
            false => right,
        };
        buf.set_stringn(x, area.y, &text, (right + width - x) as usize, style);
    }
    // The pending count, as vim's showcmd does, so a stray digit isn't a surprise.
    let (text, style) = match app.count {
        0 => (app.last_key.clone(), t.dim()),
        _ => (app.count.to_string(), t.accent().patch(BOLD)),
    };
    if !text.is_empty() {
        let text = format!(" {text} ");
        let width = (text.chars().count() as u16).min(right - area.x);
        right -= width;
        buf.set_stringn(right, area.y, &text, width as usize, style);
    }

    if !app.status.is_empty() {
        buf.set_stringn(
            area.x + 1,
            area.y,
            &app.status,
            right.saturating_sub(area.x + 1) as usize,
            match app.status.chars().next() {
                Some('✓') => Style::new().fg(t.installed).patch(BOLD),
                Some('✗') => Style::new().fg(t.archived).patch(BOLD),
                _ => Style::new(),
            },
        );
        return;
    }

    // Grouped as act | find | app. Hints that don't fit are dropped whole
    // from the end, but "? help" always stays, since it lists them all.
    const HINTS: [&[(&str, &str)]; 3] = [
        &[("enter", "install/uninstall"), ("u", "upgrade"), ("o", "open")],
        &[
            ("/", "search"),
            ("s", "sort"),
            ("*", "stars"),
            ("L", "language"),
            ("i", "installed"),
            ("a", "installable"),
            ("c", "clear"),
        ],
        &[("r", "refresh"), ("t", "theme"), ("q", "quit")],
    ];
    const HELP: (&str, &str) = ("?", "help");
    // Same width either way, so the fitting maths below does not care which.
    let divider: &str = if powerline() { POWER_SEP } else { " │ " };
    const GAP: &str = "  ";
    let end = right.saturating_sub(1);
    let draw = |buf: &mut Buffer, x: u16, sep: &str, (key, label): (&str, &str)| {
        let lit = app.flash > 0 && hint_is_for(key, &app.last_key);
        let (key_style, label_style) = match lit {
            true => (t.flash(flash_level(app.flash)), t.flash(flash_level(app.flash))),
            false => (t.accent().patch(BOLD), t.dim()),
        };
        let x = buf.set_stringn(x, area.y, sep, end.saturating_sub(x) as usize, t.dim()).0;
        let x = buf.set_stringn(x, area.y, key, end.saturating_sub(x) as usize, key_style).0;
        buf.set_stringn(x, area.y, format!(" {label}"), end.saturating_sub(x) as usize, label_style).0
    };
    let width =
        |sep: &str, (key, label): (&str, &str)| (sep.chars().count() + key.len() + 1 + label.len()) as u16;
    let help_end = end.saturating_sub(width(divider, HELP));
    let mut x = area.x + 1;
    'groups: for (g, group) in HINTS.iter().enumerate() {
        for (i, &hint) in group.iter().enumerate() {
            let sep = match (g, i) {
                (0, 0) => "",
                (_, 0) => divider,
                _ => GAP,
            };
            if x + width(sep, hint) > help_end {
                break 'groups;
            }
            x = draw(buf, x, sep, hint);
        }
    }
    draw(buf, x, if x == area.x + 1 { "" } else { divider }, HELP);
}

/// How lit the hint is with `left` ticks to go: eases up to full over the
/// first few ticks, holds, then eases back down.
fn flash_level(left: u8) -> f32 {
    const FADE: u8 = 4;
    let x = ((FLASH_TICKS - left + 1).min(left) as f32 / FADE as f32).min(1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Whether a hint's key such as "s" or "enter" is for the echoed key, which
/// may carry a count ("3s") and is spelt "Enter" by crossterm.
fn hint_is_for(key: &str, pressed: &str) -> bool {
    let pressed = pressed.trim_start_matches(|c: char| c.is_ascii_digit());
    match key.len() {
        1 => pressed == key,
        _ => pressed.eq_ignore_ascii_case(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Event;
    use crate::installed::tests::{catalog, detected};
    use crate::installed::Installed;
    use crate::managers::by_name;
    use crate::theme::THEMES;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn app() -> App {
        let c = catalog();
        let mut installed = Installed::new(detected(&["brew", "cargo"]), &c);
        installed.set_listing(by_name("brew").unwrap(), vec!["btop".into()], &c);
        App::new(c, installed, 20_717)
    }

    fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let (page, sidebar) = layout(Rect::new(0, 0, width, height));
        app.set_layout(page, sidebar);
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buf = terminal.backend().buffer();
        (0..height).map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect()).collect()
    }

    fn press(app: &mut App, c: char) {
        app.update(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    }

    fn esc(app: &mut App) {
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    }

    #[test]
    fn wide_layout_shows_every_pane() {
        let mut app = app();
        let screen = render(&mut app, 120, 30);
        let all = screen.join("\n");
        assert!(screen[1].contains("Categories") && screen[1].contains("5/6 · sort:stars"), "{}", screen[1]);
        assert!(screen[2].contains("NAME") && screen[2].contains("★ ▾") && screen[2].contains("LANGUAGE"));
        assert!(screen[3].contains("lazygit") && screen[3].contains("50k") && screen[3].contains("Go"));
        assert!(
            screen[3].contains(" 1 lazygit") && screen[4].contains(" 2 btop"),
            "line numbers: {}",
            screen[4]
        );
        assert!(screen[4].contains("✓") && screen[4].contains("btop"), "installed mark: {}", screen[4]);
        assert!(all.contains("https://github.com/o/lazygit") && all.contains("brew:lazygit"));
        assert!(all.contains("Dashboards") && all.contains("/ search"));
        assert!(!all.contains("oldtool"), "archived rows are hidden by default");

        press(&mut app, 's');
        let screen = render(&mut app, 120, 30);
        assert!(screen[2].contains("PUSH ▾") && !screen[2].contains("★ ▾"), "{}", screen[2]);
    }

    #[test]
    fn the_selection_bar_and_border_sweep_across() {
        let mut app = app();
        app.theme = crate::theme::by_name("nord");
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let row = Rect::new(0, 0, 120, 30);
        let table = areas(row).table;
        let bar = table.y + 2;
        let (left, right) = (buf[(table.x + 1, bar)].bg, buf[(table.right() - 2, bar)].bg);
        assert_ne!(left, right, "the selection bar sweeps");
        assert_eq!(left, THEMES[app.theme].accent, "it starts at the accent");
        let border = (buf[(table.x, table.y)].fg, buf[(table.right() - 1, table.bottom() - 1)].fg);
        assert_eq!(border.0, THEMES[app.theme].accent);
        assert_eq!(border.1, THEMES[app.theme].link, "and the border ends at the link colour");
    }

    #[test]
    fn narrow_layout_drops_sidebar_and_columns() {
        let mut app = app();
        let screen = render(&mut app, 60, 12);
        let all = screen.join("\n");
        assert!(!all.contains("Categories") && !all.contains("LANGUAGE"));
        assert!(all.contains("lazygit") && all.contains("DESCRIPTION"));
        app.update(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        let screen = render(&mut app, 60, 12);
        assert!(screen[1].contains("cat:Dashboards"), "{}", screen[1]);
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = app();
        for (w, h) in [(0, 0), (1, 1), (5, 2), (10, 3), (20, 5), (60, 3), (64, 4), (89, 17), (90, 18)] {
            render(&mut app, w, h);
            press(&mut app, '?');
            render(&mut app, w, h);
            // A search matching nothing, which draws its own "no matches" line.
            press(&mut app, '/');
            press(&mut app, 'z');
            press(&mut app, 'z');
            render(&mut app, w, h);
            esc(&mut app);
            press(&mut app, 'q');
            press(&mut app, '*');
            render(&mut app, w, h);
            press(&mut app, 'q');
            press(&mut app, 't');
            press(&mut app, '/');
            render(&mut app, w, h);
            press(&mut app, 'z');
            press(&mut app, 'z');
            render(&mut app, w, h);
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            // The job log, empty, which draws its own placeholder line.
            press(&mut app, 'v');
            render(&mut app, w, h);
            esc(&mut app);
        }
    }

    #[test]
    fn pending_count_shows_in_the_status_bar() {
        let mut app = app();
        press(&mut app, '1');
        press(&mut app, '2');
        let status = render(&mut app, 120, 40).pop().unwrap();
        assert!(status.trim_end().ends_with(" 12"), "{status}");
        press(&mut app, 'j');
        let status = render(&mut app, 120, 40).pop().unwrap();
        assert!(status.trim_end().ends_with(" 12j"), "the motion is echoed with its count: {status}");
    }

    #[test]
    fn flash_fades_in_holds_and_fades_out() {
        let levels: Vec<f32> = (1..=FLASH_TICKS).rev().map(flash_level).collect();
        assert!(levels[0] > 0.0, "visible on the first frame");
        assert!(levels.windows(2).take(3).all(|w| w[0] < w[1]), "{levels:?}");
        assert_eq!(levels[FLASH_TICKS as usize / 2], 1.0);
        assert!(levels.windows(2).rev().take(3).all(|w| w[0] > w[1]), "{levels:?}");
    }

    #[test]
    fn hints_match_the_pressed_key() {
        assert!(hint_is_for("s", "s"));
        assert!(hint_is_for("s", "3s"));
        assert!(!hint_is_for("L", "l"), "case matters for letters");
        assert!(hint_is_for("enter", "Enter"));
        assert!(!hint_is_for("s", "^s"));
        assert!(!hint_is_for("s", ""));
    }

    #[test]
    fn narrow_status_drops_whole_hints_but_keeps_help() {
        let mut app = app();
        let status = render(&mut app, 60, 20).pop().unwrap();
        assert!(status.contains("? help"), "{status}");
        assert!(status.contains("enter install/uninstall"), "{status}");
        assert!(!status.contains("q quit"), "{status}");
        let wide = render(&mut app, 200, 20).pop().unwrap();
        assert!(wide.contains("q quit │ ? help"), "{wide}");
    }

    #[test]
    fn typed_search_text_is_not_echoed() {
        let mut app = app();
        press(&mut app, '/');
        press(&mut app, 'x');
        let status = render(&mut app, 120, 40).pop().unwrap();
        assert!(!status.trim_end().ends_with('x'), "{status}");
    }

    #[test]
    fn theme_picker_has_a_search_bar_like_help() {
        let mut app = app();
        press(&mut app, 't');
        let all = render(&mut app, 120, 40).join("\n");
        assert!(all.contains("Press / to search themes") && all.contains("gruvbox"), "{all}");
        assert!(all.contains("✓ default"), "the saved theme is marked: {all}");
        app.update(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        let all = render(&mut app, 120, 40).join("\n");
        assert!(all.contains("✓ default") && !all.contains("✓ gruvbox"), "moving only previews: {all}");
        press(&mut app, '/');
        for c in "zz".chars() {
            press(&mut app, c);
        }
        let all = render(&mut app, 120, 40).join("\n");
        assert!(
            all.contains("/ zz") && all.contains("No matching themes") && all.contains("esc clears"),
            "{all}"
        );
    }

    #[test]
    fn search_and_pickers_are_visible() {
        let mut app = app();
        press(&mut app, '/');
        press(&mut app, 'b');
        press(&mut app, 't');
        let screen = render(&mut app, 100, 24);
        assert!(screen[1].contains("/bt"), "{}", screen[1]);
        assert!(!screen.join("\n").contains("lazygit description"));

        app.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        press(&mut app, 'j');
        app.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let all = render(&mut app, 100, 24).join("\n");
        assert!(all.contains("Install bottom?") && all.contains("cargo install --locked bottom"), "{all}");

        app.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let screen = render(&mut app, 100, 24);
        let row = screen.iter().find(|l| l.contains("bottom description")).unwrap();
        assert!(row.contains(SPINNER[0]), "running row spins: {row}");
        assert!(screen[23].contains("install bottom (cargo)"), "{}", screen[23]);
        app.update(Event::JobDone { ok: false });
        let screen = render(&mut app, 100, 24);
        assert!(screen[23].contains("✗ install bottom (cargo) failed"), "{}", screen[23]);
    }

    #[test]
    fn help_search_narrows_the_list() {
        let mut app = app();
        press(&mut app, '?');
        assert!(render(&mut app, 100, 30).join("\n").contains("Press / to search keys"));
        press(&mut app, '/');
        "theme".chars().for_each(|c| press(&mut app, c));
        let all = render(&mut app, 100, 30).join("\n");
        assert!(all.contains("/ theme") && all.contains("colour theme"), "{all}");
        assert!(!all.contains("half page"), "{all}");
    }

    #[test]
    fn empty_catalog_explains_itself() {
        let empty = tuiman_index::Builder::new(0).finish();
        let mut app = App::new(empty, Installed::default(), 0);
        assert!(render(&mut app, 100, 24).join("\n").contains("press r to download"));
        app.refreshing = true;
        assert!(render(&mut app, 100, 24).join("\n").contains("Downloading the index"));
    }
}
