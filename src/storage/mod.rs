//! SQLite persistence for WhyBig snapshots.

pub mod database;
pub mod migrations;
pub mod models;

pub use database::Storage;
