//! `/etc/keel-runner/trust/policy.toml` (contracts/signed-mode.md "Trust files"), read by both
//! halves. Unknown keys, a missing required key or a value that does not fit fail the whole file:
//! a host with no readable policy runs nothing.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

/// The file name under the trust directory.
pub const POLICY_FILE: &str = "policy.toml";
/// Trust files are small; anything larger is refused.
const MAX_TRUST_FILE: u64 = 1024 * 1024;

/// The host's local policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Keel's web origin (passkey checks, signed mode).
    pub origin: String,
    /// The passkey RP id.
    pub rp_id: String,
    /// `true`: refuse unsigned work whatever Keel says.
    #[serde(default)]
    pub require_signed: bool,
    /// The ceiling for profiles Keel may assign. Empty refuses every job.
    #[serde(default)]
    pub allowed_profiles: Vec<String>,
    #[serde(default = "default_quorum")]
    pub override_quorum: u32,
    /// Upper bound on `expires_at - now` of an approval, e.g. `24h`.
    #[serde(default = "default_override_age")]
    pub max_override_age: String,
    /// This host's name in the inventory: the executor's forced `--limit`.
    pub inventory_name: String,
}

fn default_quorum() -> u32 {
    1
}

fn default_override_age() -> String {
    "24h".to_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("cannot read {0}: {1}")]
    Read(String, std::io::Error),
    #[error("{0} is larger than 1 MiB")]
    TooLarge(String),
    #[error("policy.toml: {0}")]
    Parse(String),
    #[error("policy.toml: {0}")]
    Invalid(&'static str),
}

/// Reads a small trust file as UTF-8.
pub fn read_trust_file(path: &Path) -> Result<String, PolicyError> {
    let shown = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| PolicyError::Read(shown.clone(), e))?;
    if meta.len() > MAX_TRUST_FILE {
        return Err(PolicyError::TooLarge(shown));
    }
    std::fs::read_to_string(path).map_err(|e| PolicyError::Read(shown, e))
}

/// An inventory host name usable as a literal `--limit`: no pattern characters, no separators.
pub fn is_literal_host_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `30s`, `90m`, `24h` or `2d`.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let unit = text.chars().last()?;
    let n: u64 = text[..text.len() - unit.len_utf8()].parse().ok()?;
    let secs = match unit {
        's' => n,
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(3600)?,
        'd' => n.checked_mul(86_400)?,
        _ => return None,
    };
    (secs > 0).then(|| Duration::from_secs(secs))
}

impl Policy {
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let policy: Policy = toml::from_str(text).map_err(|e| PolicyError::Parse(e.to_string()))?;
        if !is_literal_host_name(&policy.inventory_name) {
            return Err(PolicyError::Invalid(
                "inventory_name must be a literal host name (a-z, 0-9, '-', '_', '.')",
            ));
        }
        if policy.override_quorum == 0 {
            return Err(PolicyError::Invalid("override_quorum must be at least 1"));
        }
        if parse_duration(&policy.max_override_age).is_none() {
            return Err(PolicyError::Invalid(
                "max_override_age must look like 30s, 90m, 24h or 2d",
            ));
        }
        if policy
            .allowed_profiles
            .iter()
            .any(|p| p.is_empty() || p.chars().any(|c| c.is_control() || c.is_whitespace()))
        {
            return Err(PolicyError::Invalid("allowed_profiles has an invalid name"));
        }
        if policy.origin.is_empty() || policy.rp_id.is_empty() {
            return Err(PolicyError::Invalid("origin and rp_id are required"));
        }
        Ok(policy)
    }

    /// Reads `<trust_dir>/policy.toml`.
    pub fn load(trust_dir: &Path) -> Result<Self, PolicyError> {
        Self::parse(&read_trust_file(&trust_dir.join(POLICY_FILE))?)
    }

    pub fn max_override_age(&self) -> Duration {
        parse_duration(&self.max_override_age).unwrap_or(Duration::from_secs(86_400))
    }

    /// Whether a job with these profiles may run here: at least one of them is allowed. An empty
    /// `allowed_profiles` allows nothing.
    pub fn allows_any(&self, profiles: &[String]) -> bool {
        profiles.iter().any(|p| self.allowed_profiles.contains(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str =
        "origin = \"https://keel.example\"\nrp_id = \"keel.example\"\ninventory_name = \"web-1\"\n";

    #[test]
    fn defaults_and_required_keys() {
        let p = Policy::parse(MIN).unwrap();
        assert!(!p.require_signed);
        assert!(p.allowed_profiles.is_empty());
        assert_eq!(p.override_quorum, 1);
        assert_eq!(p.max_override_age(), Duration::from_secs(86_400));
        assert!(!p.allows_any(&["base".to_owned()]));
        assert!(Policy::parse("origin = \"x\"\nrp_id = \"y\"\n").is_err());
    }

    #[test]
    fn unknown_keys_and_bad_values_fail_closed() {
        assert!(Policy::parse(&format!("{MIN}server = \"keel\"\n")).is_err());
        assert!(Policy::parse(&format!("{MIN}override_quorum = 0\n")).is_err());
        assert!(Policy::parse(&format!("{MIN}max_override_age = \"forever\"\n")).is_err());
        let star = MIN.replace("web-1", "web-*");
        assert!(Policy::parse(&star).is_err());
        let comma = MIN.replace("web-1", "web-1,db-1");
        assert!(Policy::parse(&comma).is_err());
    }

    #[test]
    fn any_allowed_profile_admits_a_job() {
        let p = Policy::parse(&format!("{MIN}allowed_profiles = [\"base\", \"web\"]\n")).unwrap();
        assert!(p.allows_any(&["db".to_owned(), "web".to_owned()]));
        assert!(!p.allows_any(&["db".to_owned()]));
        assert!(!p.allows_any(&[]));
    }
}
