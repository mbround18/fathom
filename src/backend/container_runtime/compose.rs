//! 048-docker-management: the file a compose project was started from.
//!
//! Compose records the files it was invoked with on every container it creates, so the project's
//! definition is discoverable without guessing at the operator's directory layout. What is left to
//! do is decide, honestly, which of four things is true of that path: it is there and makes sense,
//! it is gone, it is there and cannot be read, or the project never declared one at all.
//!
//! This module only ever reads. It never writes, moves, renames or deletes the operator's file, and
//! it never rewrites a file it failed to parse into something it would prefer (Principle VI,
//! FR-047): a project whose definition is corrupt reports that and keeps running on what the daemon
//! already has. The file on disk after an inspection is byte-identical to the file before it.
//!
//! `Unreadable` carries the parser's own words rather than a summary of them. "mapping values are
//! not allowed in this context at line 12 column 9" is a sentence an operator can act on; "invalid
//! compose file" is not, and would be the same sentence for a permission denial, a truncated write
//! and a tab where a space belongs.

use std::io::{Error, ErrorKind};

use super::grouping::DefinitionProbe;
use crate::protocol::v1::DefinitionFileState;

/// A definition file is a page or two of YAML. Anything past this is not one, and slurping it to
/// find that out would be the one place this feature could be made to read a DVD image
/// (Principle V). The cap is on the file, not on the operator: nothing is modified either way.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Where definition files are read from.
///
/// A seam, because every interesting case here is a filesystem the test cannot arrange reliably: a
/// socket, a file whose permissions deny the reader, bytes that are not UTF-8. The production
/// implementation is `Disk`, and it is the only one that touches a real path.
pub trait DefinitionSource {
    /// The file's contents, or why they could not be had.
    fn read(&self, path: &str) -> Result<String, Error>;
}

/// The operator's actual filesystem, read-only.
#[derive(Debug, Clone, Copy, Default)]
pub struct Disk;

impl DefinitionSource for Disk {
    fn read(&self, path: &str) -> Result<String, Error> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the declared path is not a file",
            ));
        }
        if meta.len() > MAX_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "the file is {} bytes; a definition file over {MAX_BYTES} bytes is not read",
                    meta.len()
                ),
            ));
        }
        std::fs::read_to_string(path)
    }
}

/// Inspect a declared definition path on the real filesystem.
pub fn inspect(path: &str) -> DefinitionFileState {
    inspect_with(&Disk, path)
}

/// Inspect a declared definition path through a given source.
///
/// `Missing` is reserved for the path not being there. Every other failure — a directory, a denied
/// read, bytes that are not text, YAML that does not parse — is `Unreadable`, because "missing" and
/// "there but unreadable" lead the operator to different places and conflating them would send them
/// to the wrong one.
pub fn inspect_with(source: &dyn DefinitionSource, path: &str) -> DefinitionFileState {
    let text = match source.read(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return DefinitionFileState::Missing,
        Err(e) => return DefinitionFileState::Unreadable(e.to_string()),
    };
    match serde_yaml::from_str::<serde_yaml::Value>(&text) {
        Err(e) => DefinitionFileState::Unreadable(e.to_string()),
        Ok(serde_yaml::Value::Mapping(_)) => DefinitionFileState::Present,
        Ok(serde_yaml::Value::Null) => {
            DefinitionFileState::Unreadable("the file is empty".to_owned())
        }
        Ok(_) => DefinitionFileState::Unreadable(
            "the file parses as YAML but its top level is not a mapping of compose sections"
                .to_owned(),
        ),
    }
}

impl DefinitionProbe for Disk {
    fn inspect(&self, path: &str) -> DefinitionFileState {
        inspect(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// What a fake filesystem holds at one path: text, or the error reading it produces.
    type Entry = Result<&'static str, (ErrorKind, &'static str)>;

    /// A filesystem that only contains what a test put in it.
    struct Fake(HashMap<&'static str, Entry>);

    impl Fake {
        fn new(entries: &[(&'static str, Entry)]) -> Self {
            Self(entries.iter().copied().collect())
        }
    }

    impl DefinitionSource for Fake {
        fn read(&self, path: &str) -> Result<String, Error> {
            match self.0.get(path) {
                Some(Ok(text)) => Ok((*text).to_owned()),
                Some(Err((kind, msg))) => Err(Error::new(*kind, *msg)),
                None => Err(Error::new(ErrorKind::NotFound, "no such file or directory")),
            }
        }
    }

    const COMPOSE: &str = "services:\n  app:\n    image: busybox\n    command: sleep 3600\n";

    #[test]
    fn a_readable_compose_file_is_present() {
        let fs = Fake::new(&[("/srv/shop/compose.yaml", Ok(COMPOSE))]);
        assert_eq!(
            inspect_with(&fs, "/srv/shop/compose.yaml"),
            DefinitionFileState::Present
        );
    }

    #[test]
    fn a_path_that_is_not_there_is_missing_not_unreadable() {
        let fs = Fake::new(&[]);
        assert_eq!(
            inspect_with(&fs, "/srv/gone/compose.yaml"),
            DefinitionFileState::Missing
        );
    }

    #[test]
    fn a_denied_read_keeps_the_operating_system_words() {
        let fs = Fake::new(&[(
            "/root/compose.yaml",
            Err((
                ErrorKind::PermissionDenied,
                "permission denied (os error 13)",
            )),
        )]);
        let DefinitionFileState::Unreadable(detail) = inspect_with(&fs, "/root/compose.yaml")
        else {
            panic!("a denied read is unreadable, not missing");
        };
        assert!(
            detail.contains("permission denied (os error 13)"),
            "the reason must be the system's own, not a summary: {detail}"
        );
    }

    #[test]
    fn unparseable_yaml_carries_the_parsers_own_words_including_the_position() {
        // A tab where YAML requires a space: the canonical corrupt-definition case, and one whose
        // message is only useful if it keeps the line and column.
        let fs = Fake::new(&[("/srv/bad/compose.yaml", Ok("services:\n\tapp: busybox\n"))]);
        let DefinitionFileState::Unreadable(detail) = inspect_with(&fs, "/srv/bad/compose.yaml")
        else {
            panic!("a file that does not parse is unreadable");
        };
        assert!(
            detail.contains("line") && detail.contains("column"),
            "the parser's position is the actionable part: {detail}"
        );
    }

    #[test]
    fn an_empty_file_says_so_rather_than_reporting_a_project_with_no_services() {
        let fs = Fake::new(&[("/srv/empty/compose.yaml", Ok("\n#  nothing yet\n"))]);
        assert_eq!(
            inspect_with(&fs, "/srv/empty/compose.yaml"),
            DefinitionFileState::Unreadable("the file is empty".to_owned())
        );
    }

    #[test]
    fn valid_yaml_that_is_not_a_mapping_is_not_a_definition() {
        let fs = Fake::new(&[("/srv/list/compose.yaml", Ok("- app\n- db\n"))]);
        let DefinitionFileState::Unreadable(detail) = inspect_with(&fs, "/srv/list/compose.yaml")
        else {
            panic!("a YAML sequence is not a compose file");
        };
        assert!(detail.contains("not a mapping"), "{detail}");
    }

    #[test]
    fn bytes_that_are_not_text_are_unreadable_rather_than_a_panic() {
        let fs = Fake::new(&[(
            "/srv/bin/compose.yaml",
            Err((ErrorKind::InvalidData, "stream did not contain valid UTF-8")),
        )]);
        assert!(matches!(
            inspect_with(&fs, "/srv/bin/compose.yaml"),
            DefinitionFileState::Unreadable(_)
        ));
    }

    #[test]
    fn the_real_filesystem_reads_a_file_without_changing_it() {
        let dir = std::env::temp_dir().join(format!("fathom-compose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("compose.yaml");
        std::fs::write(&path, COMPOSE).expect("write fixture");
        let before = std::fs::read(&path).expect("read fixture");

        let state = inspect(path.to_str().expect("utf-8 temp path"));

        assert_eq!(state, DefinitionFileState::Present);
        assert_eq!(
            std::fs::read(&path).expect("read fixture again"),
            before,
            "inspection must leave the operator's file byte-identical (SC-013)"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// SC-013, in the case it was written for. A file that parses is easy to leave alone; the one
    /// worth asserting is the file Fathom could *not* read, because that is where a program is
    /// tempted to "fix" it. The operator's bytes are theirs, and a hand-edit Fathom did not
    /// understand may be the only copy of what they meant (Principle VI).
    #[test]
    fn a_corrupt_definition_file_is_left_byte_identical() {
        let dir =
            std::env::temp_dir().join(format!("fathom-compose-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("compose.yaml");
        // A tab where YAML requires a space, plus a comment the operator wrote: both have to
        // survive, since a rewrite that preserved the services would still have eaten the note.
        let original = "# half-finished, do not delete\nservices:\n\tapp: busybox\n";
        std::fs::write(&path, original).expect("write fixture");

        let state = inspect(path.to_str().expect("utf-8 temp path"));

        assert!(
            matches!(state, DefinitionFileState::Unreadable(_)),
            "a file that does not parse is unreadable: {state:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            original,
            "the operator's file was rewritten"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_is_unreadable_not_present() {
        let dir = std::env::temp_dir().join(format!("fathom-compose-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert!(matches!(
            inspect(dir.to_str().expect("utf-8 temp path")),
            DefinitionFileState::Unreadable(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_path_nothing_is_at_is_missing_on_the_real_filesystem() {
        assert_eq!(
            inspect("/nonexistent/fathom/compose.yaml"),
            DefinitionFileState::Missing
        );
    }
}
