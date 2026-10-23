//! Which file in a stack directory is the compose file (R1).
//!
//! Depths picks the file Compose picks, "the first present in Compose's order" (compose-go
//! `cli/options.go`): `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`.
//! Dockge's order differs, so where two exist Dockge can edit a file Compose never runs; Depths
//! names the others as `shadowed_files` and the UI warns (FR-004).
//!
//! No I/O: the adapter reads the directory and hands over a [`DirListing`].

use serde::{Deserialize, Serialize};

/// The compose file names Compose looks for, in its order of precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ComposeFile {
    #[serde(rename = "compose.yaml")]
    ComposeYaml,
    #[serde(rename = "compose.yml")]
    ComposeYml,
    #[serde(rename = "docker-compose.yml")]
    DockerComposeYml,
    #[serde(rename = "docker-compose.yaml")]
    DockerComposeYaml,
}

impl ComposeFile {
    /// Compose's order: the first of these present is the one it runs.
    pub const ORDER: [Self; 4] = [
        Self::ComposeYaml,
        Self::ComposeYml,
        Self::DockerComposeYml,
        Self::DockerComposeYaml,
    ];

    /// The name a new stack's file gets (Dockge writes the same).
    pub const DEFAULT: Self = Self::ComposeYaml;

    pub fn file_name(self) -> &'static str {
        match self {
            Self::ComposeYaml => "compose.yaml",
            Self::ComposeYml => "compose.yml",
            Self::DockerComposeYml => "docker-compose.yml",
            Self::DockerComposeYaml => "docker-compose.yaml",
        }
    }

    /// The variant for an exact file name. Case matters, as it does to Compose on Linux.
    pub fn from_file_name(name: &str) -> Option<Self> {
        Self::ORDER.into_iter().find(|f| f.file_name() == name)
    }
}

/// The environment file Compose reads from the project directory.
pub const ENV_FILE: &str = ".env";

/// The stacks-root file Dockge (main, PR #387) passes to every command when it exists.
pub const GLOBAL_ENV_FILE: &str = "global.env";

/// The non-directory entries of one stack directory, by name. Symlinks count; whether one may be
/// followed is the adapter's decision, made when it opens the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirListing {
    pub files: Vec<String>,
}

impl DirListing {
    pub fn new<I, S>(files: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            files: files.into_iter().map(Into::into).collect(),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.files.iter().any(|f| f == name)
    }
}

/// What a directory listing says about a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// The file Compose would run.
    pub compose_file: ComposeFile,
    /// The other compose file names present, in Compose's order. Non-empty means a warning.
    pub shadowed_files: Vec<ComposeFile>,
    pub has_env: bool,
}

/// The compose file of a directory, or `None` when it holds none (and so is not a stack).
pub fn discover(listing: &DirListing) -> Option<Discovered> {
    let mut present = ComposeFile::ORDER
        .into_iter()
        .filter(|f| listing.has(f.file_name()));
    let compose_file = present.next()?;
    Some(Discovered {
        compose_file,
        shadowed_files: present.collect(),
        has_env: listing.has(ENV_FILE),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(files: &[&str]) -> Option<Discovered> {
        discover(&DirListing::new(files.iter().copied()))
    }

    #[test]
    fn each_name_alone_is_found() {
        for f in ComposeFile::ORDER {
            let d = found(&[f.file_name()]).expect("a compose file");
            assert_eq!(d.compose_file, f);
            assert!(d.shadowed_files.is_empty());
        }
    }

    #[test]
    fn the_first_present_in_composes_order_wins() {
        let d = found(&["docker-compose.yaml", "docker-compose.yml", "compose.yml"]).unwrap();
        assert_eq!(d.compose_file, ComposeFile::ComposeYml);
        assert_eq!(
            d.shadowed_files,
            vec![
                ComposeFile::DockerComposeYml,
                ComposeFile::DockerComposeYaml
            ]
        );

        // Dockge would pick docker-compose.yaml here; Compose picks docker-compose.yml.
        let d = found(&["docker-compose.yaml", "docker-compose.yml"]).unwrap();
        assert_eq!(d.compose_file, ComposeFile::DockerComposeYml);
        assert_eq!(d.shadowed_files, vec![ComposeFile::DockerComposeYaml]);

        let all = found(&[
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
            "docker-compose.yml",
        ])
        .unwrap();
        assert_eq!(all.compose_file, ComposeFile::ComposeYaml);
        assert_eq!(all.shadowed_files, ComposeFile::ORDER[1..].to_vec());
    }

    #[test]
    fn a_directory_without_a_compose_file_is_not_a_stack() {
        assert_eq!(found(&[]), None);
        assert_eq!(
            found(&[".env", "README.md", "Compose.yaml", "compose.yaml.bak"]),
            None
        );
    }

    #[test]
    fn env_presence_is_reported() {
        assert!(found(&["compose.yaml", ".env"]).unwrap().has_env);
        assert!(!found(&["compose.yaml", "global.env"]).unwrap().has_env);
    }

    #[test]
    fn file_names_round_trip() {
        for f in ComposeFile::ORDER {
            assert_eq!(ComposeFile::from_file_name(f.file_name()), Some(f));
        }
        assert_eq!(ComposeFile::from_file_name("COMPOSE.YAML"), None);
    }
}
