//! Native-picker-owned local library root authorization boundary.

pub mod commands;

mod dto;
mod picker;
mod service;

pub use dto::{
    EnrichedTrackTagView, LibraryRoot, LibraryRootAck, LibraryRootsResponse, ListTracksRequest,
    MAX_TRACK_PAGE_SIZE, PickAndAddLibraryRootRequest, PickAndAddLibraryRootResponse,
    RemoveLibraryRootRequest, TrackAvailability, TrackAvailabilityFilter, TrackFilters,
    TrackMatchStatus, TrackMetadataProvider, TrackSort, TrackTagView, TrackView, TracksPage,
};
pub use picker::{
    LibraryRootPicker, LibraryRootPickerError, LibraryRootPickerFuture, TauriLibraryRootPicker,
};
pub use service::{
    LibraryRootClock, LibraryRootService, SystemLibraryRootClock, TrackCatalog, TrackCatalogFuture,
    TrackCatalogQuery, TrackCatalogService,
};

#[cfg(test)]
mod tests;
