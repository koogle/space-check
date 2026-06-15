use crate::patterns::{self, Category};
use anyhow::Result;
use jwalk::WalkDirGeneric;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CruftEntry {
    pub path: PathBuf,
    pub size: u64,
    pub category: Category,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LargeFileEntry {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopFolderEntry {
    pub path: PathBuf,
    pub total_size: u64,
    pub cruft_size: u64,
}

#[derive(Debug)]
pub enum ScanMessage {
    CruftFound(CruftEntry),
    LargeFileFound(LargeFileEntry),
    ScanTotal(usize),
    TopFolderDone(TopFolderEntry),
    Done { cancelled: bool },
    Error(String),
}

pub fn start_scan(
    root: PathBuf,
    large_file_threshold: u64,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_scan(&root, large_file_threshold, &tx, &cancel) {
            let _ = tx.send(ScanMessage::Error(e.to_string()));
        }
        let _ = tx.send(ScanMessage::Done {
            cancelled: cancel.load(Ordering::Relaxed),
        });
    })
}

fn run_scan(
    root: &Path,
    threshold: u64,
    tx: &Sender<ScanMessage>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let mut top_dirs: Vec<PathBuf> = Vec::new();

    for entry in std::fs::read_dir(root)? {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let _ = tx.send(ScanMessage::Error(format!("{}: {err}", root.display())));
                continue;
            }
        };

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(err) => {
                let _ = tx.send(ScanMessage::Error(format!(
                    "{}: {err}",
                    entry.path().display()
                )));
                continue;
            }
        };

        if ft.is_dir() {
            top_dirs.push(entry.path());
        } else if ft.is_file() {
            match entry.metadata() {
                Ok(meta) => {
                    if meta.len() >= threshold {
                        let _ = tx.send(ScanMessage::LargeFileFound(LargeFileEntry {
                            path: entry.path(),
                            size: meta.len(),
                        }));
                    }
                }
                Err(err) => {
                    let _ = tx.send(ScanMessage::Error(format!(
                        "{}: {err}",
                        entry.path().display()
                    )));
                }
            }
        }
    }

    let _ = tx.send(ScanMessage::ScanTotal(top_dirs.len()));

    for dir in top_dirs {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entry = scan_top_folder(&dir, threshold, tx, cancel);
        if !cancel.load(Ordering::Relaxed) {
            let _ = tx.send(ScanMessage::TopFolderDone(entry));
        }
    }

    Ok(())
}

/// Directories to always skip — large, never reclaimable.
fn should_skip(name: &std::ffi::OsStr) -> bool {
    matches!(name.to_str(), Some(".git" | ".hg" | ".svn"))
}

/// Scan a single top-level folder: detect cruft, large files, compute total size.
/// All stat calls happen inside the process_read_dir callback, which runs on
/// rayon threads in parallel — much faster than accumulating in the
/// single-threaded iterator.
fn scan_top_folder(
    dir: &Path,
    threshold: u64,
    tx: &Sender<ScanMessage>,
    cancel: &Arc<AtomicBool>,
) -> TopFolderEntry {
    let cruft_size = Arc::new(AtomicU64::new(0));
    let file_size = Arc::new(AtomicU64::new(0));

    let walk = WalkDirGeneric::<((), ())>::new(dir)
        .skip_hidden(false)
        .process_read_dir({
            let tx = tx.clone();
            let cruft_size = cruft_size.clone();
            let file_size = file_size.clone();
            let cancel = cancel.clone();
            move |_depth, dir_path, _read_dir_state, children| {
                if cancel.load(Ordering::Relaxed) {
                    children.iter_mut().for_each(|c| {
                        if let Ok(ref mut entry) = c {
                            entry.read_children_path = None;
                        }
                    });
                    return;
                }
                for child in children.iter_mut().flatten() {
                    if child.file_type().is_dir() {
                        let name = child.file_name();

                        // Skip .git/.hg/.svn — never reclaimable, expensive to walk
                        if should_skip(&name) {
                            child.read_children_path = None;
                            continue;
                        }

                        if let Some(pattern) = patterns::match_cruft(&name, dir_path) {
                            child.read_children_path = None;

                            let size = dir_size_fast(&child.path(), &tx);
                            cruft_size.fetch_add(size, Ordering::Relaxed);
                            let _ = tx.send(ScanMessage::CruftFound(CruftEntry {
                                path: child.path(),
                                size,
                                category: pattern.category,
                                description: pattern.description.to_owned(),
                            }));
                        }
                    } else if child.file_type().is_file() {
                        // Stat inside the callback — runs on rayon threads in parallel
                        let path = child.path();
                        match path.metadata() {
                            Ok(meta) => {
                                let sz = meta.len();
                                file_size.fetch_add(sz, Ordering::Relaxed);
                                if sz >= threshold {
                                    let _ = tx.send(ScanMessage::LargeFileFound(LargeFileEntry {
                                        path,
                                        size: sz,
                                    }));
                                }
                            }
                            Err(err) => {
                                let _ = tx
                                    .send(ScanMessage::Error(format!("{}: {err}", path.display())));
                            }
                        }
                    }
                }
            }
        });

    // Just drive the walk to completion — all real work happens in the callback above
    for entry in walk {
        match entry {
            Ok(entry) => {
                if let Some(err) = entry.read_children_error {
                    let _ = tx.send(ScanMessage::Error(err.to_string()));
                }
            }
            Err(err) => {
                let _ = tx.send(ScanMessage::Error(err.to_string()));
            }
        }
    }

    let cruft = cruft_size.load(Ordering::Relaxed);
    let files = file_size.load(Ordering::Relaxed);
    TopFolderEntry {
        path: dir.to_path_buf(),
        total_size: files + cruft,
        cruft_size: cruft,
    }
}

/// Compute directory size with a simple recursive read_dir.
/// Much cheaper than spawning a full jwalk walker per cruft dir,
/// especially for small dirs like __pycache__ (avoids rayon/channel overhead).
fn dir_size_fast(path: &Path, tx: &Sender<ScanMessage>) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(err) => {
                let _ = tx.send(ScanMessage::Error(format!("{}: {err}", dir.display())));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    let _ = tx.send(ScanMessage::Error(format!("{}: {err}", dir.display())));
                    continue;
                }
            };
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(err) => {
                    let _ = tx.send(ScanMessage::Error(format!(
                        "{}: {err}",
                        entry.path().display()
                    )));
                    continue;
                }
            };
            if ft.is_file() {
                match entry.metadata() {
                    Ok(meta) => total += meta.len(),
                    Err(err) => {
                        let _ = tx.send(ScanMessage::Error(format!(
                            "{}: {err}",
                            entry.path().display()
                        )));
                    }
                }
            } else if ft.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("space-check-{name}-{}-{id}", std::process::id()))
    }

    fn write_bytes(path: &Path, len: usize) {
        fs::write(path, vec![b'x'; len]).unwrap();
    }

    #[test]
    fn top_folder_size_includes_files_below_large_file_threshold() {
        let root = temp_path("small-files");
        let folder = root.join("project");
        fs::create_dir_all(folder.join("src")).unwrap();
        write_bytes(&folder.join("README.md"), 5);
        write_bytes(&folder.join("src").join("main.rs"), 7);

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = scan_top_folder(&folder, 1024, &tx, &cancel);

        assert_eq!(entry.total_size, 12);
        assert_eq!(entry.cruft_size, 0);
        assert!(rx
            .try_iter()
            .all(|msg| !matches!(msg, ScanMessage::LargeFileFound(_))));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn top_folder_size_includes_pruned_cruft_once() {
        let root = temp_path("cruft");
        let folder = root.join("project");
        fs::create_dir_all(folder.join("target").join("debug")).unwrap();
        write_bytes(&folder.join("Cargo.toml"), 9);
        write_bytes(&folder.join("target").join("debug").join("app"), 11);

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = scan_top_folder(&folder, 1024, &tx, &cancel);
        let cruft_entries = rx
            .try_iter()
            .filter(|msg| matches!(msg, ScanMessage::CruftFound(_)))
            .count();

        assert_eq!(entry.total_size, 20);
        assert_eq!(entry.cruft_size, 11);
        assert_eq!(cruft_entries, 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scanning_multiple_top_folders_does_not_report_rayon_pool_errors() {
        let root = temp_path("multi-folder");
        fs::create_dir_all(root.join("first")).unwrap();
        fs::create_dir_all(root.join("second")).unwrap();
        write_bytes(&root.join("first").join("a.txt"), 3);
        write_bytes(&root.join("second").join("b.txt"), 4);

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        run_scan(&root, 1024, &tx, &cancel).unwrap();
        let messages: Vec<_> = rx.try_iter().collect();
        let errors: Vec<_> = messages
            .iter()
            .filter_map(|msg| match msg {
                ScanMessage::Error(error) => Some(error.as_str()),
                _ => None,
            })
            .collect();
        let top_folders = messages
            .iter()
            .filter(|msg| matches!(msg, ScanMessage::TopFolderDone(_)))
            .count();

        assert!(errors.is_empty(), "unexpected scan errors: {errors:?}");
        assert_eq!(top_folders, 2);

        fs::remove_dir_all(root).unwrap();
    }
}
