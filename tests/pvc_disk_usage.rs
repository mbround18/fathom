use fathom::app::utilities::{format_pvc_storage_lines, pod_pvc_usage};
use fathom::protocol::DiskUsage;

#[test]
fn test_pvc_usage_match_and_format() {
    let pvcs = vec!["data".to_string()];
    let disk_items = vec![DiskUsage {
        namespace: "myns".to_string(),
        name: "data".to_string(),
        provisioned_bytes: 10 * 1024 * 1024 * 1024, // 10 GiB
        used_bytes: Some(2 * 1024 * 1024 * 1024),   // 2 GiB
        state: None,
    }];

    let mapped = pod_pvc_usage("myns", &pvcs, &disk_items);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].0, "data");
    assert_eq!(mapped[0].1, Some(2 * 1024 * 1024 * 1024));
    assert_eq!(mapped[0].2, 10 * 1024 * 1024 * 1024);

    let lines = format_pvc_storage_lines("myns", &pvcs, &disk_items);
    assert_eq!(lines, vec!["data - 2.0 GiB / 10.0 GiB"]);
}

#[test]
fn test_pvc_usage_missing_disk_data() {
    let pvcs = vec!["cache".to_string()];
    let disk_items: Vec<DiskUsage> = vec![];

    let mapped = pod_pvc_usage("myns", &pvcs, &disk_items);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].0, "cache");
    assert_eq!(mapped[0].1, None);
    assert_eq!(mapped[0].2, 0);

    let lines = format_pvc_storage_lines("myns", &pvcs, &disk_items);
    assert_eq!(lines, vec!["cache - (unknown) / 0 B"]);
}

#[test]
fn test_pvc_namespace_mismatch() {
    let pvcs = vec!["data".to_string()];
    let disk_items = vec![DiskUsage {
        namespace: "other".to_string(),
        name: "data".to_string(),
        provisioned_bytes: 1024,
        used_bytes: Some(512),
        state: None,
    }];

    let mapped = pod_pvc_usage("myns", &pvcs, &disk_items);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].0, "data");
    assert_eq!(mapped[0].1, None);
    assert_eq!(mapped[0].2, 0);

    let lines = format_pvc_storage_lines("myns", &pvcs, &disk_items);
    assert_eq!(lines, vec!["data - (unknown) / 0 B"]);
}
