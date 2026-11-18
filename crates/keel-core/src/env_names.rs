//! Environment variable names a credential may set for `ansible-playbook` (spec 068 T011).
//!
//! One rule, shared by the manifest (`jobs.<j>.env`, checked at sync and on the runner) and the
//! executor (the `env` credential kind and `env_var` secret items), so the server never plans a
//! name the host would refuse.

/// The longest name, in bytes.
pub const MAX_ENV_NAME: usize = 64;

/// The most `env:` entries one job may declare.
pub const MAX_JOB_ENV: usize = 16;

/// Why a name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnvNameError {
    #[error("an environment name is A-Z, 0-9 and '_', not starting with a digit (at most 64)")]
    Invalid,
    #[error(
        "an environment name may not steer Ansible, Python, the shell or the dynamic linker \
         (ANSIBLE_*, LD_*, PYTHON*, KEEL_*, PATH, HOME, SHELL, IFS, ENV, BASH_ENV)"
    )]
    Reserved,
}

impl EnvNameError {
    /// The manifest validation code (contracts/manifest.md).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "env_name_invalid",
            Self::Reserved => "env_name_reserved",
        }
    }
}

/// `[A-Z_][A-Z0-9_]*`, any length. The executor's `env` credential kind has always accepted
/// this shape; the manifest adds the length bound through [`check`].
#[must_use]
pub fn is_shell_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_uppercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Names that would steer Ansible, Python, the shell or the dynamic linker, or Keel itself.
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    name.starts_with("ANSIBLE_")
        || name.starts_with("LD_")
        || name.starts_with("PYTHON")
        || name.starts_with("KEEL_")
        || matches!(name, "PATH" | "HOME" | "SHELL" | "IFS" | "ENV" | "BASH_ENV")
}

/// `^[A-Z_][A-Z0-9_]{0,63}$` and not reserved.
///
/// # Errors
///
/// The name has the wrong shape, or is reserved.
pub fn check(name: &str) -> Result<(), EnvNameError> {
    if name.len() > MAX_ENV_NAME || !is_shell_name(name) {
        return Err(EnvNameError::Invalid);
    }
    if is_reserved(name) {
        return Err(EnvNameError::Reserved);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_names() {
        for ok in ["TS_AUTHKEY", "K3S_URL", "_X", "A", &"A".repeat(64)] {
            assert_eq!(check(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn refuses_bad_shapes() {
        for bad in [
            "",
            "ts_authkey",
            "1X",
            "A-B",
            "A B",
            "A=B",
            "É",
            &"A".repeat(65),
        ] {
            assert_eq!(check(bad), Err(EnvNameError::Invalid), "{bad}");
        }
    }

    #[test]
    fn refuses_reserved_names() {
        for bad in [
            "ANSIBLE_CONFIG",
            "LD_PRELOAD",
            "PYTHONPATH",
            "PYTHON",
            "KEEL_TOKEN",
            "PATH",
            "HOME",
            "SHELL",
            "IFS",
            "ENV",
            "BASH_ENV",
        ] {
            assert_eq!(check(bad), Err(EnvNameError::Reserved), "{bad}");
            assert!(is_reserved(bad));
        }
        // Only the exact names, not names that merely contain them.
        for ok in ["MY_PATH", "HOMEDIR", "XLD_FOO", "ENVIRONMENT"] {
            assert_eq!(check(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn codes() {
        assert_eq!(EnvNameError::Invalid.code(), "env_name_invalid");
        assert_eq!(EnvNameError::Reserved.code(), "env_name_reserved");
    }
}
