//! Application state and its single transition function.
//!
//! `App::update` is pure with respect to the outside world: it mutates state
//! and returns [`Effect`]s for the shell in `main.rs` to carry out. No
//! terminal, process or network access happens here, so every interaction is
//! unit-testable.

use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tuiman_index::{Catalog, Row};

use crate::installed::{self, Choice, Installed};
use crate::managers::{Action, ManagerId, MANAGERS};
use crate::query::{Query, View};
use crate::theme::{Theme, THEMES};

const LOG_LINES: usize = 2000;
/// How long the hint for a pressed key stays lit: about a second of 80 ms ticks.
pub const FLASH_TICKS: u8 = 14;
pub const STAR_PRESETS: [u32; 7] = [0, 100, 500, 1_000, 5_000, 10_000, 50_000];

pub enum Event {
    Key(KeyEvent),
    Resize,
    /// The `PATH` walk for package managers finished.
    Detected(Vec<(ManagerId, std::path::PathBuf)>),
    /// The listing of every `PATH` directory finished.
    OnPath(std::collections::HashSet<String>),
    /// A manager's "what is installed" scan finished.
    Listing(ManagerId, Vec<String>),
    Index(IndexUpdate),
    JobOutput(String),
    JobDone {
        ok: bool,
    },
    /// Drives the spinner; only sent while something is in flight.
    Tick,
}

pub enum IndexUpdate {
    Fresh(Box<Catalog>),
    NotModified,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Quit,
    RefreshIndex,
    /// Rescan one manager, or all detected ones.
    Scan(Option<ManagerId>),
    /// Run in the background with captured output.
    Spawn(Job),
    /// Hand the terminal to the command (sudo prompts and the like).
    RunInTerminal(Job),
    OpenUrl(String),
    Copy(String),
    SaveInstalled,
    /// Remember the theme called this for the next start.
    SaveTheme(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub action: Action,
    pub manager: ManagerId,
    /// The TUI being installed or removed, marked in the list while the job is pending.
    pub row: Row,
    pub title: String,
    pub argv: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    /// `/` in the sidebar: jumps to the first category containing `text`; esc restores `original`.
    CategorySearch {
        text: String,
        original: Option<u8>,
    },
    Picker(Picker),
    Help(Help),
    Log,
}

/// The key list overlay.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Help {
    /// `Some` once `/` is pressed; the text narrows the list.
    pub filter: Option<String>,
    /// Index into the narrowed list.
    pub selected: usize,
}

pub const HELP: [(&str, &str); 24] = [
    ("h l ← →", "focus categories / list"),
    ("j k ↓ ↑", "move in the focused panel"),
    ("gg G", "first / last, 3gg row 3"),
    ("1-9", "count for the next move, as in 3j"),
    ("ctrl-d ctrl-u", "half page down / up"),
    (") (", "half page down / up, in any list"),
    ("tab shift-tab", "next / previous category"),
    ("/", "fuzzy search, or find a category in the sidebar"),
    ("s", "cycle sort: stars, last push, name"),
    ("*", "minimum stars"),
    ("L", "language"),
    ("i", "installed only"),
    ("a", "installable on this machine only"),
    ("A", "show archived projects"),
    ("enter", "install, or uninstall if installed"),
    ("u", "upgrade an installed TUI"),
    ("o", "open the project page"),
    ("y", "copy the selected item"),
    ("r", "refresh the index"),
    ("v", "view job output"),
    ("t", "colour theme"),
    ("?", "this help"),
    ("qq", "quit"),
    ("c", "clear all filters"),
];

impl Help {
    /// [`HELP`] rows whose keys or description contain the filter.
    pub fn rows(&self) -> impl Iterator<Item = &'static (&'static str, &'static str)> + '_ {
        let needle = self.filter.as_deref().unwrap_or("");
        HELP.iter().filter(move |(keys, what)| keys.contains(needle) || what.contains(needle))
    }
}

/// A modal list. What a selection means depends on `kind`.
#[derive(Debug, PartialEq, Eq)]
pub struct Picker {
    pub title: String,
    pub items: Vec<String>,
    pub selected: usize,
    pub kind: PickerKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PickerKind {
    Confirm {
        action: Action,
        row: Row,
        choices: Vec<Choice>,
    },
    MinStars,
    /// Language ids parallel to `items`; `None` is "any".
    Language(Vec<Option<u16>>),
    /// Moving previews, `original` is restored on cancel. `ids` index
    /// [`THEMES`] parallel to `items`, narrowed by `filter` once `/` opened it.
    Theme {
        original: usize,
        filter: Option<String>,
        ids: Vec<usize>,
    },
}

pub struct App {
    pub catalog: Catalog,
    pub installed: Installed,
    pub query: Query,
    pub view: View,
    /// Index into `view.rows`.
    pub selected: usize,
    /// First visible table line; maintained by `scroll_into_view`.
    pub offset: usize,
    pub mode: Mode,
    pub status: String,
    pub log: VecDeque<String>,
    pub running: Option<Job>,
    pub queue: VecDeque<Job>,
    pub refreshing: bool,
    /// The refresh in flight was not asked for, so only news is worth a message.
    quiet_refresh: bool,
    pub scans_pending: usize,
    /// The startup `PATH` walk is still running.
    pub detecting: bool,
    pub spinner: usize,
    pub today_days: u32,
    /// Height of the table body in lines, reported by the renderer's layout.
    pub page: usize,
    pub dirty: bool,
    /// Index into [`THEMES`].
    pub theme: usize,
    /// ←/→ move focus; ↑/↓ then move through categories instead of rows.
    pub sidebar_focused: bool,
    /// The layout has room for the sidebar; reported by the shell.
    pub sidebar_visible: bool,
    pub quit_armed: bool,
    /// Digits typed so far, repeating the next motion as in vim's `3j`.
    pub count: usize,
    /// The count before a first `g`, waiting for the second one of `gg`.
    g_pending: Option<usize>,
    /// The last command key with its count, echoed like vim's showcmd.
    pub last_key: String,
    /// Ticks left to light up `last_key`'s hint in the status bar.
    pub flash: u8,
}

impl App {
    pub fn new(catalog: Catalog, installed: Installed, today_days: u32) -> App {
        let mut app = App {
            catalog,
            installed,
            query: Query::default(),
            view: View::default(),
            selected: 0,
            offset: 0,
            mode: Mode::Normal,
            status: String::new(),
            log: VecDeque::new(),
            running: None,
            queue: VecDeque::new(),
            refreshing: false,
            quiet_refresh: false,
            scans_pending: 0,
            detecting: false,
            spinner: 0,
            today_days,
            page: 20,
            dirty: true,
            theme: 0,
            sidebar_focused: false,
            sidebar_visible: true,
            quit_armed: false,
            count: 0,
            g_pending: None,
            last_key: String::new(),
            flash: 0,
        };
        app.refilter(false);
        app
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme]
    }

    pub fn selected_row(&self) -> Option<Row> {
        self.view.rows.get(self.selected).copied()
    }

    /// The shell started a background refresh the user did not ask for.
    pub fn begin_quiet_refresh(&mut self) {
        self.refreshing = true;
        self.quiet_refresh = true;
    }

    /// Whether something is in flight or lit, so ticks should keep coming.
    pub fn busy(&self) -> bool {
        self.running.is_some()
            || self.refreshing
            || self.detecting
            || self.scans_pending > 0
            || self.flash > 0
    }

    pub fn update(&mut self, event: Event) -> Vec<Effect> {
        self.dirty = true;
        match event {
            Event::Key(key) => {
                let effects = self.on_key(key);
                // With a search box open the lit hint is hidden or replaced, so its
                // animation would only redraw, and each redraw restarts the cursor's blink.
                if !self.takes_count() {
                    self.flash = 0;
                }
                return effects;
            }
            Event::Resize => {}
            Event::Tick => {
                self.spinner = self.spinner.wrapping_add(1);
                self.flash = self.flash.saturating_sub(1);
            }
            Event::Detected(detected) => {
                self.detecting = false;
                self.installed.set_detected(detected, &self.catalog);
                self.refilter(true);
                return self.rescan(None);
            }
            Event::OnPath(names) => {
                self.installed.set_path_names(names, &self.catalog);
                self.refilter(true);
            }
            Event::Listing(manager, names) => {
                self.scans_pending = self.scans_pending.saturating_sub(1);
                self.installed.set_listing(manager, names, &self.catalog);
                self.refilter(true);
                if self.scans_pending == 0 {
                    return vec![Effect::SaveInstalled];
                }
            }
            Event::Index(update) => {
                self.refreshing = false;
                let quiet = std::mem::take(&mut self.quiet_refresh) && !self.catalog.is_empty();
                match update {
                    IndexUpdate::Fresh(catalog) => {
                        self.catalog = *catalog;
                        self.installed.rebuild(&self.catalog);
                        // Ids are only meaningful within one catalog.
                        self.query.category = None;
                        self.query.language = None;
                        // So are the rows and ids an open dialog holds, which could now name
                        // another TUI or none at all.
                        if matches!(
                            self.mode,
                            Mode::CategorySearch { .. }
                                | Mode::Picker(Picker {
                                    kind: PickerKind::Confirm { .. } | PickerKind::Language(_),
                                    ..
                                })
                        ) {
                            self.mode = Mode::Normal;
                        }
                        self.refilter(false);
                        self.status = format!("Index updated: {} TUIs", self.catalog.len());
                    }
                    IndexUpdate::NotModified | IndexUpdate::Failed(_) if quiet => {}
                    IndexUpdate::NotModified => self.status = "Index is up to date".into(),
                    IndexUpdate::Failed(why) => self.status = format!("Index refresh failed: {why}"),
                }
            }
            Event::JobOutput(line) => self.push_log(line),
            Event::JobDone { ok } => return self.on_job_done(ok),
        }
        Vec::new()
    }

    fn on_job_done(&mut self, ok: bool) -> Vec<Effect> {
        let Some(job) = self.running.take() else { return Vec::new() };
        self.status = match ok {
            true => format!("✓ {} finished", job.title),
            false => format!("✗ {} failed (press v for the log)", job.title),
        };
        let mut effects = self.rescan(Some(job.manager));
        if let Some(next) = self.queue.pop_front() {
            effects.push(self.start(next));
        }
        effects
    }

    /// Rescans one manager, or every detected one. The pending count is taken
    /// from what the shell will actually start: a manager that disappeared
    /// between confirming a job and its end would otherwise be waited on forever.
    fn rescan(&mut self, only: Option<ManagerId>) -> Vec<Effect> {
        let n = self.installed.detected().iter().filter(|(id, _)| only.is_none_or(|o| o == *id)).count();
        self.scans_pending += n;
        match n {
            0 if self.scans_pending == 0 => vec![Effect::SaveInstalled],
            0 => Vec::new(),
            _ => vec![Effect::Scan(only)],
        }
    }

    fn start(&mut self, job: Job) -> Effect {
        self.push_log(format!("$ {}", job.argv.join(" ")));
        self.running = Some(job.clone());
        Effect::Spawn(job)
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Re-runs the query. `keep_row` keeps the cursor on the same TUI when it
    /// is still visible; otherwise the cursor returns to the best match.
    fn refilter(&mut self, keep_row: bool) {
        let previous = self.selected_row().filter(|_| keep_row);
        self.view.run(&self.catalog, &self.installed, &self.query);
        self.selected = previous.and_then(|row| self.view.rows.iter().position(|&r| r == row)).unwrap_or(0);
        self.scroll_into_view();
    }

    fn scroll_into_view(&mut self) {
        let page = self.page.max(1);
        self.selected = self.selected.min(self.view.rows.len().saturating_sub(1));
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + page {
            self.offset = self.selected + 1 - page;
        }
        self.offset = self.offset.min(self.view.rows.len().saturating_sub(page));
    }

    fn move_by(&mut self, delta: isize) {
        self.selected = self.selected.saturating_add_signed(delta);
        self.scroll_into_view();
    }

    /// A line up or down, wrapping around at the ends.
    fn step_rows(&mut self, delta: isize) {
        self.selected = wrap_step(self.selected, delta, self.view.rows.len());
        self.scroll_into_view();
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return vec![Effect::Quit];
        }
        let armed = std::mem::take(&mut self.quit_armed);
        let mut count = std::mem::take(&mut self.count);
        let g_pending = self.g_pending.take();
        // A leading 0 is not a count, as in vim.
        if let Some(digit) = match key.code {
            KeyCode::Char(c) if !ctrl && self.takes_count() => c.to_digit(10).filter(|&d| d > 0 || count > 0),
            _ => None,
        } {
            self.count = (count * 10 + digit as usize).min(99_999);
            self.last_key.clear();
            return Vec::new();
        }
        // `g` waits for a second one: `gg` goes to the top, `3gg` to row 3.
        if key.code == KeyCode::Char('g') && !ctrl && self.takes_count() {
            match g_pending {
                Some(pending) => count = pending,
                None => {
                    self.g_pending = Some(count);
                    self.last_key = format!("{}g", if count > 0 { count.to_string() } else { String::new() });
                    return Vec::new();
                }
            }
        }
        // Text typed into a search box is already on screen, so only commands are echoed.
        self.last_key = match self.takes_count() {
            true => format!(
                "{}{}{}{}",
                if count > 0 { count.to_string() } else { String::new() },
                if ctrl { "^" } else { "" },
                if g_pending.is_some() { "g" } else { "" },
                key.code
            ),
            false => String::new(),
        };
        match self.mode {
            Mode::Normal => self.on_normal_key(key.code, ctrl, armed, count),
            Mode::Search => {
                self.on_search_key(key.code, ctrl);
                Vec::new()
            }
            Mode::CategorySearch { .. } => {
                self.on_category_search_key(key.code, ctrl);
                Vec::new()
            }
            Mode::Picker(_) => self.on_picker_key(key.code, ctrl, count),
            Mode::Help(_) => self.on_help_key(key.code, ctrl, count),
            Mode::Log if key.code == KeyCode::Char('y') => {
                vec![Effect::Copy(self.log.iter().map(String::as_str).collect::<Vec<_>>().join("\n"))]
            }
            Mode::Log => {
                self.mode = Mode::Normal;
                Vec::new()
            }
        }
    }

    /// Digits are a count wherever they are not being typed into a search.
    fn takes_count(&self) -> bool {
        match &self.mode {
            Mode::Normal => true,
            Mode::Help(help) => help.filter.is_none(),
            Mode::Picker(Picker { kind: PickerKind::Theme { filter, .. }, .. }) => filter.is_none(),
            Mode::Picker(_) => true,
            _ => false,
        }
    }

    fn on_normal_key(&mut self, code: KeyCode, ctrl: bool, quit_armed: bool, count: usize) -> Vec<Effect> {
        self.status.clear();
        self.flash = FLASH_TICKS;
        let n = count.max(1) as isize;
        let half_page = (self.page / 2).max(1) as isize;
        if self.sidebar_focused {
            let categories = self.catalog.category_count() as isize + 1;
            let step = match code {
                KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up => {
                    let step = if matches!(code, KeyCode::Char('j') | KeyCode::Down) { n } else { -n };
                    let current = self.query.category.map_or(0, |c| c as usize + 1);
                    wrap_step(current, step, categories as usize) as isize - current as isize
                }
                KeyCode::Char(')') => n * half_page,
                KeyCode::Char('(') => -n * half_page,
                // Row 1 is All, so 3gg lands on the second category.
                // Returns rather than steps, as already being there is a step of 0.
                KeyCode::Char('g') if count > 0 => {
                    self.step_category(
                        count as isize - 1 - self.query.category.map_or(0, |c| c as isize + 1),
                        false,
                    );
                    return Vec::new();
                }
                KeyCode::Char('g') | KeyCode::Home => -categories,
                KeyCode::Char('G') | KeyCode::End => categories,
                KeyCode::Enter => {
                    self.sidebar_focused = false;
                    return Vec::new();
                }
                KeyCode::Char('/') => {
                    self.mode = Mode::CategorySearch { text: String::new(), original: self.query.category };
                    return Vec::new();
                }
                KeyCode::Char('y') => {
                    let name = self.query.category.map_or("All", |id| self.catalog.category_name(id));
                    return vec![Effect::Copy(name.to_owned())];
                }
                _ => 0,
            };
            if step != 0 {
                self.step_category(step, false);
                return Vec::new();
            }
        }
        match code {
            KeyCode::Char('q') => {
                if quit_armed {
                    return vec![Effect::Quit];
                }
                self.quit_armed = true;
                self.status = match self.running {
                    Some(_) => "A job is still running · q again quits anyway",
                    None => "q again quits",
                }
                .into();
            }
            KeyCode::Char(')') => self.move_by(n * half_page),
            KeyCode::Char('(') => self.move_by(-n * half_page),
            KeyCode::Char('d') if ctrl => self.move_by(n * half_page),
            KeyCode::Char('u') if ctrl => self.move_by(-n * half_page),
            KeyCode::Char('j') | KeyCode::Down => self.step_rows(n),
            KeyCode::Char('k') | KeyCode::Up => self.step_rows(-n),
            KeyCode::PageDown => self.move_by(n * self.page as isize),
            KeyCode::PageUp => self.move_by(-n * self.page as isize),
            KeyCode::Char('g') if count > 0 => self.move_by(count as isize - 1 - self.selected as isize),
            KeyCode::Char('g') | KeyCode::Home => self.move_by(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_by(isize::MAX),
            KeyCode::Char('h') | KeyCode::Left => self.sidebar_focused = self.sidebar_visible,
            KeyCode::Char('l') | KeyCode::Right => self.sidebar_focused = false,
            KeyCode::Tab => self.step_category(1, true),
            KeyCode::BackTab => self.step_category(-1, true),
            KeyCode::Char('/') => {
                self.sidebar_focused = false;
                self.mode = Mode::Search;
            }
            KeyCode::Esc => {
                self.query.text.clear();
                self.refilter(true);
            }
            KeyCode::Char('c') => {
                self.query = Query { sort: self.query.sort, ..Query::default() };
                self.refilter(true);
            }
            KeyCode::Char('s') => {
                self.query.sort = self.query.sort.next();
                // A new order is read from the top, not from wherever the old row went.
                self.refilter(false);
            }
            KeyCode::Char('i') => {
                self.query.installed_only ^= true;
                self.refilter(true);
            }
            KeyCode::Char('a') => {
                self.query.installable_only ^= true;
                self.refilter(true);
            }
            KeyCode::Char('A') => {
                self.query.show_archived ^= true;
                self.refilter(true);
            }
            KeyCode::Char('*') => self.open_stars_picker(),
            KeyCode::Char('L') => self.open_language_picker(),
            KeyCode::Enter => match self.selected_row().is_some_and(|row| self.installed.is_installed(row)) {
                true => self.open_confirm(Action::Uninstall),
                false => self.open_confirm(Action::Install),
            },
            KeyCode::Char('u') => self.open_confirm(Action::Upgrade),
            KeyCode::Char('o') => {
                if let Some(row) = self.selected_row() {
                    return vec![Effect::OpenUrl(self.catalog.url(row).to_owned())];
                }
            }
            KeyCode::Char('y') => {
                if let Some(row) = self.selected_row() {
                    return vec![Effect::Copy(self.catalog.name(row).to_owned())];
                }
            }
            KeyCode::Char('r') if !self.refreshing => {
                self.refreshing = true;
                self.status = "Refreshing index…".into();
                return vec![Effect::RefreshIndex];
            }
            KeyCode::Char('v') => self.mode = Mode::Log,
            KeyCode::Char('t') => self.mode = Mode::Picker(self.theme_picker(self.theme, None)),
            KeyCode::Char('?') => self.mode = Mode::Help(Help::default()),
            _ => {}
        }
        Vec::new()
    }

    /// Arrows always move; typing edits the filter once `/` opened it.
    fn on_help_key(&mut self, code: KeyCode, ctrl: bool, count: usize) -> Vec<Effect> {
        let page = self.page;
        let Mode::Help(help) = &mut self.mode else { return Vec::new() };
        let len = help.rows().count();
        if let Some(to) = list_motion(code, help.filter.is_some(), help.selected, len, page, count) {
            help.selected = to;
            return Vec::new();
        }
        match (code, &mut help.filter) {
            (KeyCode::Char('y'), None) => {
                if let Some((keys, what)) = help.rows().nth(help.selected) {
                    return vec![Effect::Copy(format!("{keys}  {what}"))];
                }
            }
            (KeyCode::Left | KeyCode::Right, Some(_)) => {}
            (KeyCode::Char('/'), filter @ None) => *filter = Some(String::new()),
            (KeyCode::Esc, filter @ Some(_)) => {
                *filter = None;
                help.selected = 0;
            }
            // Closes the search, keeping the cursor on the entry it was on.
            (KeyCode::Enter, Some(_)) => {
                let entry = help.rows().nth(help.selected);
                help.filter = None;
                help.selected = entry.and_then(|e| HELP.iter().position(|h| h == e)).unwrap_or(0);
            }
            (_, Some(filter)) => match edit(filter, code, ctrl, 32) {
                true => help.selected = 0,
                false => self.mode = Mode::Normal,
            },
            _ => self.mode = Mode::Normal,
        }
        Vec::new()
    }

    fn on_search_key(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            // The query is unchanged, so the cursor stays where the arrows left it.
            KeyCode::Enter => return self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.query.text.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Down => return self.step_rows(1),
            KeyCode::Up => return self.step_rows(-1),
            _ if !edit(&mut self.query.text, code, ctrl, 64) => return,
            _ => {}
        }
        self.refilter(false);
    }

    fn on_category_search_key(&mut self, code: KeyCode, ctrl: bool) {
        let Mode::CategorySearch { text, original } = &mut self.mode else { return };
        let original = *original;
        match code {
            KeyCode::Enter => return self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.query.category = original;
                return self.refilter(true);
            }
            KeyCode::Down => return self.step_category(1, true),
            KeyCode::Up => return self.step_category(-1, true),
            _ => {}
        }
        if !edit(text, code, ctrl, 32) {
            return;
        }
        let needle = text.to_lowercase();
        let found = match needle.is_empty() {
            true => Some(original),
            false => (0..self.catalog.category_count() as u8)
                .find(|&id| self.catalog.category_name(id).to_lowercase().contains(&needle))
                .map(Some),
        };
        // No match keeps the last hit, so a typo does not throw the selection away.
        if let Some(category) = found.filter(|&c| c != self.query.category) {
            self.query.category = category;
            self.refilter(true);
        }
    }

    /// Called by the shell when the terminal is resized. Focus cannot stay on
    /// a sidebar the layout dropped, or keys would change categories unseen.
    pub fn set_layout(&mut self, page: usize, sidebar: bool) {
        self.sidebar_visible = sidebar;
        self.sidebar_focused &= sidebar;
        if self.page != page {
            self.page = page;
            self.scroll_into_view();
        }
    }

    /// Steps through "All" followed by each category, wrapping or stopping at the ends.
    fn step_category(&mut self, step: isize, wrap: bool) {
        let count = self.catalog.category_count() as isize + 1;
        let current = self.query.category.map_or(0, |c| c as isize + 1);
        let next =
            if wrap { (current + step).rem_euclid(count) } else { (current + step).clamp(0, count - 1) };
        if next == current {
            return;
        }
        self.query.category = (next > 0).then(|| (next - 1) as u8);
        self.refilter(true);
    }

    fn open_stars_picker(&mut self) {
        let items =
            STAR_PRESETS.iter().map(|&n| if n == 0 { "any".into() } else { format!("≥ {n}") }).collect();
        let selected = STAR_PRESETS.iter().rposition(|&n| n <= self.query.min_stars).unwrap_or(0);
        self.mode = Mode::Picker(Picker {
            title: "Minimum stars".into(),
            items,
            selected,
            kind: PickerKind::MinStars,
        });
    }

    /// Languages of the rows currently in view, most common first.
    fn open_language_picker(&mut self) {
        let mut counts = vec![0u32; self.catalog.language_count()];
        for &row in &self.view.rows {
            counts[self.catalog.language_id(row) as usize] += 1;
        }
        let mut ids: Vec<u16> = (0..counts.len() as u16)
            .filter(|&id| counts[id as usize] > 0 && !self.catalog.language_name(id).is_empty())
            .collect();
        ids.sort_by_key(|&id| std::cmp::Reverse(counts[id as usize]));

        let mut items = vec!["any".to_owned()];
        items.extend(
            ids.iter().map(|&id| format!("{} ({})", self.catalog.language_name(id), counts[id as usize])),
        );
        let ids: Vec<Option<u16>> = std::iter::once(None).chain(ids.into_iter().map(Some)).collect();
        let selected = ids.iter().position(|&id| id == self.query.language).unwrap_or(0);
        self.mode = Mode::Picker(Picker {
            title: "Language".into(),
            items,
            selected,
            kind: PickerKind::Language(ids),
        });
    }

    fn open_confirm(&mut self, action: Action) {
        let Some(row) = self.selected_row() else { return };
        let name = self.catalog.name(row);
        let choices = self.installed.choices(&self.catalog, row, action);
        if choices.is_empty() {
            let why = installed::impossible(&self.catalog, row, action, "press o to open its page");
            self.status = format!("✗ {why}");
            return;
        }
        let items =
            choices.iter().map(|c| MANAGERS[c.manager as usize].argv(action, &c.package).join(" ")).collect();
        let verb = match action {
            Action::Install => "Install",
            Action::Uninstall => "Uninstall",
            Action::Upgrade => "Upgrade",
        };
        let title = format!("{verb} {name}?");
        let kind = PickerKind::Confirm { action, row, choices };
        self.mode = Mode::Picker(Picker { title, items, selected: 0, kind });
    }

    /// Themes whose name contains `filter`, keeping the previewed one selected if it still matches.
    fn theme_picker(&self, original: usize, filter: Option<String>) -> Picker {
        let needle = filter.as_deref().unwrap_or_default().to_lowercase();
        let ids: Vec<usize> = (0..THEMES.len()).filter(|&i| THEMES[i].name.contains(&needle)).collect();
        Picker {
            title: "Theme".into(),
            items: ids.iter().map(|&i| THEMES[i].name.to_owned()).collect(),
            selected: ids.iter().position(|&i| i == self.theme).unwrap_or(0),
            kind: PickerKind::Theme { original, filter, ids },
        }
    }

    fn on_picker_key(&mut self, code: KeyCode, ctrl: bool, count: usize) -> Vec<Effect> {
        let Mode::Picker(picker) = &mut self.mode else { return Vec::new() };
        // `/` starts a theme search; while it is open, typing edits it, and esc
        // or enter close it on the theme being previewed.
        if let PickerKind::Theme { original, filter, .. } = &mut picker.kind {
            let edited = match (code, &mut *filter) {
                (KeyCode::Char('/'), None) => {
                    *filter = Some(String::new());
                    true
                }
                (KeyCode::Esc | KeyCode::Enter, Some(_)) => {
                    *filter = None;
                    true
                }
                (_, Some(text)) => edit(text, code, ctrl, 32),
                _ => false,
            };
            if edited {
                let (original, filter) = (*original, filter.clone());
                let picker = self.theme_picker(original, filter);
                if let PickerKind::Theme { ids, .. } = &picker.kind {
                    self.theme = ids.get(picker.selected).copied().unwrap_or(original);
                }
                self.mode = Mode::Picker(picker);
                return Vec::new();
            }
        }
        let page = self.page;
        let Mode::Picker(picker) = &mut self.mode else { return Vec::new() };
        match list_motion(code, false, picker.selected, picker.items.len(), page, count) {
            Some(to) => picker.selected = to,
            None => match code {
                // Only a confirmation reads y as yes; the other pickers copy what is selected.
                KeyCode::Char('y') if !matches!(picker.kind, PickerKind::Confirm { .. }) => {
                    let text = match &picker.kind {
                        PickerKind::Language(ids) if ids[picker.selected].is_some() => {
                            ids[picker.selected].map(|id| self.catalog.language_name(id))
                        }
                        // A theme search can match nothing.
                        _ => picker.items.get(picker.selected).map(String::as_str),
                    };
                    return text.map(|t| vec![Effect::Copy(t.to_owned())]).unwrap_or_default();
                }
                KeyCode::Enter | KeyCode::Char('y') => {
                    let Mode::Picker(picker) = std::mem::replace(&mut self.mode, Mode::Normal) else {
                        unreachable!()
                    };
                    return self.on_picked(picker);
                }
                KeyCode::Esc | KeyCode::Char('q' | 'n') => {
                    if let PickerKind::Theme { original, .. } = picker.kind {
                        self.theme = original;
                    }
                    self.mode = Mode::Normal;
                    return Vec::new();
                }
                _ => {}
            },
        }
        if let PickerKind::Theme { ids, .. } = &picker.kind {
            if let Some(&id) = ids.get(picker.selected) {
                self.theme = id;
            }
        }
        Vec::new()
    }

    fn on_picked(&mut self, picker: Picker) -> Vec<Effect> {
        match picker.kind {
            PickerKind::MinStars => {
                self.query.min_stars = STAR_PRESETS[picker.selected];
                self.refilter(true);
            }
            PickerKind::Language(ids) => {
                self.query.language = ids[picker.selected];
                self.refilter(true);
            }
            PickerKind::Theme { original, ids, .. } => {
                let Some(&id) = ids.get(picker.selected) else {
                    self.theme = original;
                    return Vec::new();
                };
                self.theme = id;
                self.status = format!("Theme: {}", self.theme().name);
                return vec![Effect::SaveTheme(self.theme().name)];
            }
            PickerKind::Confirm { action, row, choices } => {
                let choice = &choices[picker.selected];
                let manager = &MANAGERS[choice.manager as usize];
                let job = Job {
                    action,
                    manager: choice.manager,
                    row,
                    title: format!("{} {} ({})", action.verb(), self.catalog.name(row), manager.name),
                    argv: manager.argv(action, &choice.package),
                };
                if manager.tty {
                    return vec![Effect::RunInTerminal(job)];
                }
                if self.running.is_some() {
                    self.status = format!("Queued: {}", job.title);
                    self.queue.push_back(job);
                } else {
                    self.status = "Press v to watch the output".into();
                    return vec![self.start(job)];
                }
            }
        }
        Vec::new()
    }

    /// The shell reports the outcome of an `Effect::RunInTerminal` job.
    pub fn terminal_job_finished(&mut self, job: &Job, ok: bool) -> Vec<Effect> {
        self.dirty = true;
        self.status = match ok {
            true => format!("✓ {} finished", job.title),
            false => format!("✗ {} failed", job.title),
        };
        self.rescan(Some(job.manager))
    }
}

/// Moves in an overlay's list of `len` rows, shared by help and the pickers.
/// Returns the new selection, or `None` if the key is not a motion. While a
/// search is being typed only the non-letter keys move.
fn list_motion(
    code: KeyCode,
    typing: bool,
    selected: usize,
    len: usize,
    page: usize,
    count: usize,
) -> Option<usize> {
    let last = len.saturating_sub(1);
    let n = count.max(1);
    // Overlays grow to fit their list, so half a page is half of what is shown,
    // not half of the table behind them.
    let half_page = (len.min(page) / 2).max(1);
    let to = match code {
        KeyCode::Down => wrap_step(selected, n as isize, len),
        KeyCode::Up => wrap_step(selected, -(n as isize), len),
        KeyCode::Home | KeyCode::PageUp => 0,
        KeyCode::End | KeyCode::PageDown => last,
        _ if typing => return None,
        KeyCode::Char('j') => wrap_step(selected, n as isize, len),
        KeyCode::Char('k') => wrap_step(selected, -(n as isize), len),
        KeyCode::Char(')') => selected + n * half_page,
        KeyCode::Char('(') => selected.saturating_sub(n * half_page),
        KeyCode::Char('g') => count.saturating_sub(1),
        KeyCode::Char('G') => last,
        _ => return None,
    };
    Some(to.min(last))
}

/// Steps `delta` rows through a list of `len`, clamping at the ends, except
/// that stepping past an end the selection is already on wraps to the other.
fn wrap_step(selected: usize, delta: isize, len: usize) -> usize {
    let last = len.saturating_sub(1);
    match delta.signum() {
        -1 if selected == 0 => last,
        1 if selected >= last => 0,
        _ => selected.saturating_add_signed(delta).min(last),
    }
}

/// Line editing shared by every text field: backspace, ctrl-u, ctrl-w and
/// typing up to `max` bytes. Returns whether the key was an editing key, so
/// callers can tell a keystroke to swallow from a command of their own.
fn edit(text: &mut String, code: KeyCode, ctrl: bool, max: usize) -> bool {
    match code {
        KeyCode::Backspace => drop(text.pop()),
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Char('w') if ctrl => text.truncate(text.trim_end().rfind(' ').map_or(0, |i| i + 1)),
        KeyCode::Char(c) if !ctrl && text.len() < max => text.push(c),
        // Over the limit, or an unmapped ctrl chord: ignored, not a command.
        KeyCode::Char(_) => {}
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::tests::{catalog, detected};
    use crate::managers::by_name;

    fn app(managers: &[&str]) -> App {
        let c = catalog();
        let installed = Installed::new(detected(managers), &c);
        App::new(c, installed, 20_717)
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Effect> {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn type_text(app: &mut App, text: &str) {
        text.chars().for_each(|c| drop(press(app, KeyCode::Char(c))));
    }

    fn selected_name(app: &App) -> &str {
        app.catalog.name(app.selected_row().unwrap())
    }

    #[test]
    fn navigation_clamps_and_scrolls() {
        let mut app = app(&[]);
        app.set_layout(2, true);
        assert_eq!(selected_name(&app), "lazygit");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!((app.selected, app.offset), (4, 3), "k at the top wraps to the bottom");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!((app.selected, app.offset), (0, 0), "j at the bottom wraps to the top");
        press(&mut app, KeyCode::Char('G'));
        assert_eq!((app.selected, app.offset), (4, 3));
        type_text(&mut app, "gg");
        assert_eq!((app.selected, app.offset), (0, 0));

        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Char('s'));
        assert_eq!((app.selected, app.offset), (0, 0), "a new sort order starts at the top");
    }

    #[test]
    fn counts_repeat_motions() {
        let mut app = app(&[]);
        type_text(&mut app, "3j");
        assert_eq!(app.selected, 3);
        type_text(&mut app, "2kj");
        assert_eq!(app.selected, 2, "a count applies to one motion only");
        type_text(&mut app, "10j");
        assert_eq!(app.selected, 4, "clamped to the last row");
        type_text(&mut app, "2G");
        assert_eq!(app.selected, 4, "G goes to the bottom whatever the count");
        app.set_layout(4, true);
        type_text(&mut app, "G(");
        assert_eq!(app.selected, 2, "( goes up half a page");
        type_text(&mut app, ")");
        assert_eq!(app.selected, 4);
        type_text(&mut app, "3gg");
        assert_eq!(app.selected, 2, "3gg goes to row 3");
        type_text(&mut app, "gg0j");
        assert_eq!(app.selected, 1, "a leading 0 is not a count");
    }

    #[test]
    fn parens_page_through_help() {
        let mut app = app(&[]);
        app.set_layout(4, true);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char(')'));
        assert!(matches!(&app.mode, Mode::Help(h) if h.selected == 2));
        press(&mut app, KeyCode::Char('('));
        assert!(matches!(&app.mode, Mode::Help(h) if h.selected == 0));

        app.set_layout(40, true);
        press(&mut app, KeyCode::Char(')'));
        assert!(
            matches!(&app.mode, Mode::Help(h) if h.selected == HELP.len() / 2),
            "half of the box, not the table"
        );
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char(')'));
        assert!(matches!(&app.mode, Mode::Picker(p) if p.selected == THEMES.len() / 2));
        assert_eq!(app.theme, THEMES.len() / 2, "the theme previews as it moves");
        type_text(&mut app, "3k");
        assert!(matches!(&app.mode, Mode::Picker(p) if p.selected == THEMES.len() / 2 - 3));
        type_text(&mut app, "3g");
        assert!(matches!(&app.mode, Mode::Picker(p) if p.selected == THEMES.len() / 2 - 3), "one g waits");
        type_text(&mut app, "g");
        assert!(matches!(&app.mode, Mode::Picker(p) if p.selected == 2), "3gg goes to the third theme");
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('?'));
        type_text(&mut app, "5j2k");
        assert!(matches!(&app.mode, Mode::Help(h) if h.selected == 3), "counts work in help");
        type_text(&mut app, "/2");
        assert!(
            matches!(&app.mode, Mode::Help(h) if h.filter.as_deref() == Some("2")),
            "digits search once / is open"
        );
    }

    #[test]
    fn search_filters_live_and_escape_clears() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "bott");
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(selected_name(&app), "bottom");
        press(&mut app, KeyCode::Enter);
        assert_eq!((&app.mode, app.query.text.as_str()), (&Mode::Normal, "bott"));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.view.rows.len(), 5);
        assert_eq!(selected_name(&app), "bottom", "cursor stays on the same TUI");
    }

    #[test]
    fn enter_in_search_keeps_the_arrowed_row() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!((&app.mode, app.selected), (&Mode::Normal, 1));
    }

    #[test]
    fn typing_q_in_search_does_not_quit() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('/'));
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
        assert_eq!(app.query.text, "q");
    }

    #[test]
    fn categories_cycle_through_all() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.query.category, Some(0));
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.query.category, Some(app.catalog.category_count() as u8 - 1));
    }

    #[test]
    fn left_right_focus_panels_and_up_down_move_in_them() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.query.category, Some(app.catalog.category_count() as u8 - 1), "wraps past All");
        press(&mut app, KeyCode::Down);
        assert_eq!(app.query.category, None, "wraps back to All");
        press(&mut app, KeyCode::Down);
        assert_eq!((app.query.category, app.selected), (Some(0), 0));
        assert!(press(&mut app, KeyCode::Enter).is_empty(), "enter leaves the sidebar, not install");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        assert_eq!((app.query.category, app.selected), (Some(0), 1));

        app.set_layout(4, true);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char(')'));
        assert_eq!(app.query.category, Some(2), ") moves half a page of categories");
        press(&mut app, KeyCode::Char('('));
        assert_eq!(app.query.category, Some(0));
        type_text(&mut app, "3gg");
        assert_eq!(app.query.category, Some(1), "3gg goes to the third row, All being the first");
        let selected = app.selected;
        type_text(&mut app, "3gg");
        assert_eq!(
            (app.query.category, app.selected),
            (Some(1), selected),
            "3gg on row 3 leaves the list alone"
        );
        type_text(&mut app, "gg");
        assert_eq!(app.query.category, None);
        press(&mut app, KeyCode::Right);
        app.set_layout(20, true);

        press(&mut app, KeyCode::Left);
        app.set_layout(20, false);
        assert!(!app.sidebar_focused, "a resize that drops the sidebar drops its focus");
        press(&mut app, KeyCode::Left);
        assert!(!app.sidebar_focused, "a hidden sidebar cannot be focused");
    }

    #[test]
    fn slash_in_the_sidebar_finds_a_category() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "LIB");
        let category = app.query.category.expect("jumped to a category");
        assert_eq!(app.catalog.category_name(category), "Libraries");
        type_text(&mut app, "zz");
        assert_eq!(app.query.category, Some(category), "no match keeps the last hit");
        press(&mut app, KeyCode::Esc);
        assert_eq!((&app.mode, app.query.category, app.sidebar_focused), (&Mode::Normal, None, true));

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "dev");
        press(&mut app, KeyCode::Enter);
        let category = app.query.category.expect("kept the category");
        assert_eq!(app.catalog.category_name(category), "Development");
        assert_eq!((&app.mode, app.sidebar_focused), (&Mode::Normal, true), "enter only closes the search");
        assert!(app.query.text.is_empty(), "the list search is untouched");

        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.query.category, Some(category + 1), "arrows move while searching");
    }

    #[test]
    fn star_picker_applies_a_preset() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('*'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('j'));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.query.min_stars, 10_000);
        assert_eq!(app.view.rows.len(), 3);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.query, Query::default());
    }

    #[test]
    fn theme_picker_previews_saves_and_cancels() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.theme, 1, "moving previews");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.theme, 0, "esc restores");
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(press(&mut app, KeyCode::Enter), [Effect::SaveTheme(THEMES[2].name)]);
        assert_eq!(app.theme, 2);
    }

    #[test]
    fn theme_picker_searches_after_slash() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "NORD");
        let Mode::Picker(picker) = &app.mode else { panic!("no picker") };
        assert_eq!(picker.items, ["nord"]);
        assert_eq!(THEMES[app.theme].name, "nord", "the match previews");
        type_text(&mut app, "x");
        assert_eq!(press(&mut app, KeyCode::Enter), [], "enter closes the search");
        assert_eq!(app.theme, 0, "no match keeps the original");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "q");
        press(&mut app, KeyCode::Esc);
        let Mode::Picker(picker) = &app.mode else { panic!("esc only clears the search") };
        assert_eq!(picker.items.len(), THEMES.len());
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.theme, THEMES.len() - 1, "G previews the last theme");
        type_text(&mut app, "gg");
        assert_eq!(app.theme, 0, "gg previews the first theme");
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "g");
        let Mode::Picker(picker) = &app.mode else { panic!("g closed the picker") };
        assert!(matches!(&picker.kind, PickerKind::Theme { filter: Some(f), .. } if f == "g"), "g is typed");
        press(&mut app, KeyCode::End);
        let Mode::Picker(picker) = &app.mode else { panic!("end closed the picker") };
        assert_eq!(picker.selected, picker.items.len() - 1, "end still jumps while searching");
    }

    #[test]
    fn help_filters_after_slash() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "qx");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Left);
        type_text(&mut app, "éu");
        press(&mut app, KeyCode::Down);
        let Mode::Help(help) = &app.mode else { panic!("help closed") };
        assert_eq!(help.filter.as_deref(), Some("qéu"), "q is typed, not quit");
        assert_eq!(help.selected, 0, "down clamps to the (empty) list");
        type_text(&mut app, " ab cd");
        let ctrl = |c| Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        app.update(ctrl('w'));
        let Mode::Help(help) = &app.mode else { panic!("help closed") };
        assert_eq!(help.filter.as_deref(), Some("qéu ab "), "ctrl-w drops a word");
        app.update(ctrl('u'));
        let Mode::Help(help) = &app.mode else { panic!("help closed") };
        assert_eq!(help.filter.as_deref(), Some(""), "ctrl-u clears");
        type_text(&mut app, &"x".repeat(40));
        let Mode::Help(help) = &app.mode else { panic!("typing past the limit closed help") };
        assert_eq!(help.filter.as_deref().map(str::len), Some(32));
        app.update(ctrl('u'));
        press(&mut app, KeyCode::Esc);
        let Mode::Help(help) = &app.mode else { panic!("esc closes help while searching") };
        assert_eq!(help.filter, None, "first esc clears the search");
        press(&mut app, KeyCode::Char('G'));
        let Mode::Help(help) = &app.mode else { panic!("G closes help") };
        assert_eq!(help.selected, HELP.len() - 1);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "theme");
        press(&mut app, KeyCode::Enter);
        let Mode::Help(help) = &app.mode else { panic!("enter closes help while searching") };
        assert_eq!(
            (help.filter.as_deref(), HELP[help.selected].1),
            (None, "colour theme"),
            "enter keeps the entry"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);

        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Up);
        let Mode::Help(help) = &app.mode else { panic!("arrows close help") };
        assert_eq!(help.selected, 1);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.mode, Mode::Normal);

        let help = Help { filter: Some("L".into()), ..Help::default() };
        assert_eq!(help.rows().map(|(keys, _)| *keys).collect::<Vec<_>>(), ["L"], "keys match by case");
    }

    #[test]
    fn y_copies_in_every_panel() {
        let copied = |effects: Vec<Effect>| match effects.as_slice() {
            [Effect::Copy(text)] => text.clone(),
            other => panic!("expected a copy, got {other:?}"),
        };
        let mut app = app(&[]);
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), "lazygit");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), "All");
        press(&mut app, KeyCode::Char('j'));
        let category = app.catalog.category_name(app.query.category.unwrap()).to_owned();
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), category);

        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), app.theme().name);
        assert!(matches!(app.mode, Mode::Picker(_)), "copying keeps the picker open");
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "zzz");
        assert!(press(&mut app, KeyCode::Enter).is_empty(), "enter closes the search");
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('?'));
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), format!("{}  {}", HELP[0].0, HELP[0].1));
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('v'));
        app.log.extend(["one".to_owned(), "two".to_owned()]);
        assert_eq!(copied(press(&mut app, KeyCode::Char('y'))), "one\ntwo");
        assert_eq!(app.mode, Mode::Log);
    }

    #[test]
    fn language_picker_lists_languages_in_view() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('L'));
        let Mode::Picker(picker) = &app.mode else { panic!("no picker") };
        assert_eq!(picker.items, ["any", "Rust (2)", "C++ (1)", "Go (1)"]);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.view.rows.len(), 2);
    }

    #[test]
    fn a_pressed_key_lights_its_hint_for_a_while() {
        let mut app = app(&[]);
        assert!(!app.busy());
        press(&mut app, KeyCode::Char('s'));
        assert!(app.flash > 0 && app.busy(), "ticks run while the hint is lit");
        for _ in 0..FLASH_TICKS {
            app.update(Event::Tick);
        }
        assert_eq!(app.flash, 0);
        assert!(!app.busy(), "and stop once it goes dark");
    }

    #[test]
    fn opening_a_search_starts_no_ticks() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('/'));
        assert!(!app.busy(), "redraws would restart the cursor's blink");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('/'));
        assert!(!app.busy(), "nor in the sidebar");
        press(&mut app, KeyCode::Esc);
        for open in ['t', '?'] {
            press(&mut app, KeyCode::Char(open));
            press(&mut app, KeyCode::Char('/'));
            assert!(!app.busy(), "nor in the {open} overlay, lit by the key that opened it");
            press(&mut app, KeyCode::Esc);
            press(&mut app, KeyCode::Esc);
        }
    }

    #[test]
    fn install_flow_spawns_then_rescans_and_runs_the_queue() {
        let mut app = app(&["brew", "cargo"]);
        press(&mut app, KeyCode::Enter);
        let Mode::Picker(picker) = &app.mode else { panic!("no confirm") };
        assert_eq!(picker.items, ["brew install lazygit"]);
        let effects = press(&mut app, KeyCode::Enter);
        let [Effect::Spawn(job)] = &effects[..] else { panic!("{effects:?}") };
        assert_eq!(job.argv, ["brew", "install", "lazygit"]);
        assert!(app.busy());

        // A second install while the first runs is queued, not spawned.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(press(&mut app, KeyCode::Char('y')).is_empty());
        assert_eq!(app.queue.len(), 1);

        let brew = by_name("brew").unwrap();
        let effects = app.update(Event::JobDone { ok: true });
        assert_eq!(effects[0], Effect::Scan(Some(brew)));
        assert!(matches!(&effects[1], Effect::Spawn(job) if job.argv == ["brew", "install", "btop"]));

        let effects = app.update(Event::Listing(brew, vec!["lazygit".into()]));
        assert_eq!(effects, [Effect::SaveInstalled]);
        assert!(app.installed.is_installed(2));

        // Enter on an installed row offers to uninstall it.
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Enter);
        let Mode::Picker(picker) = &app.mode else { panic!("no confirm") };
        assert_eq!(picker.items, ["brew uninstall lazygit"]);
    }

    #[test]
    fn a_job_whose_manager_vanished_does_not_wait_on_a_scan() {
        let mut app = app(&["brew"]);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(app.running.is_some());
        // Startup detection lands meanwhile and brew is no longer on PATH.
        let effects = app.update(Event::Detected(Vec::new()));
        assert_eq!((effects, app.scans_pending), (vec![Effect::SaveInstalled], 0));
        let effects = app.update(Event::JobDone { ok: true });
        assert_eq!(effects, [Effect::SaveInstalled], "nothing to scan, so the cache is saved right away");
        assert_eq!((app.scans_pending, app.running.is_none()), (0, true));
    }

    #[test]
    fn upgrade_offers_the_installing_manager_only() {
        let mut app = app(&["brew", "go"]);
        assert!(press(&mut app, KeyCode::Char('u')).is_empty());
        assert!(app.status.contains("not installed"), "{}", app.status);

        app.update(Event::Listing(by_name("go").unwrap(), vec!["lazygit".into()]));
        press(&mut app, KeyCode::Char('u'));
        let Mode::Picker(picker) = &app.mode else { panic!("no confirm") };
        assert_eq!(picker.title, "Upgrade lazygit?");
        assert_eq!(picker.items, ["go install github.com/jesseduffield/lazygit@latest"]);
        let effects = press(&mut app, KeyCode::Enter);
        assert!(matches!(&effects[..], [Effect::Spawn(job)] if job.action == Action::Upgrade));
    }

    #[test]
    fn privileged_managers_take_over_the_terminal() {
        let mut app = app(&["apt"]);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(selected_name(&app), "btop");
        press(&mut app, KeyCode::Enter);
        let effects = press(&mut app, KeyCode::Enter);
        assert!(
            matches!(&effects[..], [Effect::RunInTerminal(job)] if job.argv[..2] == ["sudo", "apt-get"] || job.argv[0] == "apt-get")
        );
    }

    #[test]
    fn impossible_actions_explain_themselves() {
        let mut app = app(&["brew"]);
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Enter);
        assert!(app.status.starts_with("✗ ") && app.status.contains("packaged for nix"), "{}", app.status);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn log_stays_bounded_across_jobs() {
        let mut app = app(&["brew"]);
        for _ in 0..3 {
            press(&mut app, KeyCode::Enter);
            press(&mut app, KeyCode::Enter);
            for _ in 0..LOG_LINES {
                app.update(Event::JobOutput("x".into()));
            }
            app.update(Event::JobDone { ok: true });
        }
        assert_eq!(app.log.len(), LOG_LINES);
    }

    #[test]
    fn quitting_needs_two_presses() {
        let mut app = app(&[]);
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
        assert!(app.status.contains("q again"), "{}", app.status);
        assert!(press(&mut app, KeyCode::Char('j')).is_empty(), "any other key disarms");
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
        assert_eq!(press(&mut app, KeyCode::Char('q')), [Effect::Quit]);
    }

    #[test]
    fn quitting_with_a_running_job_says_so() {
        let mut app = app(&["brew"]);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
        assert!(app.status.contains("still running"), "{}", app.status);
        assert_eq!(press(&mut app, KeyCode::Char('q')), [Effect::Quit]);
    }

    #[test]
    fn fresh_index_resets_catalog_scoped_filters() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('l'));
        app.update(Event::Index(IndexUpdate::Fresh(Box::new(catalog()))));
        assert_eq!(app.query.category, None);
        assert!(app.status.contains("6 TUIs"));
    }

    #[test]
    fn fresh_index_closes_dialogs_holding_old_rows() {
        let smaller = || {
            let mut b = tuiman_index::Builder::new(0);
            b.push(&tuiman_index::Entry { name: "x", url: "https://x.y", ..Default::default() }).unwrap();
            Box::new(b.finish())
        };
        let mut app = app(&["brew"]);
        let row = app.selected_row().unwrap();
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Picker(_)));
        app.update(Event::Index(IndexUpdate::Fresh(smaller())));
        assert_eq!(app.mode, Mode::Normal, "row {row} is gone from the new catalog");
        assert!(press(&mut app, KeyCode::Enter).is_empty());

        press(&mut app, KeyCode::Char('L'));
        app.update(Event::Index(IndexUpdate::Fresh(smaller())));
        assert_eq!(app.mode, Mode::Normal);

        press(&mut app, KeyCode::Char('t'));
        app.update(Event::Index(IndexUpdate::Fresh(smaller())));
        assert!(matches!(app.mode, Mode::Picker(_)), "themes do not depend on the catalog");
    }
}
