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
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tuiman_index::Row;

use crate::app::{App, Mode};
use crate::managers::MANAGERS;
use crate::query::Sort;
use crate::theme::Theme;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

pub struct Areas {
    pub sidebar: Rect,
    pub table: Rect,
    pub details: Rect,
    pub status: Rect,
}

/// Hand-rolled layout: four rectangles do not need a constraint solver.
pub fn areas(area: Rect) -> Areas {
    let status_h = area.height.min(1);
    let body_h = area.height - status_h;
    let details_h = if body_h >= 18 { 8 } else { 0 };
    let sidebar_w = if area.width >= 90 { 24 } else { 0 };
    let top_h = body_h - details_h;
    Areas {
        sidebar: Rect::new(area.x, area.y, sidebar_w, top_h),
        table: Rect::new(area.x + sidebar_w, area.y, area.width - sidebar_w, top_h),
        details: Rect::new(area.x, area.y + top_h, area.width, details_h),
        status: Rect::new(area.x, area.y + body_h, area.width, status_h),
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
        Mode::Picker(picker) => overlay::picker(buf, area, picker, app.theme()),
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
    }
}

/// Column x-offsets and widths for a given inner table width.
struct Columns {
    name: (u16, u16),
    stars: (u16, u16),
    language: Option<(u16, u16)>,
    age: Option<(u16, u16)>,
    desc: (u16, u16),
}

impl Columns {
    fn new(width: u16) -> Columns {
        let wide = width >= 72;
        let name_w = if wide { 24 } else { 18 }.min(width.saturating_sub(10));
        let mut x = 2;
        let mut next = |w: u16| {
            let col = (x, w);
            x += w + 1;
            col
        };
        let name = next(name_w);
        let stars = next(6);
        let language = wide.then(|| next(11));
        let age = wide.then(|| next(6));
        let desc = (x, width.saturating_sub(x));
        Columns { name, stars, language, age, desc }
    }
}

fn table(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme();
    let title = match (&app.mode, app.query.text.is_empty()) {
        (Mode::Search, _) | (_, false) => {
            Line::from(vec![" /".into(), Span::styled(app.query.text.as_str(), BOLD), " ".into()])
        }
        _ => Line::from(" tuiman "),
    };
    let block = panel(t, !app.sidebar_focused).title_top(title).title_top(filters(app).right_aligned());
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 2 || inner.width < 12 {
        return;
    }

    let cols = Columns::new(inner.width);
    let header = |label: &str, sort: Option<Sort>| match sort == Some(app.query.sort) {
        true => format!("{label}▾"),
        false => label.to_owned(),
    };
    let put = |buf: &mut Buffer, y, (x, w): (u16, u16), text: &str, style| {
        buf.set_stringn(inner.x + x, y, text, w as usize, style);
    };
    let put_right = |buf: &mut Buffer, y, (x, w): (u16, u16), text: &str, style| {
        let pad = w.saturating_sub(text.chars().count() as u16);
        buf.set_stringn(inner.x + x + pad, y, text, (w - pad) as usize, style);
    };

    put(buf, inner.y, cols.name, &header("NAME", Some(Sort::Name)), t.dim());
    put_right(buf, inner.y, cols.stars, &header("★", Some(Sort::Stars)), t.dim());
    if let (Some(language), Some(age)) = (cols.language, cols.age) {
        put(buf, inner.y, language, "LANGUAGE", t.dim());
        put_right(buf, inner.y, age, &header("PUSH", Some(Sort::Updated)), t.dim());
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

    let body = (inner.y + 1..inner.bottom()).zip(app.view.rows.iter().enumerate().skip(app.offset));
    for (y, (i, &row)) in body {
        let cat = &app.catalog;
        let selected = i == app.selected;
        let faded = cat.is_archived(row);
        let text = if faded { t.dim() } else { Style::new() };

        // A job on this row outranks the installed mark: spinning while it runs, ⋯ while queued.
        if app.running.as_ref().is_some_and(|job| job.row == row) {
            put(buf, y, (0, 1), SPINNER[app.spinner % SPINNER.len()], t.accent().patch(BOLD));
        } else if app.queue.iter().any(|job| job.row == row) {
            put(buf, y, (0, 1), "⋯", t.accent());
        } else if app.installed.is_installed(row) {
            put(buf, y, (0, 1), "✓", Style::new().fg(t.installed));
        }
        put(buf, y, cols.name, cat.name(row), text.patch(BOLD));
        put_right(
            buf,
            y,
            cols.stars,
            &format::stars(cat.stars(row)),
            if faded { t.dim() } else { Style::new().fg(t.stars) },
        );
        if let (Some(language), Some(age)) = (cols.language, cols.age) {
            put(buf, y, language, cat.language(row), t.dim());
            put_right(buf, y, age, &format::age(cat.pushed_days(row), app.today_days), t.dim());
        }
        put(buf, y, cols.desc, cat.desc(row), text);
        if selected {
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), cursor(t, !app.sidebar_focused));
        }
    }
}

/// A focusable panel: thick accent border with a bold title when focused, thin and dim otherwise.
fn panel(t: &Theme, focused: bool) -> Block<'static> {
    match focused {
        true => Block::bordered()
            .border_type(BorderType::Thick)
            .border_style(t.accent())
            .title_style(t.accent().patch(BOLD)),
        false => Block::bordered().border_style(t.dim()).title_style(t.dim()),
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
    let block = Block::bordered().border_style(t.dim());
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
    const HINTS: &str =
        " / search · s sort · * stars · L language · i installed · a installable · enter install/uninstall · o open · t theme · ? help";
    match app.status.is_empty() {
        true => buf.set_stringn(area.x, area.y, HINTS, area.width as usize, t.dim()),
        false => buf.set_stringn(
            area.x + 1,
            area.y,
            &app.status,
            area.width.saturating_sub(1) as usize,
            match app.status.chars().next() {
                Some('✓') => Style::new().fg(t.installed).patch(BOLD),
                Some('✗') => Style::new().fg(t.archived).patch(BOLD),
                _ => Style::new(),
            },
        ),
    };

    let scanning = if app.detecting { 1 } else { app.scans_pending };
    let activity = match (&app.running, app.refreshing, scanning) {
        (Some(job), _, _) if app.queue.is_empty() => job.title.clone(),
        (Some(job), _, _) => format!("{} (+{} queued)", job.title, app.queue.len()),
        (None, true, _) => "refreshing index".to_owned(),
        (None, false, 1..) => "scanning installed".to_owned(),
        (None, false, 0) => return,
    };
    let text = format!(" {} {activity} ", SPINNER[app.spinner % SPINNER.len()]);
    let width = (text.chars().count() as u16).min(area.width);
    // A running job gets a solid badge; background scans stay a quiet spinner.
    let style = if app.running.is_some() { t.selected().patch(BOLD) } else { t.accent() };
    buf.set_stringn(area.right() - width, area.y, &text, width as usize, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Event;
    use crate::installed::tests::{catalog, detected};
    use crate::installed::Installed;
    use crate::managers::by_name;
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

    #[test]
    fn wide_layout_shows_every_pane() {
        let mut app = app();
        let screen = render(&mut app, 120, 30);
        let all = screen.join("\n");
        assert!(screen[0].contains("Categories") && screen[0].contains("5/6 · sort:stars"), "{}", screen[0]);
        assert!(screen[1].contains("NAME") && screen[1].contains("★▾") && screen[1].contains("LANGUAGE"));
        assert!(screen[2].contains("lazygit") && screen[2].contains("50k") && screen[2].contains("Go"));
        assert!(screen[3].contains("✓") && screen[3].contains("btop"), "installed mark: {}", screen[3]);
        assert!(all.contains("https://github.com/o/lazygit") && all.contains("brew:lazygit"));
        assert!(all.contains("Dashboards") && all.contains("/ search"));
        assert!(!all.contains("oldtool"), "archived rows are hidden by default");
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
        assert!(screen[0].contains("cat:Dashboards"), "{}", screen[0]);
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = app();
        for (w, h) in [(0, 0), (1, 1), (5, 2), (10, 3), (20, 5), (89, 17), (90, 18)] {
            render(&mut app, w, h);
            press(&mut app, '?');
            render(&mut app, w, h);
            press(&mut app, 'q');
            press(&mut app, '*');
            render(&mut app, w, h);
            press(&mut app, 'q');
        }
    }

    #[test]
    fn search_and_pickers_are_visible() {
        let mut app = app();
        press(&mut app, '/');
        press(&mut app, 'b');
        press(&mut app, 't');
        let screen = render(&mut app, 100, 24);
        assert!(screen[0].contains("/bt"), "{}", screen[0]);
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
