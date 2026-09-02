//! Native-picker-owned local library root authorization boundary.

pub mod commands;

mod dto;
mod picker;
mod service;

pub use dto::{
    LibraryRoot, LibraryRootAck, LibraryRootsResponse, PickAndAddLibraryRootRequest,
    PickAndAddLibraryRootResponse, RemoveLibraryRootRequest,
};
pub use picker::{
    LibraryRootPicker, LibraryRootPickerError, LibraryRootPickerFuture, TauriLibraryRootPicker,
};
pub use service::{LibraryRootClock, LibraryRootService, SystemLibraryRootClock};

#[cfg(test)]
mod tests;
