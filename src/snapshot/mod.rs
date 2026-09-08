//! The snapshot workflow: tie the filesystem scan to durable storage.

pub mod service;

pub use service::{SnapshotOutcome, SnapshotService, StatusReport};
