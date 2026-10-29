//! Keep tags (research R7, FR-027, FR-028): the local tags Depths sets on the images it keeps.
//!
//! `depths-keep/<stack>:<service>-previous` names the image a service ran before its last
//! successful update (the way back), and `depths-keep/<stack>:<service>-held` the image a hold
//! or a rollback set aside for review. Nobody pushes to `depths-keep`, so the tags stay local;
//! they are visible in `docker images` and protect the images from `docker image prune`.
//!
//! Stack and service names are lowercased and any character outside `[a-z0-9._-]` becomes `-`,
//! with an 8-hex hash suffix when that changed the name, so two names that sanitise alike still
//! get different tags.

use sha2::{Digest, Sha256};

/// The repository every keep tag lives in.
pub const REPOSITORY: &str = "depths-keep";

/// Which image a keep tag holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeepKind {
    /// The image a service ran before its last successful update: its rollback target.
    Previous,
    /// The image a hold or a rollback set aside.
    Held,
}

impl KeepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Previous => "previous",
            Self::Held => "held",
        }
    }
}

/// One name made safe for a reference: lowercased, anything outside `[a-z0-9._-]` as `-`, and
/// an 8-hex suffix of the original's SHA-256 when that changed it.
pub fn sanitise(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean == name {
        return clean;
    }
    let digest = Sha256::digest(name.as_bytes());
    let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{clean}-{hex}")
}

/// The keep repository of a stack: `depths-keep/<stack>`.
pub fn repository(stack: &str) -> String {
    format!("{REPOSITORY}/{}", sanitise(stack))
}

/// The keep tag of one service: `depths-keep/<stack>:<service>-<kind>`.
pub fn tag(stack: &str, service: &str, kind: KeepKind) -> String {
    format!("{}:{}-{}", repository(stack), sanitise(service), kind.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_and_held_tags_name_the_stack_and_service() {
        assert_eq!(
            tag("media", "app", KeepKind::Previous),
            "depths-keep/media:app-previous"
        );
        assert_eq!(
            tag("media", "app", KeepKind::Held),
            "depths-keep/media:app-held"
        );
        assert_eq!(repository("media"), "depths-keep/media");
    }

    #[test]
    fn valid_names_are_kept_as_they_are() {
        for n in ["web", "my-stack", "a.b_c-1", "0day"] {
            assert_eq!(sanitise(n), n);
        }
    }

    #[test]
    fn names_are_lowercased_and_odd_characters_become_dashes_with_a_hash() {
        let s = sanitise("Media");
        assert!(s.starts_with("media-"), "{s}");
        assert_eq!(s.len(), "media-".len() + 8);
        assert!(s["media-".len()..].bytes().all(|b| b.is_ascii_hexdigit()));
        let t = sanitise("my stack/1");
        assert!(t.starts_with("my-stack-1-"), "{t}");
        assert_eq!(
            tag("Media", "App", KeepKind::Previous),
            format!("depths-keep/{}:{}-previous", sanitise("Media"), sanitise("App"))
        );
    }

    #[test]
    fn names_that_sanitise_alike_still_differ() {
        assert_ne!(sanitise("a b"), sanitise("a+b"));
        assert_ne!(sanitise("Web"), sanitise("web"));
        assert_eq!(sanitise("a b"), sanitise("a b"), "deterministic");
    }

    #[test]
    fn every_tag_is_a_valid_reference() {
        for (stack, svc) in [("Media", "App"), ("x y", "z@q"), ("web", "app")] {
            for kind in [KeepKind::Previous, KeepKind::Held] {
                let t = tag(stack, svc, kind);
                let r = super::super::reference::ImageRef::parse(&t).unwrap();
                assert!(!r.is_pinned());
                assert!(
                    t.bytes()
                        .all(|b| b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || b".:/_-".contains(&b)),
                    "{t}"
                );
            }
        }
    }
}
