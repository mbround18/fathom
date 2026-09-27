//! Import the user's login-shell environment when the desktop app is started from a launcher
//! (feedback #1).
//!
//! A `.desktop` entry (`Exec=fathom-desktop`) starts Fathom with the graphical session's
//! environment, not the one the user's shell builds in `.profile`/`.bashrc`/`.zshrc`. A
//! `KUBECONFIG` exported there is then invisible, and so are the `PATH` entries that hold exec
//! credential plugins (`gke-gcloud-auth-plugin`, `aws`, `kubelogin`), so "Reload kubeconfig"
//! appears to do nothing. Before the backend starts, this module runs `$SHELL` once as a login,
//! interactive shell, reads its environment and merges in a short allowlist of variables.
//!
//! It never blocks startup for long ([`TIMEOUT`]), never panics, and leaves the environment
//! untouched on any failure. It is skipped when Fathom was started from a terminal (the
//! environment is already the shell's) and on non-Unix platforms.
//!
//! Imported variables (be conservative: only what kubeconfig loading and exec credential plugins
//! need):
//! - `KUBECONFIG`, `KUBECACHEDIR`, `USE_GKE_GCLOUD_AUTH_PLUGIN`
//! - `PATH`: the shell's entries first, then any entry of the current `PATH` it lacks
//! - any variable starting with `AWS_`, `CLOUDSDK_`, `GOOGLE_` or `AZURE_` (profiles, regions,
//!   config dirs and credential files the cloud CLIs read)
//!
//! Proxy variables, locale, `HOME`, display/session variables and everything else are left
//! alone. `FATHOM_IMPORT_SHELL_ENV=0` disables the import; `=1` forces it even from a terminal.

use std::collections::HashMap;
use std::ffi::OsString;
use std::time::Duration;

/// Upper bound on how long startup waits for the login shell.
pub const TIMEOUT: Duration = Duration::from_secs(3);

/// Opt-out (`0`) / force (`1`) switch.
pub const SWITCH_VAR: &str = "FATHOM_IMPORT_SHELL_ENV";

const EXACT: &[&str] = &["KUBECONFIG", "KUBECACHEDIR", "USE_GKE_GCLOUD_AUTH_PLUGIN"];
const PREFIXES: &[&str] = &["AWS_", "CLOUDSDK_", "GOOGLE_", "AZURE_"];

const BEGIN: &str = "__FATHOM_SHELL_ENV_BEGIN__";
const END: &str = "__FATHOM_SHELL_ENV_END__";

/// Whether `name` is on the import allowlist (`PATH` is handled separately by [`merge`]).
pub fn is_imported(name: &str) -> bool {
    name == "PATH" || EXACT.contains(&name) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Parse `env -0` output framed by the begin/end markers. Anything the shell printed before the
/// begin marker (a motd, `echo` in an rc file) is ignored. `None` when either marker is missing,
/// i.e. the shell did not run the command to completion.
pub fn parse_env0(out: &[u8]) -> Option<HashMap<String, String>> {
    let begin = find(out, BEGIN.as_bytes())? + BEGIN.len();
    let rest = &out[begin..];
    let end = find(rest, END.as_bytes())?;
    let body = &rest[..end];
    let mut vars = HashMap::new();
    for entry in body.split(|b| *b == 0) {
        let Ok(entry) = std::str::from_utf8(entry) else {
            continue;
        };
        // Names cannot contain '='; values can. Skip blanks and bash's exported functions.
        let Some((name, value)) = entry.split_once('=') else {
            continue;
        };
        if name.is_empty() || name.contains(char::is_whitespace) || name.starts_with("BASH_FUNC_") {
            continue;
        }
        vars.insert(name.to_owned(), value.to_owned());
    }
    Some(vars)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The variables to set: allowlisted shell variables whose value differs from the current one.
/// `PATH` is the shell's, followed by any current entries it lacks, so nothing that worked before
/// the import stops working. Sorted by name for deterministic logs and tests.
pub fn merge(
    current: &HashMap<String, String>,
    shell: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, value) in shell {
        if !is_imported(name) {
            continue;
        }
        let value = if name == "PATH" {
            merge_path(value, current.get("PATH").map(String::as_str))
        } else {
            value.clone()
        };
        if value.is_empty() && name == "PATH" {
            continue;
        }
        if current.get(name) != Some(&value) {
            out.push((name.clone(), value));
        }
    }
    out.sort();
    out
}

fn merge_path(shell: &str, current: Option<&str>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in shell.split(':').chain(current.unwrap_or("").split(':')) {
        if !p.is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    parts.join(":")
}

/// What the import did, logged once tracing is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Skipped(&'static str),
    Failed(String),
    /// Names of the variables that were set or changed.
    Imported(Vec<String>),
}

impl Outcome {
    pub fn log(&self) {
        match self {
            Outcome::Skipped(why) => tracing::info!("Login-shell environment not imported: {why}"),
            Outcome::Failed(why) => {
                tracing::warn!("Login-shell environment not imported, continuing without it: {why}")
            }
            Outcome::Imported(names) if names.is_empty() => {
                tracing::info!("Login-shell environment matches the launch environment")
            }
            Outcome::Imported(names) => {
                tracing::info!("Imported from the login shell: {}", names.join(", "))
            }
        }
    }
}

/// Decide whether to import, from the switch variable and whether a terminal is attached.
pub fn should_import(switch: Option<&str>, on_terminal: bool) -> Result<(), &'static str> {
    match switch.map(str::trim) {
        Some("0") | Some("false") | Some("no") => Err("disabled by FATHOM_IMPORT_SHELL_ENV"),
        Some("1") | Some("true") | Some("yes") => Ok(()),
        _ if on_terminal => Err("started from a terminal"),
        _ => Ok(()),
    }
}

/// Import the login-shell environment into this process. Call it before any other thread starts
/// (it mutates the process environment).
pub fn import() -> Outcome {
    if !cfg!(unix) {
        return Outcome::Skipped("not a Unix platform");
    }
    use std::io::IsTerminal;
    let on_terminal = std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal();
    let switch = std::env::var(SWITCH_VAR).ok();
    if let Err(why) = should_import(switch.as_deref(), on_terminal) {
        return Outcome::Skipped(why);
    }
    let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) else {
        return Outcome::Skipped("SHELL is not set");
    };
    let out = match run_shell(&shell, TIMEOUT) {
        Ok(out) => out,
        Err(e) => return Outcome::Failed(e),
    };
    let Some(vars) = parse_env0(&out) else {
        return Outcome::Failed(format!(
            "{} did not print its environment",
            shell.to_string_lossy()
        ));
    };
    let current: HashMap<String, String> = std::env::vars().collect();
    let changes = merge(&current, &vars);
    let names = changes.iter().map(|(n, _)| n.clone()).collect();
    for (name, value) in changes {
        // SAFETY: called at the very start of `run_with_args`, before the backend, Tauri or any
        // other thread that reads the environment exists. The only other thread alive is this
        // module's pipe reader, which never touches the environment.
        unsafe { std::env::set_var(name, value) };
    }
    Outcome::Imported(names)
}

/// Run `shell -l -i -c '<print env>'` and return its stdout up to the end marker. The shell's
/// stdin is closed and stderr discarded. Gives up (and kills the shell) after `timeout`.
#[cfg(unix)]
fn run_shell(shell: &OsString, timeout: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    // `printf` and `env -0` work in bash, zsh, fish and POSIX sh alike.
    let script = format!("printf '%s' '{BEGIN}'; command env -0; printf '%s' '{END}'");
    let mut cmd = Command::new(shell);
    cmd.args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env(SWITCH_VAR, "0");
    // New session without a controlling terminal: an interactive shell then cannot try to take
    // over a terminal's foreground process group (and stop itself on SIGTTIN/SIGTTOU).
    // SAFETY: setsid() is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", shell.to_string_lossy()))?;
    let mut stdout = child.stdout.take().ok_or("no stdout pipe")?;

    // Read on a thread: a background process started by an rc file (an agent, a daemon) can hold
    // the pipe open after the shell exits, so EOF is not a reliable end. The end marker is.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let result = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Err(format!(
                "{} did not finish within {timeout:?}",
                shell.to_string_lossy()
            ));
        }
        match rx.recv_timeout(left) {
            Ok(chunk) => {
                out.extend_from_slice(&chunk);
                if find(&out, END.as_bytes()).is_some() {
                    break Ok(out);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break Err(format!(
                    "{} exited without printing its environment",
                    shell.to_string_lossy()
                ));
            }
        }
    };
    // The shell is done or hung; either way it has nothing more to give.
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(not(unix))]
fn run_shell(_shell: &OsString, _timeout: Duration) -> Result<Vec<u8>, String> {
    Err("not supported on this platform".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut v = b"Welcome to Ubuntu!\nbash: no job control\n".to_vec();
        v.extend_from_slice(BEGIN.as_bytes());
        v.extend_from_slice(body);
        v.extend_from_slice(END.as_bytes());
        v.extend_from_slice(b"\nlogout noise");
        v
    }

    #[test]
    fn parses_nul_separated_env_between_markers() {
        let out = framed(b"PATH=/a:/b\0KUBECONFIG=/x/one:/x/two\0MULTI=line1\nline2\0EQ=a=b\0");
        let vars = parse_env0(&out).expect("framed");
        assert_eq!(vars["PATH"], "/a:/b");
        assert_eq!(vars["KUBECONFIG"], "/x/one:/x/two");
        assert_eq!(vars["MULTI"], "line1\nline2");
        assert_eq!(vars["EQ"], "a=b");
        assert_eq!(vars.len(), 4);
    }

    #[test]
    fn ignores_noise_before_the_marker_and_bash_functions() {
        let out = framed(b"BASH_FUNC_foo%%=() {  echo hi\n}\0KUBECONFIG=/k\0garbage\0\0");
        let vars = parse_env0(&out).expect("framed");
        assert_eq!(vars, map(&[("KUBECONFIG", "/k")]));
    }

    #[test]
    fn missing_markers_mean_no_environment() {
        assert!(parse_env0(b"PATH=/a\0").is_none());
        let mut half = BEGIN.as_bytes().to_vec();
        half.extend_from_slice(b"PATH=/a\0");
        assert!(parse_env0(&half).is_none());
    }

    #[test]
    fn merge_imports_only_the_allowlist() {
        let current = map(&[("PATH", "/usr/bin"), ("HOME", "/home/u")]);
        let shell = map(&[
            ("PATH", "/home/u/google-cloud-sdk/bin:/usr/bin"),
            ("KUBECONFIG", "/home/u/.kube/prod"),
            ("AWS_PROFILE", "prod"),
            ("CLOUDSDK_CONFIG", "/home/u/.gcloud"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/k.json"),
            ("AZURE_CONFIG_DIR", "/az"),
            ("USE_GKE_GCLOUD_AUTH_PLUGIN", "True"),
            ("HOME", "/elsewhere"),
            ("HTTPS_PROXY", "http://proxy"),
            ("PS1", "$ "),
            ("SHLVL", "2"),
        ]);
        let names: Vec<String> = merge(&current, &shell)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names,
            [
                "AWS_PROFILE",
                "AZURE_CONFIG_DIR",
                "CLOUDSDK_CONFIG",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "KUBECONFIG",
                "PATH",
                "USE_GKE_GCLOUD_AUTH_PLUGIN",
            ]
        );
    }

    #[test]
    fn merge_path_keeps_current_entries_the_shell_lacks() {
        let current = map(&[("PATH", "/usr/bin:/snap/bin:/opt/launcher")]);
        let shell = map(&[("PATH", "/home/u/bin:/usr/bin:/snap/bin:/usr/bin")]);
        assert_eq!(
            merge(&current, &shell),
            [(
                "PATH".to_owned(),
                "/home/u/bin:/usr/bin:/snap/bin:/opt/launcher".to_owned()
            )]
        );
    }

    #[test]
    fn merge_skips_unchanged_values() {
        let current = map(&[("PATH", "/usr/bin"), ("KUBECONFIG", "/k")]);
        let shell = map(&[("PATH", "/usr/bin"), ("KUBECONFIG", "/k")]);
        assert!(merge(&current, &shell).is_empty());
    }

    #[test]
    fn shell_value_wins_over_the_launch_environment() {
        let current = map(&[("KUBECONFIG", "/session/kube")]);
        let shell = map(&[("KUBECONFIG", "/rc/kube")]);
        assert_eq!(
            merge(&current, &shell),
            [("KUBECONFIG".to_owned(), "/rc/kube".to_owned())]
        );
    }

    #[test]
    fn switch_and_terminal_decide_whether_to_import() {
        assert!(should_import(None, false).is_ok());
        assert!(should_import(None, true).is_err());
        assert!(should_import(Some("1"), true).is_ok());
        assert!(should_import(Some("0"), false).is_err());
        assert!(should_import(Some(""), false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn reads_the_environment_from_a_real_shell() {
        for shell in ["/bin/sh", "/bin/bash", "/usr/bin/zsh", "/usr/bin/fish"] {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let out = run_shell(&OsString::from(shell), Duration::from_secs(10))
                .unwrap_or_else(|e| panic!("{shell}: {e}"));
            let vars = parse_env0(&out).expect("framed");
            assert!(vars.contains_key("PATH"), "{shell}");
            assert_eq!(
                vars.get(SWITCH_VAR).map(String::as_str),
                Some("0"),
                "{shell}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_shell_times_out_quickly() {
        // A "shell" that ignores its arguments and never prints the markers.
        let dir = std::env::temp_dir().join(format!("fathom-login-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("hang.sh");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let start = std::time::Instant::now();
        let err = run_shell(&fake.into_os_string(), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
