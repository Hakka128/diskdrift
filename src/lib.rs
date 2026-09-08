//! WhyBig — a disk-space *growth* tracker.
//!
//! Disk analyzers tell you what's big. WhyBig tells you what **got** big.
//!
//! Milestone 1 provides:
//! - `whybig init`      — create/migrate the WhyBig data directory + database
//! - `whybig snapshot`  — record a directory-size snapshot (scanner + storage)
//! - `whybig status`    — inspect the stored snapshots
//!
//! The scanner is deliberately decoupled from SQLite: `scanner::scan` walks the
//! filesystem and returns pure data, which `snapshot::service` hands to
//! `storage::Storage` for transactional persistence.

pub mod cli;
pub mod config;
pub mod diff;
pub mod error;
pub mod inspect;
pub mod output;
pub mod pathutil;
pub mod scanner;
pub mod snapshot;
pub mod storage;
