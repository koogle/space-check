use crate::patterns::{self, Category};
use anyhow::Result;
use jwalk::WalkDirGeneric;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
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

#[derive(Debug)]
pub enum ScanMessage {
    CruftFound(CruftEntry),
    LargeFileFound(LargeFileEntry),
    Progress(PathBuf),
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

fn run_scan(root: &Path, large_file_threshold: u64, tx: &Sender<ScanMessage>) -> Result<()> {
    let tx_progress = tx.clone();
    let tx_cruft = tx.clone();
    let threshold = large_file_threshold;
    let tx_large = tx.clone();

    let walk = WalkDirGeneric::<((), ())>::new(root)
        .skip_hidden(false)
        .process_read_dir(move |_depth, dir_path, _read_dir_state, children| {
            // Send progress for current directory
            let _ = tx_progress.send(ScanMessage::Progress(dir_path.to_path_buf()));

            for child in children.iter_mut().flatten() {
                let child_path = child.path();
                let file_name = child.file_name();

                // Check if this is a cruft directory
                if child.file_type().is_dir() {
                    if let Some(pattern) = patterns::match_cruft(&file_name, dir_path) {
                        // Skip recursing into this cruft dir
                        child.read_children_path = None;

                        // Compute size and report
                        let size = dir_size(&child_path);
                        let _ = tx_cruft.send(ScanMessage::CruftFound(CruftEntry {
                            path: child_path,
                            size,
                            category: pattern.category,
                            description: pattern.description,
                        }));
                    }
                } else if child.file_type().is_file() {
                    // Check large file
                    if let Ok(meta) = child_path.metadata() {
                        let size = meta.len();
                        if size >= threshold {
                            let _ = tx_large.send(ScanMessage::LargeFileFound(LargeFileEntry {
                                path: child_path,
                                size,
                            }));
                        }
                    }
                }
            }
        });

    for _entry in walk {
        // Walk is driven by iterating; results sent via channels above
    }

    Ok(())
}

/// Compute total size of a directory tree using jwalk for parallelism.
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
