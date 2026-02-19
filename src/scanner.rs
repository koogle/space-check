use crate::patterns::{self, Category};
use anyhow::Result;
use jwalk::WalkDirGeneric;
use rayon::prelude::*;
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
    Done,
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
        let _ = tx.send(ScanMessage::Done);
    })
}

fn run_scan(root: &Path, threshold: u64, tx: &Sender<ScanMessage>, cancel: &Arc<AtomicBool>) -> Result<()> {
    let mut top_dirs: Vec<PathBuf> = Vec::new();

    for entry in std::fs::read_dir(root)?.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let ft = entry.file_type();
        if ft.as_ref().map_or(false, |ft| ft.is_dir()) {
            top_dirs.push(entry.path());
        } else if ft.as_ref().map_or(false, |ft| ft.is_file()) {
            if let Ok(meta) = entry.metadata() {
                if meta.len() >= threshold {
                    let _ = tx.send(ScanMessage::LargeFileFound(LargeFileEntry {
                        path: entry.path(),
                        size: meta.len(),
                    }));
                }
            }
        }
    }

    let _ = tx.send(ScanMessage::ScanTotal(top_dirs.len()));

    let cancel2 = cancel.clone();
    top_dirs.par_iter().for_each(|dir| {
        if cancel2.load(Ordering::Relaxed) {
            return;
        }
        let entry = scan_top_folder(dir, threshold, tx, &cancel2);
        let _ = tx.send(ScanMessage::TopFolderDone(entry));
    });

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
fn scan_top_folder(dir: &Path, threshold: u64, tx: &Sender<ScanMessage>, cancel: &Arc<AtomicBool>) -> TopFolderEntry {
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

                            let size = dir_size_fast(&child.path());
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
                        if let Ok(meta) = child.path().metadata() {
                            let sz = meta.len();
                            file_size.fetch_add(sz, Ordering::Relaxed);
                            if sz >= threshold {
                                let _ = tx.send(ScanMessage::LargeFileFound(LargeFileEntry {
                                    path: child.path(),
                                    size: sz,
                                }));
                            }
                        }
                    }
                }
            }
        });

    // Just drive the walk to completion — all real work happens in the callback above
    for _ in walk {}

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
fn dir_size_fast(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            } else if ft.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}
