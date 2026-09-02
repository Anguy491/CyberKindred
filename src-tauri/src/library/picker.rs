use std::{future::Future, path::PathBuf, pin::Pin};

use tauri_plugin_dialog::DialogExt;

pub type LibraryRootPickerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<PathBuf>, LibraryRootPickerError>> + Send + 'a>>;

/// Native directory-selection capability injected into the library service.
pub trait LibraryRootPicker: Send + Sync {
    fn pick_directory(&self) -> LibraryRootPickerFuture<'_>;
}

/// Opaque picker failure. It deliberately retains no path or platform error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LibraryRootPickerError;

/// Production adapter for the pinned Tauri dialog plugin.
pub struct TauriLibraryRootPicker {
    app_handle: tauri::AppHandle,
}

impl TauriLibraryRootPicker {
    #[must_use]
    pub const fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl LibraryRootPicker for TauriLibraryRootPicker {
    fn pick_directory(&self) -> LibraryRootPickerFuture<'_> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.app_handle
            .dialog()
            .file()
            .set_title("选择本地曲库")
            .pick_folder(move |selection| {
                let selected = selection
                    .map(|path| path.into_path().map_err(|_| LibraryRootPickerError))
                    .transpose();
                let _ = sender.send(selected);
            });
        Box::pin(async move { receiver.await.map_err(|_| LibraryRootPickerError)? })
    }
}
