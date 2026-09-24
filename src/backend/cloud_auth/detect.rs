//! Exec plugin classification and binary resolution (FR-001, FR-002). Nothing here ever
//! spawns a process: it only inspects the kubeconfig and the filesystem.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kube::config::{AuthInfo, ExecConfig, Kubeconfig};

use crate::protocol::{CloudAuthInfo, CloudProvider, ExecPolicy};

/// Lower-cased command basename with a Windows launcher extension stripped.
pub fn basename(command: &str) -> String {
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let lower = name.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    lower
}

/// Classify an exec plugin by command basename (research.md R3).
pub fn classify(command: &str, args: &[String]) -> CloudProvider {
    match basename(command).as_str() {
        "aws" => {
            let has = |a: &str| args.iter().any(|x| x == a);
            if has("eks") && has("get-token") {
                CloudProvider::Eks
            } else {
                CloudProvider::GenericExec
            }
        }
        "aws-iam-authenticator" | "heptio-authenticator-aws" => CloudProvider::Eks,
        "gke-gcloud-auth-plugin" => CloudProvider::Gke,
        "kubelogin" => CloudProvider::Aks,
        _ => CloudProvider::GenericExec,
    }
}

/// Classify a kubeconfig exec entry.
pub fn classify_exec(exec: &ExecConfig) -> CloudProvider {
    match exec.command.as_deref() {
        Some(cmd) if !cmd.trim().is_empty() => {
            classify(cmd, exec.args.as_deref().unwrap_or_default())
        }
        _ => CloudProvider::GenericExec,
    }
}

/// Official install guidance for a provider's plugin. Fathom never runs this.
pub fn install_hint(provider: CloudProvider, command: &str) -> String {
    match provider {
        CloudProvider::Eks => {
            "Install the AWS CLI v2 (https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html) or aws-iam-authenticator".to_owned()
        }
        CloudProvider::Gke => "gcloud components install gke-gcloud-auth-plugin".to_owned(),
        CloudProvider::Aks => "az aks install-cli   (installs kubectl and kubelogin)".to_owned(),
        CloudProvider::GenericExec | CloudProvider::None => {
            format!("Install `{command}` and make sure it is on your PATH")
        }
    }
}

/// Provider-specific re-login guidance. Fathom never runs this.
pub fn relogin_hint(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::Eks => "Run `aws sso login` (or refresh your AWS credentials), then retry",
        CloudProvider::Gke => "Run `gcloud auth login`, then retry",
        CloudProvider::Aks => {
            "Run `az login` or `kubelogin convert-kubeconfig`/`kubelogin`, then retry"
        }
        CloudProvider::GenericExec | CloudProvider::None => {
            "Log in with the provider's CLI, then retry"
        }
    }
}

/// Directories searched for the plugin: `PATH` first, then well-known install locations that a
/// GUI-launched app often misses (macOS Finder / Windows Start launch; edge case in spec.md).
pub fn search_dirs(path_env: Option<OsString>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut extra: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                let pf = PathBuf::from(pf);
                extra.push(pf.join("Amazon").join("AWSCLIV2"));
                extra.push(pf.join("Google/Cloud SDK/google-cloud-sdk/bin"));
                extra.push(pf.join("Microsoft SDKs/Azure/CLI2/wbin"));
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            extra.push(PathBuf::from(local).join("Google/Cloud SDK/google-cloud-sdk/bin"));
        }
    } else {
        extra.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/snap/bin",
                "/usr/lib/google-cloud-sdk/bin",
            ]
            .map(PathBuf::from),
        );
        if let Some(home) = dirs::home_dir() {
            extra.push(home.join(".local/bin"));
            extra.push(home.join("bin"));
            extra.push(home.join("google-cloud-sdk/bin"));
        }
    }
    for dir in extra {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Search directories from the current process environment.
pub fn default_search_dirs() -> Vec<PathBuf> {
    search_dirs(std::env::var_os("PATH"))
}

/// Result of resolving an exec command without spawning it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Found(PathBuf),
    /// A file exists but is not executable.
    NotExecutable(PathBuf),
    NotFound,
}

fn is_executable(path: &Path) -> Option<bool> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        Some(true)
    }
}

fn candidates(dir: Option<&Path>, command: &str) -> Vec<PathBuf> {
    let base = match dir {
        Some(d) => d.join(command),
        None => PathBuf::from(command),
    };
    let mut out = vec![base.clone()];
    if cfg!(windows) && Path::new(command).extension().is_none() {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let mut s = base.clone().into_os_string();
            s.push(ext);
            out.push(PathBuf::from(s));
        }
    }
    out
}

/// Resolve `command` (a bare name or a path) against `search`, without spawning it.
pub fn resolve_command(command: &str, search: &[PathBuf]) -> Resolution {
    let has_sep = command.contains('/') || (cfg!(windows) && command.contains('\\'));
    let mut not_exec = None;
    let lists: Vec<Vec<PathBuf>> = if has_sep {
        vec![candidates(None, command)]
    } else {
        search
            .iter()
            .map(|d| candidates(Some(d), command))
            .collect()
    };
    for cand in lists.into_iter().flatten() {
        match is_executable(&cand) {
            Some(true) => return Resolution::Found(cand),
            Some(false) => not_exec = not_exec.or(Some(cand)),
            None => {}
        }
    }
    not_exec.map_or(Resolution::NotFound, Resolution::NotExecutable)
}

/// The exec entry for `context`'s user, if any.
pub fn exec_for_context(kubeconfig: &Kubeconfig, context: &str) -> Option<ExecConfig> {
    let user = kubeconfig
        .contexts
        .iter()
        .find(|c| c.name == context)?
        .context
        .as_ref()?
        .user
        .clone()?;
    let info: &AuthInfo = kubeconfig
        .auth_infos
        .iter()
        .find(|a| a.name == user)?
        .auth_info
        .as_ref()?;
    info.exec.clone()
}

/// Whether `provider` may run under `policy`.
pub fn policy_allows(provider: CloudProvider, policy: ExecPolicy) -> bool {
    match provider {
        CloudProvider::GenericExec => policy == ExecPolicy::AllowAll,
        _ => true,
    }
}

/// Detection for one context (FR-001/FR-002).
pub fn detect_context(
    kubeconfig: &Kubeconfig,
    context: &str,
    policy: ExecPolicy,
    search: &[PathBuf],
) -> CloudAuthInfo {
    let Some(exec) = exec_for_context(kubeconfig, context) else {
        return CloudAuthInfo {
            context: context.to_owned(),
            provider: CloudProvider::None,
            command: None,
            installed: None,
            resolved_path: None,
            exec_policy_allowed: true,
            exec_policy: policy,
            install_hint: None,
        };
    };
    let provider = classify_exec(&exec);
    let command = exec.command.clone().unwrap_or_default();
    let resolution = resolve_command(&command, search);
    let (installed, resolved) = match &resolution {
        Resolution::Found(p) => (true, Some(p.display().to_string())),
        _ => (false, None),
    };
    CloudAuthInfo {
        context: context.to_owned(),
        provider,
        install_hint: (!installed).then(|| install_hint(provider, &command)),
        command: Some(command),
        installed: Some(installed),
        resolved_path: resolved,
        exec_policy_allowed: policy_allows(provider, policy),
        exec_policy: policy,
    }
}

/// Detection for every context in `kubeconfig`, sorted by context name.
pub fn detect_all(
    kubeconfig: &Kubeconfig,
    policies: &HashMap<String, ExecPolicy>,
    search: &[PathBuf],
) -> Vec<CloudAuthInfo> {
    let mut names: Vec<&str> = kubeconfig
        .contexts
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    names.sort_unstable();
    names
        .into_iter()
        .map(|n| {
            let policy = policies.get(n).copied().unwrap_or_default();
            detect_context(kubeconfig, n, policy, search)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn classifies_by_basename() {
        assert_eq!(
            classify("aws", &s(&["eks", "get-token", "--cluster-name", "x"])),
            CloudProvider::Eks
        );
        assert_eq!(
            classify("/usr/local/bin/aws-iam-authenticator", &[]),
            CloudProvider::Eks
        );
        assert_eq!(classify("gke-gcloud-auth-plugin", &[]), CloudProvider::Gke);
        assert_eq!(
            classify("kubelogin", &s(&["get-token"])),
            CloudProvider::Aks
        );
        assert_eq!(
            classify("aws", &s(&["s3", "ls"])),
            CloudProvider::GenericExec
        );
        assert_eq!(classify("my-plugin", &[]), CloudProvider::GenericExec);
    }

    #[test]
    fn strips_windows_extensions_and_dirs() {
        assert_eq!(
            classify("C:\\tools\\kubelogin.EXE", &[]),
            CloudProvider::Aks
        );
        assert_eq!(
            classify("gke-gcloud-auth-plugin.cmd", &[]),
            CloudProvider::Gke
        );
        assert_eq!(
            classify("aws.exe", &s(&["eks", "get-token"])),
            CloudProvider::Eks
        );
    }

    fn kubeconfig(yaml: &str) -> Kubeconfig {
        Kubeconfig::from_yaml(yaml).expect("kubeconfig")
    }

    const FIXTURE: &str = r"
apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: {server: 'https://127.0.0.1:1'}
contexts:
- {name: eks, context: {cluster: c, user: u-eks}}
- {name: gke, context: {cluster: c, user: u-gke}}
- {name: aks, context: {cluster: c, user: u-aks}}
- {name: gen, context: {cluster: c, user: u-gen}}
- {name: static, context: {cluster: c, user: u-static}}
users:
- name: u-eks
  user: {exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: aws, args: [eks, get-token, --cluster-name, x]}}
- name: u-gke
  user: {exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: gke-gcloud-auth-plugin}}
- name: u-aks
  user: {exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: kubelogin, args: [get-token]}}
- name: u-gen
  user: {exec: {apiVersion: client.authentication.k8s.io/v1beta1, command: weird-plugin}}
- name: u-static
  user: {token: abc}
";

    #[test]
    fn detects_every_class_without_spawning() {
        let kc = kubeconfig(FIXTURE);
        let infos = detect_all(&kc, &HashMap::new(), &[]);
        let by = |n: &str| infos.iter().find(|i| i.context == n).unwrap().clone();
        assert_eq!(by("eks").provider, CloudProvider::Eks);
        assert_eq!(by("gke").provider, CloudProvider::Gke);
        assert_eq!(by("aks").provider, CloudProvider::Aks);
        assert_eq!(by("gen").provider, CloudProvider::GenericExec);
        assert_eq!(by("static").provider, CloudProvider::None);
        assert_eq!(by("static").installed, None);
        // Nothing on an empty search path: all exec contexts are missing, with a hint.
        for n in ["eks", "gke", "aks", "gen"] {
            let i = by(n);
            assert_eq!(i.installed, Some(false), "{n}");
            assert!(i.install_hint.is_some());
        }
        assert!(
            by("gke")
                .install_hint
                .unwrap()
                .contains("gke-gcloud-auth-plugin")
        );
        // Generic exec is blocked by the default policy, allowed with allowAll.
        assert!(!by("gen").exec_policy_allowed);
        let mut pol = HashMap::new();
        pol.insert("gen".to_owned(), ExecPolicy::AllowAll);
        let infos = detect_all(&kc, &pol, &[]);
        assert!(
            infos
                .iter()
                .find(|i| i.context == "gen")
                .unwrap()
                .exec_policy_allowed
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolves_installed_and_reports_non_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("kubelogin");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();
        let search = vec![dir.path().to_path_buf()];
        assert_eq!(
            resolve_command("kubelogin", &search),
            Resolution::NotExecutable(bin.clone())
        );
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            resolve_command("kubelogin", &search),
            Resolution::Found(bin.clone())
        );
        assert_eq!(resolve_command("nope", &search), Resolution::NotFound);
        let kc = kubeconfig(FIXTURE);
        let info = detect_context(&kc, "aks", ExecPolicy::default(), &search);
        assert_eq!(info.installed, Some(true));
        assert_eq!(info.resolved_path.as_deref(), Some(bin.to_str().unwrap()));
        assert!(info.install_hint.is_none());
    }

    #[test]
    fn search_dirs_append_gui_locations_after_path() {
        let dirs = search_dirs(Some(OsString::from(if cfg!(windows) {
            "C:\\a"
        } else {
            "/a"
        })));
        assert_eq!(
            dirs[0],
            PathBuf::from(if cfg!(windows) { "C:\\a" } else { "/a" })
        );
        assert!(dirs.len() > 1);
    }
}
