use crate::patterns::Category;
use crate::scanner::{CruftEntry, LargeFileEntry, ScanMessage, TopFolderEntry};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Folders,
    Cruft,
    LargeFiles,
    Overview,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Folders => "Folders",
            Tab::Cruft => "Cruft Dirs",
            Tab::LargeFiles => "Large Files",
            Tab::Overview => "Overview",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tab::Folders => Tab::Cruft,
            Tab::Cruft => Tab::LargeFiles,
            Tab::LargeFiles => Tab::Overview,
            Tab::Overview => Tab::Folders,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Folders => Tab::Overview,
            Tab::Cruft => Tab::Folders,
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

pub enum DeleteMessage {
    /// One item was processed (successfully or not).
    Progress,
    /// All items processed.
    Done { deleted: usize, errors: Vec<String> },
}

enum DeleteItem {
    Dir(PathBuf),
    File(PathBuf),
}

#[derive(Debug)]
pub enum Dialog {
    None,
    ConfirmDelete { count: usize, total_size: u64 },
    Deleting { done: usize, total: usize },
    DeleteResult { deleted: usize, errors: Vec<String> },
}

pub struct App {
    pub tab: Tab,
    // Top folder view
    pub top_folders: Vec<TopFolderEntry>,
    pub top_table_state: TableState,
    pub top_selected: HashSet<usize>,
    // Cruft view
    pub cruft_items: Vec<CruftEntry>,
    pub cruft_table_state: TableState,
    pub cruft_selected: HashSet<usize>,
    // Large files view
    pub large_file_items: Vec<LargeFileEntry>,
    pub large_table_state: TableState,
    pub large_selected: HashSet<usize>,
    // Sorting
    pub sort_field: SortField,
    pub sort_ascending: bool,
    // Dialogs
    pub dialog: Dialog,
    // Scan state
    pub scanning: bool,
    pub folders_total: usize,
    pub folders_completed: usize,
    pub bytes_scanned: u64,
    pub should_quit: bool,
    rx: Receiver<ScanMessage>,
    delete_rx: Option<Receiver<DeleteMessage>>,
}

impl App {
    pub fn new(rx: Receiver<ScanMessage>) -> Self {
        Self {
            tab: Tab::Folders,
            top_folders: Vec::new(),
            top_table_state: TableState::default(),
            top_selected: HashSet::new(),
            cruft_items: Vec::new(),
            cruft_table_state: TableState::default(),
            cruft_selected: HashSet::new(),
            large_file_items: Vec::new(),
            large_table_state: TableState::default(),
            large_selected: HashSet::new(),
            sort_field: SortField::Size,
            sort_ascending: false,
            dialog: Dialog::None,
            scanning: true,
            folders_total: 0,
            folders_completed: 0,
            bytes_scanned: 0,
            should_quit: false,
            rx,
            delete_rx: None,
        }
    }

    /// Drain pending scanner messages. Called each frame tick.
    pub fn poll_scanner(&mut self) {
        let mut got_cruft = false;
        let mut got_large = false;
        let mut got_folder = false;

        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                ScanMessage::ScanTotal(total) => {
                    self.folders_total = total;
                }
                ScanMessage::TopFolderDone(entry) => {
                    self.folders_completed += 1;
                    self.bytes_scanned += entry.total_size;
                    self.top_folders.push(entry);
                    got_folder = true;
                }
                ScanMessage::CruftFound(entry) => {
                    self.cruft_items.push(entry);
                    got_cruft = true;
                }
                ScanMessage::LargeFileFound(entry) => {
                    self.large_file_items.push(entry);
                    got_large = true;
                }
                ScanMessage::Done => {
                    self.scanning = false;
                }
                ScanMessage::Error(e) => {
                    // Show error in progress area by noting it
                    eprintln!("Scan error: {e}");
                }
            }
        }

        if got_folder {
            self.top_folders.sort_by(|a, b| b.total_size.cmp(&a.total_size));
            if self.top_table_state.selected().is_none() && !self.top_folders.is_empty() {
                self.top_table_state.select(Some(0));
            }
        }
        if got_cruft {
            self.sort_cruft();
            if self.cruft_table_state.selected().is_none() && !self.cruft_items.is_empty() {
                self.cruft_table_state.select(Some(0));
            }
        }
        if got_large {
            self.sort_large();
            if self.large_table_state.selected().is_none() && !self.large_file_items.is_empty() {
                self.large_table_state.select(Some(0));
            }
        }
    }

    fn sort_cruft(&mut self) {
        self.cruft_selected.clear();
        let asc = self.sort_ascending;
        match self.sort_field {
            SortField::Size => self.cruft_items.sort_by(|a, b| {
                if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
            }),
            SortField::Path => self.cruft_items.sort_by(|a, b| {
                if asc { a.path.cmp(&b.path) } else { b.path.cmp(&a.path) }
            }),
            SortField::Category => self.cruft_items.sort_by(|a, b| {
                let cmp = a.category.as_str().cmp(b.category.as_str());
                if asc { cmp } else { cmp.reverse() }
            }),
        }
    }

    fn sort_large(&mut self) {
        self.large_selected.clear();
        let asc = self.sort_ascending;
        match self.sort_field {
            SortField::Size | SortField::Category => self.large_file_items.sort_by(|a, b| {
                if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
            }),
            SortField::Path => self.large_file_items.sort_by(|a, b| {
                if asc { a.path.cmp(&b.path) } else { b.path.cmp(&a.path) }
            }),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
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
            Dialog::Deleting { .. } => {
                return; // block all keys while deleting
            }
            Dialog::DeleteResult { .. } => {
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
        if matches!(self.tab, Tab::Overview) {
            return;
        }
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
            Tab::Folders => self.top_folders.len(),
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
            Tab::Folders => &self.top_selected,
            Tab::Cruft => &self.cruft_selected,
            Tab::LargeFiles => &self.large_selected,
            Tab::Overview => return,
        };
        if selected.is_empty() {
            return;
        }
        let (count, total_size) = match self.tab {
            Tab::Folders => {
                let size: u64 = selected.iter().map(|&i| self.top_folders[i].total_size).sum();
                (selected.len(), size)
            }
            Tab::Cruft => {
                let size: u64 = selected.iter().map(|&i| self.cruft_items[i].size).sum();
                (selected.len(), size)
            }
            Tab::LargeFiles => {
                let size: u64 = selected.iter().map(|&i| self.large_file_items[i].size).sum();
                (selected.len(), size)
            }
            _ => return,
        };
        self.dialog = Dialog::ConfirmDelete { count, total_size };
    }

    fn execute_delete(&mut self) {
        let mut deleted = 0;
        let mut errors = Vec::new();

        match self.tab {
            Tab::Folders => {
                let mut indices: Vec<usize> = self.top_selected.iter().copied().collect();
                indices.sort_unstable_by(|a, b| b.cmp(a));
                for idx in &indices {
                    let path = &self.top_folders[*idx].path;
                    match std::fs::remove_dir_all(path) {
                        Ok(()) => deleted += 1,
                        Err(e) => errors.push(format!("{}: {e}", path.display())),
                    }
                }
                for idx in &indices {
                    self.top_folders.remove(*idx);
                }
                self.top_selected.clear();
                if let Some(sel) = self.top_table_state.selected() {
                    if sel >= self.top_folders.len() && !self.top_folders.is_empty() {
                        self.top_table_state
                            .select(Some(self.top_folders.len() - 1));
                    } else if self.top_folders.is_empty() {
                        self.top_table_state.select(None);
                    }
                }
            }
            Tab::Cruft => {
                let mut indices: Vec<usize> = self.cruft_selected.iter().copied().collect();
                indices.sort_unstable_by(|a, b| b.cmp(a));
                for idx in &indices {
                    let path = &self.cruft_items[*idx].path;
                    match std::fs::remove_dir_all(path) {
                        Ok(()) => deleted += 1,
                        Err(e) => errors.push(format!("{}: {e}", path.display())),
                    }
                }
                for idx in &indices {
                    self.cruft_items.remove(*idx);
                }
                self.cruft_selected.clear();
                if let Some(sel) = self.cruft_table_state.selected() {
                    if sel >= self.cruft_items.len() && !self.cruft_items.is_empty() {
                        self.cruft_table_state
                            .select(Some(self.cruft_items.len() - 1));
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
            _ => {}
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
        self.sort_cruft();
        self.sort_large();
    }

    fn active_table(&mut self) -> (&mut TableState, usize) {
        match self.tab {
            Tab::Folders => (&mut self.top_table_state, self.top_folders.len()),
            Tab::Cruft => (&mut self.cruft_table_state, self.cruft_items.len()),
            Tab::LargeFiles => (&mut self.large_table_state, self.large_file_items.len()),
            Tab::Overview => (&mut self.top_table_state, 0),
        }
    }

    fn active_selected_mut(&mut self) -> &mut HashSet<usize> {
        match self.tab {
            Tab::Folders => &mut self.top_selected,
            Tab::Cruft => &mut self.cruft_selected,
            _ => &mut self.large_selected,
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
        stats.sort_by(|a, b| b.1.cmp(&a.1));
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
            Tab::Folders => self.top_selected.iter().map(|&i| self.top_folders[i].total_size).sum(),
            Tab::Cruft => self.cruft_selected.iter().map(|&i| self.cruft_items[i].size).sum(),
            Tab::LargeFiles => self.large_selected.iter().map(|&i| self.large_file_items[i].size).sum(),
            _ => 0,
        }
    }

    pub fn selected_count(&self) -> usize {
        match self.tab {
            Tab::Folders => self.top_selected.len(),
            Tab::Cruft => self.cruft_selected.len(),
            Tab::LargeFiles => self.large_selected.len(),
            _ => 0,
        }
    }

    pub fn scan_progress_ratio(&self) -> f64 {
        if self.folders_total == 0 {
            0.0
        } else {
            self.folders_completed as f64 / self.folders_total as f64
        }
    }
}
