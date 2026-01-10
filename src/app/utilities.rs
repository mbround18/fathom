//! Utility functions for UI operations and data formatting.

use crate::protocol::DiskUsage;

/// Match PVC names to disk usage data by namespace and name.
/// Returns tuples of (pvc_name, used_bytes, provisioned_bytes).
pub fn pod_pvc_usage(
    namespace: &str,
    pvcs: &[String],
    disk_items: &[DiskUsage],
) -> Vec<(String, Option<u64>, u64)> {
    pvcs.iter()
        .map(|pvc| {
            let disk_match = disk_items
                .iter()
                .find(|d| d.namespace == namespace && d.name == *pvc);

            match disk_match {
                Some(disk) => (pvc.clone(), disk.used_bytes, disk.provisioned_bytes),
                None => (pvc.clone(), None, 0),
            }
        })
        .collect()
}

/// Format PVC storage information as human-readable strings.
pub fn format_pvc_storage_lines(
    namespace: &str,
    pvcs: &[String],
    disk_items: &[DiskUsage],
) -> Vec<String> {
    pod_pvc_usage(namespace, pvcs, disk_items)
        .iter()
        .map(|(name, used, provisioned)| {
            let used_str = used
                .map(|u| format_bytes(u))
                .unwrap_or_else(|| "(unknown)".to_string());
            let prov_str = format_bytes(*provisioned);
            format!("{} - {} / {}", name, used_str, prov_str)
        })
        .collect()
}

/// Format bytes as human-readable string (B, KiB, MiB, GiB, etc.)
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        // For bytes, don't show decimal places
        format!("{} {}", size as u64, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pod_pvc_usage_with_match() {
        let pvcs = vec!["data".to_string()];
        let disk_items = vec![DiskUsage {
            namespace: "myns".to_string(),
            name: "data".to_string(),
            provisioned_bytes: 10 * 1024 * 1024 * 1024,
            used_bytes: Some(2 * 1024 * 1024 * 1024),
            state: None,
        }];

        let result = pod_pvc_usage("myns", &pvcs, &disk_items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "data");
        assert_eq!(result[0].1, Some(2 * 1024 * 1024 * 1024));
        assert_eq!(result[0].2, 10 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_pod_pvc_usage_no_match() {
        let pvcs = vec!["cache".to_string()];
        let disk_items: Vec<DiskUsage> = vec![];

        let result = pod_pvc_usage("myns", &pvcs, &disk_items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "cache");
        assert_eq!(result[0].1, None);
        assert_eq!(result[0].2, 0);
    }

    #[test]
    fn test_format_pvc_storage_lines() {
        let pvcs = vec!["data".to_string()];
        let disk_items = vec![DiskUsage {
            namespace: "myns".to_string(),
            name: "data".to_string(),
            provisioned_bytes: 10 * 1024 * 1024 * 1024,
            used_bytes: Some(2 * 1024 * 1024 * 1024),
            state: None,
        }];

        let lines = format_pvc_storage_lines("myns", &pvcs, &disk_items);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "data - 2.0 GiB / 10.0 GiB");
    }
}
