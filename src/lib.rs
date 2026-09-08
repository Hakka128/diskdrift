//! DiskDrift — a disk-space *growth* tracker.
//!
//! Disk analyzers tell you what's big. DiskDrift tells you what **got** big.
//!
//! Milestone 1 provides:
//! - `diskdrift init`      — create/migrate the DiskDrift data directory + database
//! - `diskdrift snapshot`  — record a directory-size snapshot (scanner + storage)
//! - `diskdrift status`    — inspect the stored snapshots
//!
//! The scanner is deliberately decoupled from SQLite: `scanner::scan` walks the
//! filesystem and returns pure data, which `snapshot::service` hands to
//! `storage::Storage` for transactional persistence.

pub mod benchtree;
pub mod cli;
pub mod config;
pub mod diff;
pub mod error;
pub mod history;
pub mod inspect;
pub mod json;
pub mod output;
pub mod pathutil;
pub mod retention;
pub mod scanner;
pub mod since;
pub mod snapshot;
pub mod storage;
pub mod top;
