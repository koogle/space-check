use crate::patterns::Category;
use crate::scanner::{CruftEntry, LargeFileEntry, ScanMessage};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Cruft,
    LargeFiles,
    Overview,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Cruft => "Cruft Dirs",
            Tab::LargeFiles => "Large Files",
            Tab::Overview => "Overview",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tab::Cruft => Tab::LargeFiles,
            Tab::LargeFiles => Tab::Overview,
            Tab::Overview => Tab::Cruft,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Cruft => Tab::Overview,
            Tab::LargeFiles => Tab::Cruft,
            Tab::Overview => Tab::LargeFiles,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Size,
    Path,
    Category,
}

#[derive(Debug)]
pub enum Dialog {
    None,
    ConfirmDelete { count: usize, total_size: u64 },
    DeleteResult { deleted: usize, errors: Vec<String> },
}

pub struct App {
    pub tab: Tab,
    pub cruft_items: Vec<CruftEntry>,
    pub large_file_items: Vec<LargeFileEntry>,
    pub cruft_table_state: TableState,
    pub large_table_state: TableState,
    pub cruft_selected: HashSet<usize>,
    pub large_selected: HashSet<usize>,
    pub sort_field: SortField,
    pub sort_ascending: bool,
    pub dialog: Dialog,
    pub scanning: bool,
    pub scan_progress: Option<PathBuf>,
    pub should_quit: bool,
    rx: Receiver<ScanMessage>,
}

impl App {
    pub fn new(rx: Receiver<ScanMessage>) -> Self {
        Self {
            tab: Tab::Cruft,
            cruft_items: Vec::new(),
            large_file_items: Vec::new(),
            cruft_table_state: TableState::default(),
            large_table_state: TableState::default(),
            cruft_selected: HashSet::new(),
            large_selected: HashSet::new(),
            sort_field: SortField::Size,
            sort_ascending: false,
            dialog: Dialog::None,
            scanning: true,
            scan_progress: None,
            should_quit: false,
            rx,
        }
    }

    /// Drain pending scanner messages. Called each frame tick.
    pub fn poll_scanner(&mut self) {
        let mut got_items = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                ScanMessage::CruftFound(entry) => {
                    self.cruft_items.push(entry);
                    got_items = true;
                }
                ScanMessage::LargeFileFound(entry) => {
                    self.large_file_items.push(entry);
                    got_items = true;
                }
                ScanMessage::Progress(path) => {
                    self.scan_progress = Some(path);
                }
                ScanMessage::Done => {
                    self.scanning = false;
                }
                ScanMessage::Error(e) => {
                    self.scan_progress = Some(PathBuf::from(format!("Error: {e}")));
                }
            }
        }
        if got_items {
            self.sort_items();
            // Auto-select first row if nothing selected yet
            if self.cruft_table_state.selected().is_none() && !self.cruft_items.is_empty() {
                self.cruft_table_state.select(Some(0));
            }
            if self.large_table_state.selected().is_none() && !self.large_file_items.is_empty() {
                self.large_table_state.select(Some(0));
            }
        }
    }

    fn sort_items(&mut self) {
        // Clear selections before sort since indices change
        self.cruft_selected.clear();
        self.large_selected.clear();

        let asc = self.sort_ascending;
        match self.sort_field {
            SortField::Size => {
                self.cruft_items.sort_by(|a, b| {
                    if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
                });
                self.large_file_items.sort_by(|a, b| {
                    if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
                });
            }
            SortField::Path => {
                self.cruft_items.sort_by(|a, b| {
                    if asc { a.path.cmp(&b.path) } else { b.path.cmp(&a.path) }
                });
                self.large_file_items.sort_by(|a, b| {
                    if asc { a.path.cmp(&b.path) } else { b.path.cmp(&a.path) }
                });
            }
            SortField::Category => {
                self.cruft_items.sort_by(|a, b| {
                    let cmp = a.category.as_str().cmp(b.category.as_str());
                    if asc { cmp } else { cmp.reverse() }
                });
                // Large files don't have categories, fall back to size
                self.large_file_items.sort_by(|a, b| {
                    if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
                });
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Only handle Press events (prevents double-handling on Windows)
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Handle dialog keys first
        match &self.dialog {
            Dialog::ConfirmDelete { .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.execute_delete(),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.dialog = Dialog::None;
                    }
                    _ => {}
                }
                return;
            }
            Dialog::DeleteResult { .. } => {
                // Any key dismisses
                self.dialog = Dialog::None;
                return;
            }
            Dialog::None => {}
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Tab => self.tab = self.tab.next(),
            KeyCode::BackTab => self.tab = self.tab.prev(),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char('G') | KeyCode::End => self.move_cursor_to_end(),
            KeyCode::Home => self.move_cursor_to_start(),
            KeyCode::Char(' ') => self.toggle_selection(),
            KeyCode::Char('a') => self.select_all(),
            KeyCode::Char('d') => self.request_delete(),
            KeyCode::Char('s') => self.cycle_sort(),
            _ => {}
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let (state, len) = self.active_table();
        if len == 0 {
            return;
        }
        let current = state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, len as i32 - 1) as usize;
        state.select(Some(next));
    }

    fn move_cursor_to_end(&mut self) {
        let (state, len) = self.active_table();
        if len > 0 {
            state.select(Some(len - 1));
        }
    }

    fn move_cursor_to_start(&mut self) {
        let (state, len) = self.active_table();
        if len > 0 {
            state.select(Some(0));
        }
    }

    fn toggle_selection(&mut self) {
        let (state, _) = self.active_table();
        if let Some(idx) = state.selected() {
            let selected = self.active_selected_mut();
            if !selected.remove(&idx) {
                selected.insert(idx);
            }
        }
    }

    fn select_all(&mut self) {
        let len = match self.tab {
            Tab::Cruft => self.cruft_items.len(),
            Tab::LargeFiles => self.large_file_items.len(),
            Tab::Overview => return,
        };
        let selected = self.active_selected_mut();
        if selected.len() == len {
            selected.clear();
        } else {
            *selected = (0..len).collect();
        }
    }

    fn request_delete(&mut self) {
        let selected = match self.tab {
            Tab::Cruft => &self.cruft_selected,
            Tab::LargeFiles => &self.large_selected,
            Tab::Overview => return,
        };
        if selected.is_empty() {
            return;
        }
        let (count, total_size) = match self.tab {
            Tab::Cruft => {
                let size: u64 = selected.iter().map(|&i| self.cruft_items[i].size).sum();
                (selected.len(), size)
            }
            Tab::LargeFiles => {
                let size: u64 = selected.iter().map(|&i| self.large_file_items[i].size).sum();
                (selected.len(), size)
            }
            Tab::Overview => return,
        };
        self.dialog = Dialog::ConfirmDelete { count, total_size };
    }

    fn execute_delete(&mut self) {
        let mut deleted = 0;
        let mut errors = Vec::new();

        match self.tab {
            Tab::Cruft => {
                let mut indices: Vec<usize> = self.cruft_selected.iter().copied().collect();
                indices.sort_unstable_by(|a, b| b.cmp(a)); // reverse to remove from end
                for idx in &indices {
                    let path = &self.cruft_items[*idx].path;
                    match std::fs::remove_dir_all(path) {
                        Ok(()) => deleted += 1,
                        Err(e) => errors.push(format!("{}: {e}", path.display())),
                    }
                }
                // Remove from list in reverse order
                for idx in &indices {
                    self.cruft_items.remove(*idx);
                }
                self.cruft_selected.clear();
                // Fix cursor
                if let Some(sel) = self.cruft_table_state.selected() {
                    if sel >= self.cruft_items.len() && !self.cruft_items.is_empty() {
                        self.cruft_table_state.select(Some(self.cruft_items.len() - 1));
                    } else if self.cruft_items.is_empty() {
                        self.cruft_table_state.select(None);
                    }
                }
            }
            Tab::LargeFiles => {
                let mut indices: Vec<usize> = self.large_selected.iter().copied().collect();
                indices.sort_unstable_by(|a, b| b.cmp(a));
                for idx in &indices {
                    let path = &self.large_file_items[*idx].path;
                    match std::fs::remove_file(path) {
                        Ok(()) => deleted += 1,
                        Err(e) => errors.push(format!("{}: {e}", path.display())),
                    }
                }
                for idx in &indices {
                    self.large_file_items.remove(*idx);
                }
                self.large_selected.clear();
                if let Some(sel) = self.large_table_state.selected() {
                    if sel >= self.large_file_items.len() && !self.large_file_items.is_empty() {
                        self.large_table_state
                            .select(Some(self.large_file_items.len() - 1));
                    } else if self.large_file_items.is_empty() {
                        self.large_table_state.select(None);
                    }
                }
            }
            Tab::Overview => {}
        }

        self.dialog = Dialog::DeleteResult { deleted, errors };
    }

    fn cycle_sort(&mut self) {
        if self.sort_field == SortField::Size && !self.sort_ascending {
            self.sort_ascending = true;
        } else {
            self.sort_ascending = false;
            self.sort_field = match self.sort_field {
                SortField::Size => SortField::Path,
                SortField::Path => SortField::Category,
                SortField::Category => SortField::Size,
            };
        }
        self.sort_items();
    }

    fn active_table(&mut self) -> (&mut TableState, usize) {
        match self.tab {
            Tab::Cruft => (&mut self.cruft_table_state, self.cruft_items.len()),
            Tab::LargeFiles => (&mut self.large_table_state, self.large_file_items.len()),
            Tab::Overview => (&mut self.cruft_table_state, 0), // overview has no table
        }
    }

    fn active_selected_mut(&mut self) -> &mut HashSet<usize> {
        match self.tab {
            Tab::Cruft => &mut self.cruft_selected,
            Tab::LargeFiles | Tab::Overview => &mut self.large_selected,
        }
    }

    /// Compute category breakdown for the overview tab.
    pub fn category_stats(&self) -> Vec<(Category, u64, usize)> {
        let mut map: HashMap<Category, (u64, usize)> = HashMap::new();
        for item in &self.cruft_items {
            let entry = map.entry(item.category).or_insert((0, 0));
            entry.0 += item.size;
            entry.1 += 1;
        }
        let mut stats: Vec<(Category, u64, usize)> = map
            .into_iter()
            .map(|(cat, (size, count))| (cat, size, count))
            .collect();
        stats.sort_by(|a, b| b.1.cmp(&a.1)); // sort by size desc
        stats
    }

    pub fn total_cruft_size(&self) -> u64 {
        self.cruft_items.iter().map(|i| i.size).sum()
    }

    pub fn total_large_size(&self) -> u64 {
        self.large_file_items.iter().map(|i| i.size).sum()
    }

    pub fn selected_size(&self) -> u64 {
        match self.tab {
            Tab::Cruft => self.cruft_selected.iter().map(|&i| self.cruft_items[i].size).sum(),
            Tab::LargeFiles => self.large_selected.iter().map(|&i| self.large_file_items[i].size).sum(),
            Tab::Overview => 0,
        }
    }

    pub fn selected_count(&self) -> usize {
        match self.tab {
            Tab::Cruft => self.cruft_selected.len(),
            Tab::LargeFiles => self.large_selected.len(),
            Tab::Overview => 0,
        }
    }
}
