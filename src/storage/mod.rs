//! SQLite persistence for DiskDrift snapshots.

pub mod database;
pub mod migrations;
pub mod models;

pub use database::Storage;
