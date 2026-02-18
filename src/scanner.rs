use crate::patterns::{self, Category};
use anyhow::Result;
use jwalk::WalkDirGeneric;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone)]
pub struct CruftEntry {
    pub path: PathBuf,
    pub size: u64,
    pub category: Category,
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct LargeFileEntry {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct TopFolderEntry {
    pub path: PathBuf,
    pub total_size: u64,
    pub cruft_size: u64,
}

#[derive(Debug)]
pub enum ScanMessage {
    CruftFound(CruftEntry),
    LargeFileFound(LargeFileEntry),
    /// Total number of top-level folders to scan
    ScanTotal(usize),
    /// A top-level folder finished scanning
    TopFolderDone(TopFolderEntry),
    Done,
    Error(String),
}

pub fn start_scan(
    root: PathBuf,
    large_file_threshold: u64,
    tx: Sender<ScanMessage>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_scan(&root, large_file_threshold, &tx) {
            let _ = tx.send(ScanMessage::Error(e.to_string()));
        }
        let _ = tx.send(ScanMessage::Done);
    })
}

fn run_scan(root: &Path, threshold: u64, tx: &Sender<ScanMessage>) -> Result<()> {
    let mut top_dirs: Vec<PathBuf> = Vec::new();

    // Enumerate top-level entries
    for entry in std::fs::read_dir(root)?.flatten() {
        let ft = entry.file_type();
        if ft.as_ref().map_or(false, |ft| ft.is_dir()) {
            top_dirs.push(entry.path());
        } else if ft.as_ref().map_or(false, |ft| ft.is_file()) {
            // Check root-level large files
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

    // Scan each top-level dir in parallel
    top_dirs.par_iter().for_each(|dir| {
        let entry = scan_top_folder(dir, threshold, tx);
        let _ = tx.send(ScanMessage::TopFolderDone(entry));
    });

    Ok(())
}

/// Scan a single top-level folder: detect cruft, large files, compute total size.
fn scan_top_folder(dir: &Path, threshold: u64, tx: &Sender<ScanMessage>) -> TopFolderEntry {
    let cruft_size = Arc::new(AtomicU64::new(0));
    let cruft_size_cb = cruft_size.clone();

    let walk = WalkDirGeneric::<((), ())>::new(dir)
        .skip_hidden(false)
        .process_read_dir({
            let tx = tx.clone();
            move |_depth, dir_path, _read_dir_state, children| {
                for child in children.iter_mut().flatten() {
                    if child.file_type().is_dir() {
                        let file_name = child.file_name();
                        if let Some(pattern) = patterns::match_cruft(&file_name, dir_path) {
                            // Skip recursing into this cruft dir
                            child.read_children_path = None;

                            let size = dir_size(&child.path());
                            cruft_size_cb.fetch_add(size, Ordering::Relaxed);
                            let _ = tx.send(ScanMessage::CruftFound(CruftEntry {
                                path: child.path(),
                                size,
                                category: pattern.category,
                                description: pattern.description,
                            }));
                        }
                    }
                }
            }
        });

    // Drive the walk and accumulate file sizes (non-cruft files only,
    // since cruft dirs are skipped and counted separately via dir_size)
    let mut file_size: u64 = 0;
    for entry in walk {
        if let Ok(e) = entry {
            if e.file_type().is_file() {
                if let Ok(meta) = e.path().metadata() {
                    let sz = meta.len();
                    file_size += sz;
                    if sz >= threshold {
                        let _ = tx.send(ScanMessage::LargeFileFound(LargeFileEntry {
                            path: e.path(),
                            size: sz,
                        }));
                    }
                }
            }
        }
    }

    let cruft = cruft_size.load(Ordering::Relaxed);
    TopFolderEntry {
        path: dir.to_path_buf(),
        total_size: file_size + cruft,
        cruft_size: cruft,
    }
}

/// Compute total size of a directory tree.
fn dir_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    for entry in jwalk::WalkDir::new(path).skip_hidden(false) {
        if let Ok(e) = entry {
            if e.file_type().is_file() {
                if let Ok(meta) = e.path().metadata() {
                    total += meta.len();
                }
            }
        }
    }
    total
}
