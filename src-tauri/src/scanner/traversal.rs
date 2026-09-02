use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::mpsc;

pub(super) const MAX_TRAVERSAL_DEPTH: usize = 64;
pub(super) const MAX_TRAVERSAL_ENTRIES: u64 = 100_000;
pub(super) const TRAVERSAL_CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Default)]
pub(super) struct ScanCancellation(Arc<AtomicBool>);

impl ScanCancellation {
    pub(super) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(super) struct TraversalFile {
    pub relative_path: PathBuf,
    pub relative_path_text: String,
}

pub(super) enum TraversalItem {
    File(TraversalFile),
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TraversalError {
    Cancelled,
    RootUnavailable,
    WorkLimitExceeded,
    ConsumerStopped,
}

/// Walks a root without following links and sends at most a small bounded batch
/// ahead of the async consumer.
pub(super) fn walk_authorized_root(
    authorized_root: &Path,
    cancellation: &ScanCancellation,
    sender: &mpsc::Sender<TraversalItem>,
) -> Result<(), TraversalError> {
    if cancellation.is_cancelled() {
        return Err(TraversalError::Cancelled);
    }
    reject_reparse_or_symlink(authorized_root).map_err(|()| TraversalError::RootUnavailable)?;
    let canonical_root =
        fs::canonicalize(authorized_root).map_err(|_| TraversalError::RootUnavailable)?;
    if !canonical_root.is_dir() {
        return Err(TraversalError::RootUnavailable);
    }

    let mut stack = vec![(canonical_root.clone(), 0_usize)];
    let mut entries_seen = 0_u64;
    while let Some((directory, depth)) = stack.pop() {
        if cancellation.is_cancelled() {
            return Err(TraversalError::Cancelled);
        }
        if depth > MAX_TRAVERSAL_DEPTH {
            send(sender, TraversalItem::Rejected)?;
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            send(sender, TraversalItem::Rejected)?;
            continue;
        };
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err(TraversalError::Cancelled);
            }
            entries_seen = entries_seen
                .checked_add(1)
                .ok_or(TraversalError::WorkLimitExceeded)?;
            if entries_seen > MAX_TRAVERSAL_ENTRIES {
                return Err(TraversalError::WorkLimitExceeded);
            }
            let Ok(entry) = entry else {
                send(sender, TraversalItem::Rejected)?;
                continue;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                send(sender, TraversalItem::Rejected)?;
                continue;
            };
            if is_reparse_or_symlink(&metadata) {
                send(sender, TraversalItem::Rejected)?;
                continue;
            }
            if metadata.is_dir() {
                match fs::canonicalize(&path) {
                    Ok(canonical) if canonical.starts_with(&canonical_root) => {
                        stack.push((canonical, depth.saturating_add(1)));
                    }
                    _ => send(sender, TraversalItem::Rejected)?,
                }
                continue;
            }
            if !metadata.is_file() {
                send(sender, TraversalItem::Rejected)?;
                continue;
            }
            let Ok(canonical) = fs::canonicalize(&path) else {
                send(sender, TraversalItem::Rejected)?;
                continue;
            };
            if !canonical.starts_with(&canonical_root) {
                send(sender, TraversalItem::Rejected)?;
                continue;
            }
            let Ok(relative_path) = canonical.strip_prefix(&canonical_root) else {
                send(sender, TraversalItem::Rejected)?;
                continue;
            };
            let Some(relative_path_text) = normalized_relative_text(relative_path) else {
                send(sender, TraversalItem::Rejected)?;
                continue;
            };
            send(
                sender,
                TraversalItem::File(TraversalFile {
                    relative_path: relative_path.to_path_buf(),
                    relative_path_text,
                }),
            )?;
        }
    }
    Ok(())
}

fn send(sender: &mpsc::Sender<TraversalItem>, item: TraversalItem) -> Result<(), TraversalError> {
    sender
        .blocking_send(item)
        .map_err(|_| TraversalError::ConsumerStopped)
}

fn normalized_relative_text(path: &Path) -> Option<String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_str()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn reject_reparse_or_symlink(path: &Path) -> Result<(), ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if is_reparse_or_symlink(&metadata) {
        return Err(());
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
