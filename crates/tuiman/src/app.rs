//! Application state and its single transition function.
//!
//! `App::update` is pure with respect to the outside world: it mutates state
//! and returns [`Effect`]s for the shell in `main.rs` to carry out. No
//! terminal, process or network access happens here, so every interaction is
//! unit-testable.

use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tuiman_index::{Catalog, Row};

use crate::installed::{Choice, Installed};
use crate::managers::{Action, ManagerId, MANAGERS};
use crate::query::{Query, View};
use crate::theme::{Theme, THEMES};

const LOG_LINES: usize = 2000;
pub const STAR_PRESETS: [u32; 7] = [0, 100, 500, 1_000, 5_000, 10_000, 50_000];

pub enum Event {
    Key(KeyEvent),
    Resize,
    /// The `PATH` walk for package managers finished.
    Detected(Vec<(ManagerId, std::path::PathBuf)>),
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
    SaveInstalled,
    /// Remember the theme called this for the next start.
    SaveTheme(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub action: Action,
    pub manager: ManagerId,
    pub title: String,
    pub argv: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Picker(Picker),
    Help,
    Log,
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
    /// Items are [`THEMES`]; moving previews, `original` is restored on cancel.
    Theme {
        original: usize,
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
    quit_armed: bool,
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
            quit_armed: false,
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

    /// Whether something is in flight and the spinner should animate.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.refreshing || self.detecting || self.scans_pending > 0
    }

    pub fn update(&mut self, event: Event) -> Vec<Effect> {
        self.dirty = true;
        match event {
            Event::Key(key) => return self.on_key(key),
            Event::Resize => {}
            Event::Tick => self.spinner = self.spinner.wrapping_add(1),
            Event::Detected(detected) => {
                self.scans_pending += detected.len();
                self.detecting = false;
                self.installed.set_detected(detected, &self.catalog);
                self.refilter(true);
                return vec![Effect::Scan(None)];
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
        let mut effects = vec![Effect::Scan(Some(job.manager))];
        self.scans_pending += 1;
        if let Some(next) = self.queue.pop_front() {
            effects.push(self.start(next));
        }
        effects
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

    /// Called by the shell when the layout changes.
    pub fn set_page(&mut self, page: usize) {
        if self.page != page {
            self.page = page;
            self.scroll_into_view();
        }
    }

    fn move_by(&mut self, delta: isize) {
        self.selected = self.selected.saturating_add_signed(delta);
        self.scroll_into_view();
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return vec![Effect::Quit];
        }
        let armed = std::mem::take(&mut self.quit_armed);
        match self.mode {
            Mode::Normal => self.on_normal_key(key.code, ctrl, armed),
            Mode::Search => {
                self.on_search_key(key.code, ctrl);
                Vec::new()
            }
            Mode::Picker(_) => self.on_picker_key(key.code),
            Mode::Help | Mode::Log => {
                self.mode = Mode::Normal;
                Vec::new()
            }
        }
    }

    fn on_normal_key(&mut self, code: KeyCode, ctrl: bool, quit_armed: bool) -> Vec<Effect> {
        self.status.clear();
        let half_page = (self.page / 2).max(1) as isize;
        match code {
            KeyCode::Char('q') => {
                if self.running.is_none() || quit_armed {
                    return vec![Effect::Quit];
                }
                self.quit_armed = true;
                self.status = "A job is still running (press q again to quit anyway)".into();
            }
            KeyCode::Char('d') if ctrl => self.move_by(half_page),
            KeyCode::Char('u') if ctrl => self.move_by(-half_page),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(self.page as isize),
            KeyCode::PageUp => self.move_by(-(self.page as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.move_by(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_by(isize::MAX),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => self.cycle_category(1),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab => self.cycle_category(-1),
            KeyCode::Char('/') => self.mode = Mode::Search,
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
                self.refilter(true);
            }
            KeyCode::Char('t') => {
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
            KeyCode::Char('S') => self.open_stars_picker(),
            KeyCode::Char('L') => self.open_language_picker(),
            KeyCode::Char('i') | KeyCode::Enter => self.open_confirm(Action::Install),
            KeyCode::Char('x') => self.open_confirm(Action::Uninstall),
            KeyCode::Char('o') => {
                if let Some(row) = self.selected_row() {
                    return vec![Effect::OpenUrl(self.catalog.url(row).to_owned())];
                }
            }
            KeyCode::Char('r') if !self.refreshing => {
                self.refreshing = true;
                self.status = "Refreshing index…".into();
                return vec![Effect::RefreshIndex];
            }
            KeyCode::Char('v') => self.mode = Mode::Log,
            KeyCode::Char('T') => {
                self.mode = Mode::Picker(Picker {
                    title: "Theme".into(),
                    items: THEMES.iter().map(|t| t.name.to_owned()).collect(),
                    selected: self.theme,
                    kind: PickerKind::Theme { original: self.theme },
                });
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
        Vec::new()
    }

    fn on_search_key(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.query.text.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Down => return self.move_by(1),
            KeyCode::Up => return self.move_by(-1),
            KeyCode::Backspace => {
                self.query.text.pop();
            }
            KeyCode::Char('u') if ctrl => self.query.text.clear(),
            KeyCode::Char('w') if ctrl => {
                let kept = self.query.text.trim_end().rfind(' ').map_or(0, |i| i + 1);
                self.query.text.truncate(kept);
            }
            KeyCode::Char(c) if !ctrl && self.query.text.len() < 64 => self.query.text.push(c),
            _ => return,
        }
        self.refilter(false);
    }

    /// Steps through "All" followed by each category.
    fn cycle_category(&mut self, step: isize) {
        let count = self.catalog.category_count() as isize + 1;
        let current = self.query.category.map_or(0, |c| c as isize + 1);
        let next = (current + step).rem_euclid(count);
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
            self.status = match action {
                Action::Uninstall => {
                    format!("{name} is not installed through a package manager tuiman knows")
                }
                Action::Install if self.catalog.is_library(row) => {
                    format!("{name} is a library, not an application")
                }
                Action::Install if self.installed.is_installed(row) => format!("{name} is already installed"),
                Action::Install => {
                    let known: Vec<&str> = self.catalog.packages(row).map(|(eco, _)| eco.name()).collect();
                    match known.is_empty() {
                        true => format!("No known package for {name} (press o to open its page)"),
                        false => format!(
                            "{name} is packaged for {}, none of which is on this machine",
                            known.join(", ")
                        ),
                    }
                }
            };
            return;
        }
        let items =
            choices.iter().map(|c| MANAGERS[c.manager as usize].argv(action, &c.package).join(" ")).collect();
        let title = format!("{} {name}?", if action == Action::Install { "Install" } else { "Uninstall" });
        let kind = PickerKind::Confirm { action, row, choices };
        self.mode = Mode::Picker(Picker { title, items, selected: 0, kind });
    }

    fn on_picker_key(&mut self, code: KeyCode) -> Vec<Effect> {
        let Mode::Picker(picker) = &mut self.mode else { return Vec::new() };
        let last = picker.items.len().saturating_sub(1);
        match code {
            KeyCode::Char('j') | KeyCode::Down => picker.selected = (picker.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('y') => {
                let Mode::Picker(picker) = std::mem::replace(&mut self.mode, Mode::Normal) else {
                    unreachable!()
                };
                return self.on_picked(picker);
            }
            KeyCode::Esc | KeyCode::Char('q' | 'n') => {
                if let PickerKind::Theme { original } = picker.kind {
                    self.theme = original;
                }
                self.mode = Mode::Normal;
                return Vec::new();
            }
            _ => {}
        }
        if let PickerKind::Theme { .. } = picker.kind {
            self.theme = picker.selected;
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
            PickerKind::Theme { .. } => {
                self.theme = picker.selected;
                self.status = format!("Theme: {}", self.theme().name);
                return vec![Effect::SaveTheme(self.theme().name)];
            }
            PickerKind::Confirm { action, row, choices } => {
                let choice = &choices[picker.selected];
                let manager = &MANAGERS[choice.manager as usize];
                let job = Job {
                    action,
                    manager: choice.manager,
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
                    self.status = format!("Running: {} (press v for output)", job.title);
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
        self.scans_pending += 1;
        vec![Effect::Scan(Some(job.manager))]
    }
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
        app.set_page(2);
        assert_eq!(selected_name(&app), "lazygit");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected, 0);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!((app.selected, app.offset), (4, 3));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected, 4);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!((app.selected, app.offset), (0, 0));
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
    fn typing_q_in_search_does_not_quit() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('/'));
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
        assert_eq!(app.query.text, "q");
    }

    #[test]
    fn categories_cycle_through_all() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.query.category, Some(0));
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.query.category, Some(app.catalog.category_count() as u8 - 1));
    }

    #[test]
    fn star_picker_applies_a_preset() {
        let mut app = app(&[]);
        press(&mut app, KeyCode::Char('S'));
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
        press(&mut app, KeyCode::Char('T'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.theme, 1, "moving previews");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.theme, 0, "esc restores");
        press(&mut app, KeyCode::Char('T'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(press(&mut app, KeyCode::Enter), [Effect::SaveTheme(THEMES[2].name)]);
        assert_eq!(app.theme, 2);
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
    fn install_flow_spawns_then_rescans_and_runs_the_queue() {
        let mut app = app(&["brew", "cargo"]);
        press(&mut app, KeyCode::Char('i'));
        let Mode::Picker(picker) = &app.mode else { panic!("no confirm") };
        assert_eq!(picker.items, ["brew install lazygit"]);
        let effects = press(&mut app, KeyCode::Enter);
        let [Effect::Spawn(job)] = &effects[..] else { panic!("{effects:?}") };
        assert_eq!(job.argv, ["brew", "install", "lazygit"]);
        assert!(app.busy());

        // A second install while the first runs is queued, not spawned.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('i'));
        assert!(press(&mut app, KeyCode::Char('y')).is_empty());
        assert_eq!(app.queue.len(), 1);

        let brew = by_name("brew").unwrap();
        let effects = app.update(Event::JobDone { ok: true });
        assert_eq!(effects[0], Effect::Scan(Some(brew)));
        assert!(matches!(&effects[1], Effect::Spawn(job) if job.argv == ["brew", "install", "btop"]));

        let effects = app.update(Event::Listing(brew, vec!["lazygit".into()]));
        assert_eq!(effects, [Effect::SaveInstalled]);
        assert!(app.installed.is_installed(2));
    }

    #[test]
    fn privileged_managers_take_over_the_terminal() {
        let mut app = app(&["apt"]);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(selected_name(&app), "btop");
        press(&mut app, KeyCode::Char('i'));
        let effects = press(&mut app, KeyCode::Enter);
        assert!(
            matches!(&effects[..], [Effect::RunInTerminal(job)] if job.argv[..2] == ["sudo", "apt-get"] || job.argv[0] == "apt-get")
        );
    }

    #[test]
    fn impossible_actions_explain_themselves() {
        let mut app = app(&["brew"]);
        press(&mut app, KeyCode::Char('x'));
        assert!(app.status.contains("not installed"), "{}", app.status);
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Char('i'));
        assert!(app.status.contains("packaged for nix"), "{}", app.status);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn log_stays_bounded_across_jobs() {
        let mut app = app(&["brew"]);
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('i'));
            press(&mut app, KeyCode::Enter);
            for _ in 0..LOG_LINES {
                app.update(Event::JobOutput("x".into()));
            }
            app.update(Event::JobDone { ok: true });
        }
        assert_eq!(app.log.len(), LOG_LINES);
    }

    #[test]
    fn quitting_with_a_running_job_needs_two_presses() {
        let mut app = app(&["brew"]);
        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Enter);
        assert!(press(&mut app, KeyCode::Char('q')).is_empty());
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
}
