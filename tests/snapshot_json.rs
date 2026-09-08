//! `snapshot --json` tests: warnings into the document, Unicode roots, clean
//! schema (stdout purity is covered by the e2e suite).

mod common;

use common::new_harness;
use whybig::json::{serialize, JsonSnapshotReportV1};

#[test]
fn snapshot_json_is_valid_and_has_expected_fields() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 100]);
    let outcome = h.service.run_snapshot(&h.root, &mut |_| {}).unwrap();
    let doc = JsonSnapshotReportV1::build(&outcome);
    let text = serialize(&doc).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["command"], "snapshot");
    assert_eq!(v["size_kind"], "apparent");
    assert_eq!(v["total_size_bytes"], 100);
    assert_eq!(v["file_count"], 1);
    assert_eq!(v["snapshot_id"], outcome.snapshot_id);
    assert!(v["created_at"].is_string());
    assert!(v["root"].is_string());
    assert_eq!(v["warnings"], serde_json::json!([]));
    assert!(v["scan_duration_ms"].is_u64());
    assert_eq!(v["skipped_count"], 0);
}

#[cfg(unix)]
#[test]
fn snapshot_json_embeds_warnings() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = new_harness();
    h.write("ok/f", &[1u8; 1]);
    let secret = h.abs("secret");
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(secret.join("hidden"), [1u8; 5]).unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();

    let outcome = h.service.run_snapshot(&h.root, &mut |_| {}).unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(outcome.skipped_count >= 1);
    let doc = JsonSnapshotReportV1::build(&outcome);
    let v: serde_json::Value = serde_json::from_str(&serialize(&doc).unwrap()).unwrap();
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        !warnings.is_empty(),
        "warnings must be inside the JSON document"
    );
    assert_eq!(warnings[0]["kind"], "unreadable_dir");
    assert!(warnings[0]["path"].as_str().unwrap().contains("secret"));
    assert_eq!(v["skipped_count"], warnings.len() as u64);
}

#[test]
fn snapshot_json_keeps_unicode_root() {
    let td = tempfile::TempDir::new().unwrap();
    let root = td.path().join("数据-目录");
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = td.path().join("data");
    let mut service = whybig::snapshot::service::SnapshotService::new(data_dir).unwrap();
    let outcome = service.run_snapshot(&root, &mut |_| {}).unwrap();
    let doc = JsonSnapshotReportV1::build(&outcome);
    let v: serde_json::Value = serde_json::from_str(&serialize(&doc).unwrap()).unwrap();
    let root_str = v["root"].as_str().unwrap();
    assert!(
        root_str.contains("数据-目录"),
        "unicode root preserved: {root_str}"
    );
}

#[test]
fn snapshot_json_has_no_ansi() {
    let mut h = new_harness();
    h.write("f", &[1u8; 1]);
    let outcome = h.service.run_snapshot(&h.root, &mut |_| {}).unwrap();
    let text = serialize(&JsonSnapshotReportV1::build(&outcome)).unwrap();
    assert!(!text.contains('\u{1b}'));
}
