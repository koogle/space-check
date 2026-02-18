use crate::patterns::Category;
use crate::scanner::{self, CruftEntry, LargeFileEntry, ScanMessage, TopFolderEntry};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Folders,
    Cruft,
    LargeFiles,
    Selected,
    Overview,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Folders => "Folders",
            Tab::Cruft => "Cruft Dirs",
            Tab::LargeFiles => "Large Files",
            Tab::Selected => "Selected",
            Tab::Overview => "Overview",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tab::Folders => Tab::Cruft,
            Tab::Cruft => Tab::LargeFiles,
            Tab::LargeFiles => Tab::Selected,
            Tab::Selected => Tab::Overview,
            Tab::Overview => Tab::Folders,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Folders => Tab::Overview,
            Tab::Cruft => Tab::Folders,
            Tab::LargeFiles => Tab::Cruft,
            Tab::Selected => Tab::LargeFiles,
            Tab::Overview => Tab::Selected,
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
    // Navigation
    pub nav_stack: Vec<PathBuf>,
    pub threshold_bytes: u64,
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
    // Persistent path-based selections (survive navigation)
    pub selected_paths: HashSet<PathBuf>,
    pub selected_table_state: TableState,
    // Scan state
    pub scanning: bool,
    pub folders_total: usize,
    pub folders_completed: usize,
    pub bytes_scanned: u64,
    pub should_quit: bool,
    rx: Receiver<ScanMessage>,
    cancel: Arc<AtomicBool>,
    delete_rx: Option<Receiver<DeleteMessage>>,
}

impl App {
    pub fn new(rx: Receiver<ScanMessage>, root: PathBuf, threshold_bytes: u64, cancel: Arc<AtomicBool>) -> Self {
        Self {
            tab: Tab::Folders,
            nav_stack: vec![root],
            threshold_bytes,
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
            selected_paths: HashSet::new(),
            selected_table_state: TableState::default(),
            scanning: true,
            folders_total: 0,
            folders_completed: 0,
            bytes_scanned: 0,
            should_quit: false,
            rx,
            cancel,
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
            self.rebuild_top_selected();
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

    /// Rebuild index-based selections from `selected_paths`.
    fn rebuild_top_selected(&mut self) {
        self.top_selected.clear();
        for (i, f) in self.top_folders.iter().enumerate() {
            if self.selected_paths.contains(&f.path) {
                self.top_selected.insert(i);
            }
        }
    }

    fn rebuild_cruft_selected(&mut self) {
        self.cruft_selected.clear();
        for (i, c) in self.cruft_items.iter().enumerate() {
            if self.selected_paths.contains(&c.path) {
                self.cruft_selected.insert(i);
            }
        }
    }

    fn rebuild_large_selected(&mut self) {
        self.large_selected.clear();
        for (i, l) in self.large_file_items.iter().enumerate() {
            if self.selected_paths.contains(&l.path) {
                self.large_selected.insert(i);
            }
        }
    }

    fn sort_cruft(&mut self) {
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
        self.rebuild_cruft_selected();
    }

    fn sort_large(&mut self) {
        let asc = self.sort_ascending;
        match self.sort_field {
            SortField::Size | SortField::Category => self.large_file_items.sort_by(|a, b| {
                if asc { a.size.cmp(&b.size) } else { b.size.cmp(&a.size) }
            }),
            SortField::Path => self.large_file_items.sort_by(|a, b| {
                if asc { a.path.cmp(&b.path) } else { b.path.cmp(&a.path) }
            }),
        }
        self.rebuild_large_selected();
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
            KeyCode::Enter => self.enter_folder(),
            KeyCode::Backspace => self.go_back(),
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
        if self.tab == Tab::Selected {
            // On Selected tab, space removes the item
            let paths: Vec<PathBuf> = self.sorted_selected_paths();
            if let Some(idx) = self.selected_table_state.selected() {
                if let Some(p) = paths.get(idx) {
                    self.selected_paths.remove(p);
                    // Fix cursor if it's past the end
                    if self.selected_paths.is_empty() {
                        self.selected_table_state.select(None);
                    } else if idx >= self.selected_paths.len() {
                        self.selected_table_state.select(Some(self.selected_paths.len() - 1));
                    }
                }
            }
            return;
        }
        let (state, _) = self.active_table();
        if let Some(idx) = state.selected() {
            let path = self.path_at(idx);
            let selected = self.active_selected_mut();
            if !selected.remove(&idx) {
                selected.insert(idx);
                if let Some(p) = path {
                    self.selected_paths.insert(p);
                }
            } else if let Some(p) = path {
                self.selected_paths.remove(&p);
            }
        }
    }

    fn select_all(&mut self) {
        if self.tab == Tab::Selected {
            // On Selected tab, "a" clears all selections
            self.selected_paths.clear();
            self.selected_table_state.select(None);
            // Also clear index-based selections
            self.top_selected.clear();
            self.cruft_selected.clear();
            self.large_selected.clear();
            return;
        }
        let (paths, len): (Vec<PathBuf>, usize) = match self.tab {
            Tab::Folders => (self.top_folders.iter().map(|f| f.path.clone()).collect(), self.top_folders.len()),
            Tab::Cruft => (self.cruft_items.iter().map(|c| c.path.clone()).collect(), self.cruft_items.len()),
            Tab::LargeFiles => (self.large_file_items.iter().map(|l| l.path.clone()).collect(), self.large_file_items.len()),
            Tab::Overview | Tab::Selected => return,
        };
        let selected = self.active_selected_mut();
        if selected.len() == len {
            // Deselect all
            selected.clear();
            for p in &paths {
                self.selected_paths.remove(p);
            }
        } else {
            // Select all
            *selected = (0..len).collect();
            for p in paths {
                self.selected_paths.insert(p);
            }
        }
    }

    fn request_delete(&mut self) {
        if matches!(self.tab, Tab::Overview) {
            return;
        }
        // If nothing explicitly selected, default to the cursor row
        let selected = match self.tab {
            Tab::Folders => &mut self.top_selected,
            Tab::Cruft => &mut self.cruft_selected,
            Tab::LargeFiles => &mut self.large_selected,
            Tab::Overview => unreachable!(),
        };
        if selected.is_empty() {
            let (state, len) = self.active_table();
            if let Some(idx) = state.selected() {
                if idx < len {
                    if let Some(p) = self.path_at(idx) {
                        self.selected_paths.insert(p);
                    }
                    let sel = self.active_selected_mut();
                    sel.insert(idx);
                }
            }
        }
        let selected = match self.tab {
            Tab::Folders => &self.top_selected,
            Tab::Cruft => &self.cruft_selected,
            Tab::LargeFiles => &self.large_selected,
            Tab::Overview => unreachable!(),
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
        // Collect items to delete and remove them from the UI lists immediately.
        let mut items: Vec<DeleteItem> = Vec::new();

        match self.tab {
            Tab::Folders => {
                let mut indices: Vec<usize> = self.top_selected.iter().copied().collect();
                indices.sort_unstable_by(|a, b| b.cmp(a));
                for &idx in &indices {
                    self.selected_paths.remove(&self.top_folders[idx].path);
                    items.push(DeleteItem::Dir(self.top_folders[idx].path.clone()));
                }
                for &idx in &indices {
                    self.top_folders.remove(idx);
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
                for &idx in &indices {
                    self.selected_paths.remove(&self.cruft_items[idx].path);
                    items.push(DeleteItem::Dir(self.cruft_items[idx].path.clone()));
                }
                for &idx in &indices {
                    self.cruft_items.remove(idx);
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
                for &idx in &indices {
                    self.selected_paths.remove(&self.large_file_items[idx].path);
                    items.push(DeleteItem::File(self.large_file_items[idx].path.clone()));
                }
                for &idx in &indices {
                    self.large_file_items.remove(idx);
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
            _ => return,
        }

        let total = items.len();
        let (tx, rx) = std::sync::mpsc::channel();
        self.delete_rx = Some(rx);
        self.dialog = Dialog::Deleting { done: 0, total };

        std::thread::spawn(move || {
            let mut deleted = 0;
            let mut errors = Vec::new();
            for item in items {
                let result = match &item {
                    DeleteItem::Dir(p) => std::fs::remove_dir_all(p)
                        .map_err(|e| format!("{}: {e}", p.display())),
                    DeleteItem::File(p) => std::fs::remove_file(p)
                        .map_err(|e| format!("{}: {e}", p.display())),
                };
                match result {
                    Ok(()) => deleted += 1,
                    Err(msg) => errors.push(msg),
                }
                let _ = tx.send(DeleteMessage::Progress);
            }
            let _ = tx.send(DeleteMessage::Done { deleted, errors });
        });
    }

    /// Drain pending delete messages. Called each frame tick.
    pub fn poll_delete(&mut self) {
        let rx = match &self.delete_rx {
            Some(rx) => rx,
            None => return,
        };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                DeleteMessage::Progress => {
                    if let Dialog::Deleting { ref mut done, .. } = self.dialog {
                        *done += 1;
                    }
                }
                DeleteMessage::Done { deleted, errors } => {
                    self.dialog = Dialog::DeleteResult { deleted, errors };
                    self.delete_rx = None;
                    return;
                }
            }
        }
    }

    pub fn is_deleting(&self) -> bool {
        self.delete_rx.is_some()
    }

    fn start_new_scan(&mut self, path: PathBuf) {
        // Cancel the previous scan
        self.cancel.store(true, Ordering::Relaxed);

        // New cancel token for the new scan
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();

        let (tx, rx) = mpsc::channel();
        scanner::start_scan(path, self.threshold_bytes, tx, cancel);
        self.rx = rx;

        self.top_folders.clear();
        self.top_table_state = TableState::default();
        self.top_selected.clear();
        self.cruft_items.clear();
        self.cruft_table_state = TableState::default();
        self.cruft_selected.clear();
        self.large_file_items.clear();
        self.large_table_state = TableState::default();
        self.large_selected.clear();

        self.scanning = true;
        self.folders_total = 0;
        self.folders_completed = 0;
        self.bytes_scanned = 0;
    }

    fn enter_folder(&mut self) {
        if self.tab != Tab::Folders {
            return;
        }
        let idx = match self.top_table_state.selected() {
            Some(i) if i < self.top_folders.len() => i,
            _ => return,
        };
        let path = self.top_folders[idx].path.clone();
        self.nav_stack.push(path.clone());
        self.start_new_scan(path);
    }

    fn go_back(&mut self) {
        if self.nav_stack.len() > 1 {
            // Go back to previously visited parent
            self.nav_stack.pop();
        } else {
            // At root of stack — go up to filesystem parent
            let current = self.nav_stack.last().unwrap();
            if let Some(parent) = current.parent() {
                let parent = parent.to_path_buf();
                if parent == *current {
                    return; // already at filesystem root
                }
                self.nav_stack[0] = parent;
            } else {
                return;
            }
        }
        let path = self.nav_stack.last().unwrap().clone();
        self.start_new_scan(path);
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
            Tab::Selected => (&mut self.selected_table_state, self.selected_paths.len()),
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

    fn path_at(&self, idx: usize) -> Option<PathBuf> {
        match self.tab {
            Tab::Folders => self.top_folders.get(idx).map(|f| f.path.clone()),
            Tab::Cruft => self.cruft_items.get(idx).map(|c| c.path.clone()),
            Tab::LargeFiles => self.large_file_items.get(idx).map(|l| l.path.clone()),
            Tab::Selected => self.sorted_selected_paths().into_iter().nth(idx),
            Tab::Overview => None,
        }
    }

    pub fn sorted_selected_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.selected_paths.iter().cloned().collect();
        paths.sort();
        paths
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
