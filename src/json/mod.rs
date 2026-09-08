//! Stable, versioned JSON API for WhyBig (`--json`).
//!
//! These are **independent public structs** — internal domain structs are
//! never serialized directly, so the schema cannot drift with refactors.
//! Conventions (see DESIGN-M3.md §3):
//! - every document carries `"schema_version": 1` and `"command"`;
//! - timestamps are RFC3339 **UTC** (no timezone ambiguity);
//! - all sizes/deltas are integer **bytes** (never "11.8 GB");
//! - `state` is lowercase `added|removed|changed`.

use serde::Serialize;

use crate::diff::{DiffEntry, DiffState, SinceInfo, SnapshotDiff};
use crate::error::{Result, WhyBigError};
use crate::history::HistoryReport;
use crate::inspect::InspectReport;
use crate::retention::PrunePlan;
use crate::scanner::WarningKind;
use crate::snapshot::service::{SnapshotOutcome, StatusReport};
use crate::top::{TopMode, TopReport};

/// Lowercase membership state of a diff entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonStateV1 {
    Added,
    Removed,
    Changed,
}

/// Reference to one snapshot (used in before/after pairs everywhere).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonSnapshotRefV1 {
    pub snapshot_id: i64,
    /// RFC3339 UTC.
    pub created_at: String,
    pub size_bytes: u64,
}

/// `--since` selection info (present only when the pair was chosen by it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonSinceV1 {
    pub requested_seconds: i64,
    /// RFC3339 UTC ideal window start.
    pub requested_target: String,
    /// RFC3339 UTC of the snapshot actually used as `before`.
    pub effective_before: String,
    pub used_earliest: bool,
}

/// One non-fatal scan problem in `snapshot --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonWarningV1 {
    pub path: String,
    pub error: String,
    /// Lowercase kind: `stat_failed` | `unreadable_dir` | `entry_read_failed`.
    pub kind: String,
}

/// One directory's change between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonDeltaEntryV1 {
    pub path: String,
    pub before_size_bytes: u64,
    pub after_size_bytes: u64,
    pub delta_bytes: i128,
    pub state: JsonStateV1,
}

/// `whybig diff --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonDiffReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub root: String,
    pub before: JsonSnapshotRefV1,
    pub after: JsonSnapshotRefV1,
    pub total_before_bytes: u64,
    pub total_after_bytes: u64,
    pub delta_bytes: i128,
    pub grew: Vec<JsonDeltaEntryV1>,
    pub shrank: Vec<JsonDeltaEntryV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<JsonSinceV1>,
}

/// `whybig inspect <path> --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonInspectReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub root: String,
    pub path: String,
    pub before: JsonSnapshotRefV1,
    pub after: JsonSnapshotRefV1,
    pub target_before_bytes: u64,
    pub target_after_bytes: u64,
    pub target_delta_bytes: i128,
    pub other_bytes: i128,
    pub contributors: Vec<JsonDeltaEntryV1>,
}

/// One point of `whybig history --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonHistoryPointV1 {
    pub snapshot_id: i64,
    pub created_at: String,
    pub size_bytes: u64,
    /// `null` for the first point.
    pub delta_from_previous_bytes: Option<i128>,
}

/// `whybig history --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonHistoryReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub root: String,
    pub path: String,
    pub points: Vec<JsonHistoryPointV1>,
    pub total_delta_bytes: i128,
}

/// `whybig top --json` (mode is "grew" or "shrank")
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonTopReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub root: String,
    pub path: String,
    pub mode: String,
    pub before: JsonSnapshotRefV1,
    pub after: JsonSnapshotRefV1,
    pub entries: Vec<JsonDeltaEntryV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<JsonSinceV1>,
}

/// `whybig status --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonStatusReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub initialized: bool,
    pub data_dir: String,
    pub database_path: String,
    pub database_size_bytes: u64,
    pub snapshot_count: i64,
    pub earliest: Option<JsonSnapshotRefV1>,
    pub latest: Option<JsonSnapshotRefV1>,
}

fn rfc3339_utc(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    match DateTime::<Utc>::from_timestamp_millis(ms) {
        Some(dt) => dt.to_rfc3339(),
        None => "1970-01-01T00:00:00Z".to_string(),
    }
}

fn snapshot_ref(id: i64, created_at_ms: i64, size_bytes: u64) -> JsonSnapshotRefV1 {
    JsonSnapshotRefV1 {
        snapshot_id: id,
        created_at: rfc3339_utc(created_at_ms),
        size_bytes,
    }
}

fn state_of(s: DiffState) -> JsonStateV1 {
    match s {
        DiffState::Added => JsonStateV1::Added,
        DiffState::Removed => JsonStateV1::Removed,
        DiffState::Changed => JsonStateV1::Changed,
    }
}

fn entry_of(e: &DiffEntry) -> JsonDeltaEntryV1 {
    JsonDeltaEntryV1 {
        path: e.path.clone(),
        before_size_bytes: e.before_size,
        after_size_bytes: e.after_size,
        delta_bytes: e.delta,
        state: state_of(e.state),
    }
}

fn since_of(s: SinceInfo) -> JsonSinceV1 {
    JsonSinceV1 {
        requested_seconds: s.requested_seconds,
        requested_target: rfc3339_utc(s.requested_target_ms),
        effective_before: rfc3339_utc(s.effective_before_ms),
        used_earliest: s.used_earliest,
    }
}

fn warning_kind_str(k: WarningKind) -> &'static str {
    match k {
        WarningKind::StatFailed => "stat_failed",
        WarningKind::UnreadableDir => "unreadable_dir",
        WarningKind::EntryReadFailed => "entry_read_failed",
    }
}

impl JsonDiffReportV1 {
    pub fn build(d: &SnapshotDiff) -> Self {
        Self {
            schema_version: 1,
            command: "diff".to_string(),
            root: d.root.clone(),
            before: snapshot_ref(d.before.id, d.before.created_at_ms, d.total_before),
            after: snapshot_ref(d.after.id, d.after.created_at_ms, d.total_after),
            total_before_bytes: d.total_before,
            total_after_bytes: d.total_after,
            delta_bytes: d.total_delta,
            grew: d.grew.iter().map(entry_of).collect(),
            shrank: d.shrank.iter().map(entry_of).collect(),
            since: d.since.map(since_of),
        }
    }
}

impl JsonInspectReportV1 {
    pub fn build(r: &InspectReport) -> Self {
        Self {
            schema_version: 1,
            command: "inspect".to_string(),
            root: r.root.clone(),
            path: r.target.clone(),
            before: snapshot_ref(r.before.id, r.before.created_at_ms, r.target_before),
            after: snapshot_ref(r.after.id, r.after.created_at_ms, r.target_after),
            target_before_bytes: r.target_before,
            target_after_bytes: r.target_after,
            target_delta_bytes: r.target_delta,
            other_bytes: r.other,
            contributors: r.contributors.iter().map(entry_of).collect(),
        }
    }
}

impl JsonHistoryReportV1 {
    pub fn build(r: &HistoryReport) -> Self {
        Self {
            schema_version: 1,
            command: "history".to_string(),
            root: r.root.to_string_lossy().into_owned(),
            path: r.target.to_string_lossy().into_owned(),
            points: r
                .points
                .iter()
                .map(|p| JsonHistoryPointV1 {
                    snapshot_id: p.snapshot_id,
                    created_at: rfc3339_utc(p.created_at_ms),
                    size_bytes: p.size,
                    delta_from_previous_bytes: p.delta_from_previous,
                })
                .collect(),
            total_delta_bytes: r.total_delta,
        }
    }
}

impl JsonTopReportV1 {
    pub fn build(r: &TopReport) -> Self {
        let mode = match r.mode {
            TopMode::Growth => "grew",
            TopMode::Shrink => "shrank",
        };
        Self {
            schema_version: 1,
            command: "top".to_string(),
            root: r.root.clone(),
            path: r.scope.clone(),
            mode: mode.to_string(),
            before: snapshot_ref(
                r.before.id,
                r.before.created_at_ms,
                r.before.total_size_u64(),
            ),
            after: snapshot_ref(r.after.id, r.after.created_at_ms, r.after.total_size_u64()),
            entries: r.entries.iter().map(entry_of).collect(),
            since: r.since.map(since_of),
        }
    }
}

impl JsonStatusReportV1 {
    pub fn build(r: &StatusReport) -> Self {
        Self {
            schema_version: 1,
            command: "status".to_string(),
            initialized: r.initialized,
            data_dir: r.data_dir.to_string_lossy().into_owned(),
            database_path: r.db_path.to_string_lossy().into_owned(),
            database_size_bytes: r.db_size,
            snapshot_count: r.snapshot_count,
            earliest: r
                .earliest
                .as_ref()
                .map(|e| snapshot_ref(e.id, e.created_at_ms, e.total_size_u64())),
            latest: r
                .latest
                .as_ref()
                .map(|l| snapshot_ref(l.id, l.created_at_ms, l.total_size_u64())),
        }
    }
}

/// `whybig snapshot <path> --json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonSnapshotReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub snapshot_id: i64,
    /// RFC3339 UTC.
    pub created_at: String,
    pub root: String,
    pub size_kind: String,
    pub total_size_bytes: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub scan_duration_ms: u64,
    pub skipped_count: u64,
    pub warnings: Vec<JsonWarningV1>,
}

impl JsonSnapshotReportV1 {
    pub fn build(o: &SnapshotOutcome) -> Self {
        Self {
            schema_version: 1,
            command: "snapshot".to_string(),
            snapshot_id: o.snapshot_id,
            created_at: rfc3339_utc(o.created_at_ms),
            root: o.root_path.to_string_lossy().into_owned(),
            size_kind: "apparent".to_string(),
            total_size_bytes: o.total_size,
            file_count: o.file_count,
            dir_count: o.dir_count,
            scan_duration_ms: o.elapsed.as_millis() as u64,
            skipped_count: o.skipped_count,
            warnings: o
                .warnings
                .iter()
                .map(|w| JsonWarningV1 {
                    path: w.path.to_string_lossy().into_owned(),
                    error: w.error.clone(),
                    kind: warning_kind_str(w.kind).to_string(),
                })
                .collect(),
        }
    }
}

/// Per-root entry of `whybig prune --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonPruneRootV1 {
    pub root: String,
    pub snapshot_count: usize,
    pub keep: Vec<i64>,
    pub remove: Vec<i64>,
}

/// `whybig prune --json` / `prune --apply --json`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonPruneReportV1 {
    pub schema_version: u8,
    pub command: String,
    pub applied: bool,
    pub policy: JsonRetentionPolicyV1,
    pub snapshots_before: i64,
    pub snapshots_keep: i64,
    pub snapshots_remove: i64,
    /// RFC3339 UTC of the oldest snapshot that would be / was removed.
    pub oldest_removed: Option<String>,
    pub database_size_bytes: u64,
    pub roots: Vec<JsonPruneRootV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JsonRetentionPolicyV1 {
    pub recent_days: u32,
    pub daily_days: u32,
    pub weekly_days: u32,
}

impl JsonPruneReportV1 {
    /// `oldest_removed_ms` is `None` when nothing would be removed.
    pub fn build(
        plan: &PrunePlan,
        applied: bool,
        snapshots_before: i64,
        database_size_bytes: u64,
        oldest_removed_ms: Option<i64>,
    ) -> Self {
        Self {
            schema_version: 1,
            command: "prune".to_string(),
            applied,
            policy: JsonRetentionPolicyV1 {
                recent_days: plan.policy.recent_days,
                daily_days: plan.policy.daily_days,
                weekly_days: plan.policy.weekly_days,
            },
            snapshots_before,
            snapshots_keep: plan.total_keep() as i64,
            snapshots_remove: plan.total_remove() as i64,
            oldest_removed: oldest_removed_ms.map(rfc3339_utc),
            database_size_bytes,
            roots: plan
                .roots
                .iter()
                .map(|r| JsonPruneRootV1 {
                    root: r.root.clone(),
                    snapshot_count: r.snapshot_count,
                    keep: r.keep_ids(),
                    remove: r.remove.clone(),
                })
                .collect(),
        }
    }
}

/// Serialize a report to compact JSON. The CLI prints exactly this value in
/// JSON mode, so stdout stays single-document and parseable.
pub fn serialize<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| WhyBigError::Json(e.to_string()))
}
