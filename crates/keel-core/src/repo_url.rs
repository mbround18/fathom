//! Repository URL normalization, so a receiver payload's clone URLs match a binding's URL
//! whichever transport either uses (spec 055 T050, contracts/receivers.md step 5).

/// `host/owner/repo`, lowercased, without scheme, user, port, trailing `/` or `.git`.
///
/// `https://Git.Example.com/Ops/Infra.git`, `ssh://git@git.example.com:2222/ops/infra` and
/// `git@git.example.com:ops/infra.git` all normalize to `git.example.com/ops/infra`. Returns
/// `None` for anything that is not a recognisable remote URL.
#[must_use]
pub fn normalize(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (authority, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git"
        ) {
            return None;
        }
        match rest.split_once('/') {
            Some((a, p)) => (a, p),
            None => return None,
        }
    } else {
        // scp-like: [user@]host:path
        let (a, p) = url.split_once(':')?;
        if a.contains('/') || p.starts_with("//") {
            return None;
        }
        (a, p)
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?.0,
        None => host.split_once(':').map_or(host, |(h, _)| h),
    };
    if host.is_empty() {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if path.is_empty() || path.split('/').any(|c| c.is_empty() || c == "..") {
        return None;
    }
    Some(format!("{host}/{path}").to_ascii_lowercase())
}

/// Whether two URLs name the same repository.
#[must_use]
pub fn same_repo(a: &str, b: &str) -> bool {
    matches!((normalize(a), normalize(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_agree() {
        let want = Some("git.example.com/ops/infra".to_owned());
        for u in [
            "https://Git.Example.com/Ops/Infra.git",
            "https://token@git.example.com/ops/infra/",
            "http://git.example.com:3000/ops/infra",
            "ssh://git@git.example.com:2222/ops/infra.git",
            "git@git.example.com:ops/infra.git",
            "git.example.com:ops/infra",
        ] {
            assert_eq!(normalize(u), want, "{u}");
        }
        assert!(same_repo(
            "git@github.com:a/b.git",
            "https://github.com/a/b"
        ));
        assert!(!same_repo(
            "https://github.com/a/b",
            "https://github.com/a/bc"
        ));
        assert!(!same_repo(
            "https://github.com/a/b",
            "https://gitlab.com/a/b"
        ));
    }

    #[test]
    fn junk_is_refused() {
        for u in [
            "",
            "file:///srv/repo",
            "https://host",
            "https://host/",
            "https://host/a/../b",
            "https://ho st/a",
            "/srv/repo.git",
            "ext::sh -c x",
        ] {
            assert_eq!(normalize(u), None, "{u}");
        }
        assert!(!same_repo("", ""));
    }
}
