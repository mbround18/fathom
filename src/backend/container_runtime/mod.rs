//! Local container-runtime management (spec 048).
//!
//! Everything here reaches the runtime through its command line, using the mockable [`Runner`]
//! that `backend::kind` already uses for its prerequisite probe. That is a requirement, not a
//! convenience: FR-018 says a mutation preview must show the operation "in a form the operator
//! could reproduce themselves", and an argv is reproducible where a socket call is not. The
//! happy side effect is that nearly every test in this module runs with no daemon at all.

pub mod adoption;
pub mod blast_radius;
pub mod compose;
pub mod grouping;
pub mod inventory;
pub mod mutate;
pub mod preview;
pub mod probe;
pub mod stream;

use super::kind::{CmdOutput, RunError, Runner};

/// Which binary to invoke, resolved once from the presence probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cli {
    bin: &'static str,
}

impl Cli {
    pub fn docker() -> Self {
        Self { bin: "docker" }
    }

    pub fn podman() -> Self {
        Self { bin: "podman" }
    }

    pub fn from_runtime(kind: crate::protocol::v1::KindRuntimeKind) -> Self {
        match kind {
            crate::protocol::v1::KindRuntimeKind::Docker => Self::docker(),
            crate::protocol::v1::KindRuntimeKind::Podman => Self::podman(),
        }
    }

    pub fn bin(self) -> &'static str {
        self.bin
    }

    /// The argv for a subcommand, program first — the same shape a preview displays.
    pub fn argv(self, args: &[&str]) -> Vec<String> {
        let mut v = Vec::with_capacity(args.len() + 1);
        v.push(self.bin.to_owned());
        v.extend(args.iter().map(|a| (*a).to_owned()));
        v
    }

    pub fn run(self, runner: &dyn Runner, args: &[&str]) -> Result<CmdOutput, RunError> {
        runner.run(self.bin, args)
    }
}

/// Parses a runtime-formatted size ("130MB", "1.21GB", "522kB", "0B") into bytes.
///
/// Returns `None` for anything it does not recognise, including the empty string. `None` means
/// "unavailable" and is rendered as such: it never becomes `0`, because a prune whose objects all
/// report zero bytes looks free when it is not.
pub fn parse_size(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if s.is_empty() || s == "N/A" {
        return None;
    }
    // Docker may append a shared-layer note: "522MB (virtual 1.1GB)".
    let s = s.split_whitespace().next()?;
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let value: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1_000.0,
        "mb" => 1_000_000.0,
        "gb" => 1_000_000_000.0,
        "tb" => 1_000_000_000_000.0,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = value * mult;
    if bytes.is_finite() && bytes >= 0.0 {
        Some(bytes as u64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_puts_the_program_first() {
        assert_eq!(
            Cli::docker().argv(&["stop", "abc"]),
            vec!["docker".to_owned(), "stop".into(), "abc".into()]
        );
    }

    #[test]
    fn sizes_parse_in_both_decimal_and_binary_units() {
        assert_eq!(parse_size("130MB"), Some(130_000_000));
        assert_eq!(parse_size("1.21GB"), Some(1_210_000_000));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("24.56MiB"), Some(25_753_026));
        assert_eq!(parse_size("522MB (virtual 1.1GB)"), Some(522_000_000));
    }

    /// An unreported size stays unavailable. Turning it into `0` would make a prune of
    /// unmeasurable objects read as reclaiming nothing, which is the opposite of the truth.
    #[test]
    fn unparseable_size_is_none_not_zero() {
        for raw in ["", "N/A", "lots", "12", "12qq"] {
            assert_eq!(parse_size(raw), None, "{raw}");
        }
    }
}
