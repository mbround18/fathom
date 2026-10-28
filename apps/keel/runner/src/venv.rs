//! What is installed in `/opt/keel-runner/venv`: the ansible-core and Python versions, read from
//! the files `uv` wrote, never by running anything.

use std::path::Path;

/// Reported when a version cannot be read. It never meets a minimum.
pub const UNKNOWN: &str = "unknown";

/// The ansible-core version from its `ansible_core-<version>.dist-info` directory.
pub fn ansible_core_version(venv: &Path) -> String {
    let Ok(lib) = std::fs::read_dir(venv.join("lib")) else {
        return UNKNOWN.to_owned();
    };
    let mut found = Vec::new();
    for py in lib.flatten() {
        let Ok(site) = std::fs::read_dir(py.path().join("site-packages")) else {
            continue;
        };
        for entry in site.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(v) = name
                .strip_prefix("ansible_core-")
                .and_then(|r| r.strip_suffix(".dist-info"))
                && !v.is_empty()
                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
            {
                found.push(v.to_owned());
            }
        }
    }
    // Two installed versions is not a state to guess about.
    match found.as_slice() {
        [one] => one.clone(),
        _ => UNKNOWN.to_owned(),
    }
}

/// The Python version from `pyvenv.cfg` (`version = …` or `version_info = …`).
pub fn python_version(venv: &Path) -> String {
    let Ok(cfg) = std::fs::read_to_string(venv.join("pyvenv.cfg")) else {
        return UNKNOWN.to_owned();
    };
    cfg.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| matches!(k.trim(), "version" | "version_info"))
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
        .unwrap_or_else(|| UNKNOWN.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_come_from_dist_info_and_pyvenv_cfg() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ansible_core_version(dir.path()), UNKNOWN);
        assert_eq!(python_version(dir.path()), UNKNOWN);
        std::fs::create_dir_all(
            dir.path()
                .join("lib/python3.12/site-packages/ansible_core-2.21.5.dist-info"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("pyvenv.cfg"),
            "home = /usr/bin\nversion_info = 3.12.3\n",
        )
        .unwrap();
        assert_eq!(ansible_core_version(dir.path()), "2.21.5");
        assert_eq!(python_version(dir.path()), "3.12.3");
    }
}
