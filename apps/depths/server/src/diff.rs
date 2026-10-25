//! The `file_diff` a save, create or delete preview shows: a unified diff with one hunk.
//!
//! The changed span is everything between the longest common prefix and the longest common
//! suffix of the two texts' lines. That is linear in the file size (the editor's files are
//! capped, but not small), and for the usual edit, a few lines in one place, it is the same
//! hunk a full diff would give.

/// Lines of context either side of the change.
const CONTEXT: usize = 3;

/// A unified diff of `old` to `new` for `file`. `None` for no file (a create or a removal).
/// Empty when nothing changed.
pub fn unified(file: &str, old: Option<&str>, new: Option<&str>) -> String {
    if old == new {
        return String::new();
    }
    let a: Vec<&str> = old.map(|t| t.lines().collect()).unwrap_or_default();
    let b: Vec<&str> = new.map(|t| t.lines().collect()).unwrap_or_default();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    // Line endings or a trailing newline only: the lines match but the bytes do not.
    let (start, a_end, b_end) = if prefix == a.len() && prefix == b.len() {
        (0, a.len(), b.len())
    } else {
        (prefix, a.len() - suffix, b.len() - suffix)
    };
    let from = start.saturating_sub(CONTEXT);
    let a_to = (a_end + CONTEXT).min(a.len());
    let b_to = (b_end + CONTEXT).min(b.len());

    let mut out = String::new();
    out.push_str(&match old {
        Some(_) => format!("--- a/{file}\n"),
        None => "--- /dev/null\n".to_owned(),
    });
    out.push_str(&match new {
        Some(_) => format!("+++ b/{file}\n"),
        None => "+++ /dev/null\n".to_owned(),
    });
    out.push_str(&format!(
        "@@ -{} +{} @@\n",
        range(from, a_to - from),
        range(from, b_to - from)
    ));
    for l in &a[from..start] {
        out.push_str(&format!(" {l}\n"));
    }
    for l in &a[start..a_end] {
        out.push_str(&format!("-{l}\n"));
    }
    for l in &b[start..b_end] {
        out.push_str(&format!("+{l}\n"));
    }
    for l in &a[a_end..a_to] {
        out.push_str(&format!(" {l}\n"));
    }
    out
}

/// Lines removed from `old` and added in `new`, over the same changed span [`unified`] shows.
fn changed_lines(old: Option<&str>, new: Option<&str>) -> (usize, usize) {
    let a: Vec<&str> = old.map(|t| t.lines().collect()).unwrap_or_default();
    let b: Vec<&str> = new.map(|t| t.lines().collect()).unwrap_or_default();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (a.len() - prefix - suffix, b.len() - prefix - suffix)
}

/// `KEY` to value, for each `KEY=value` line of a `.env` (last one wins, as Compose reads it).
/// Only `KEY=` lines count: the lines of a quoted multi-line value (a PEM key, say) are content,
/// and a bare word is as likely one of them as a variable, so neither is ever listed as a key.
fn env_pairs(text: &str) -> std::collections::BTreeMap<&str, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut open: Option<(&str, char, String)> = None;
    for line in text.lines() {
        if let Some((k, q, mut v)) = open.take() {
            v.push('\n');
            v.push_str(line);
            if line.contains(q) {
                out.insert(k, v);
            } else {
                open = Some((k, q, v));
            }
            continue;
        }
        let l = line.trim_start();
        let l = l.strip_prefix("export ").unwrap_or(l);
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let name_like = k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !name_like {
            continue;
        }
        let vt = v.trim_start();
        match vt.chars().next() {
            Some(q @ ('"' | '\'')) if !vt[1..].contains(q) => {
                open = Some((k, q, v.to_owned()));
            }
            _ => {
                out.insert(k, v.to_owned());
            }
        }
    }
    if let Some((k, _, v)) = open {
        out.insert(k, v);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    depths_core::hash::FileDigest::of(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The audit's account of a save or create (FR-035, data-model `AuditEntry`): lines added and
/// removed in the compose file with a digest of the new one, `.env` reduced to the keys added,
/// changed and removed, and the stack hash it was based on. Never a value or a line of content.
/// `new_env` `None` leaves `.env` as it is.
pub fn summary(
    file: &str,
    old: Option<&str>,
    new: &str,
    old_env: Option<&str>,
    new_env: Option<&str>,
    base: Option<&str>,
) -> String {
    let (removed, added) = changed_lines(old, Some(new));
    let mut out = format!(
        "{file}: +{added} -{removed} lines, sha256 {}",
        sha256_hex(new.as_bytes())
    );
    if let Some(new_env) = new_env {
        let before = env_pairs(old_env.unwrap_or(""));
        let after = env_pairs(new_env);
        let keys = |pick: &dyn Fn(&str) -> bool| -> Vec<&str> {
            before
                .keys()
                .chain(after.keys())
                .copied()
                .filter(|k| pick(k))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        let added = keys(&|k| !before.contains_key(k) && after.contains_key(k));
        let changed =
            keys(&|k| matches!((before.get(k), after.get(k)), (Some(a), Some(b)) if a != b));
        let removed = keys(&|k| before.contains_key(k) && !after.contains_key(k));
        let list = |v: &[&str]| {
            if v.is_empty() {
                "-".to_owned()
            } else {
                v.join(",")
            }
        };
        out.push_str(&format!(
            "; .env: added {}, changed {}, removed {}",
            list(&added),
            list(&changed),
            list(&removed)
        ));
    }
    out.push_str(&format!("; base {}", base.unwrap_or("none")));
    out
}

/// A hunk range: 1-based start and length, `start-1,0` for an empty side.
fn range(from: usize, len: usize) -> String {
    if len == 0 {
        format!("{from},0")
    } else {
        format!("{},{len}", from + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_changed_line_with_context() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\n";
        assert_eq!(
            unified("compose.yaml", Some(old), Some(new)),
            "--- a/compose.yaml\n+++ b/compose.yaml\n@@ -2,7 +2,7 @@\n b\n c\n d\n-e\n+E\n f\n g\n h\n"
        );
    }

    #[test]
    fn create_and_remove() {
        assert_eq!(
            unified("compose.yaml", None, Some("x\ny\n")),
            "--- /dev/null\n+++ b/compose.yaml\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
        assert_eq!(
            unified("compose.yaml", Some("x\n"), None),
            "--- a/compose.yaml\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\n"
        );
    }

    #[test]
    fn unchanged_is_empty_and_an_insertion_keeps_both_sides() {
        assert_eq!(unified("f", Some("a\n"), Some("a\n")), "");
        assert_eq!(
            unified("f", Some("a\nc\n"), Some("a\nb\nc\n")),
            "--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n a\n+b\n c\n"
        );
    }

    #[test]
    fn the_summary_names_keys_never_values_or_continuation_lines() {
        let old_env =
            "A=1\nPEM=\"-----BEGIN KEY-----\nMIIBsecretline\nQUJD=\n-----END KEY-----\"\nBARE\n";
        let s = summary(
            "compose.yaml",
            Some("a\nb\n"),
            "a\nc\nd\n",
            Some(old_env),
            Some("A=2\nB=3\n"),
            Some("abc"),
        );
        assert!(s.starts_with("compose.yaml: +2 -1 lines, sha256 "), "{s}");
        assert!(
            s.ends_with("; .env: added B, changed A, removed PEM; base abc"),
            "{s}"
        );
        for content in ["MIIB", "BEGIN", "QUJD", "BARE"] {
            assert!(!s.contains(content), "{content} in {s}");
        }
        let s = summary("compose.yaml", None, "x\n", None, None, None);
        assert!(s.starts_with("compose.yaml: +1 -0 lines") && s.ends_with("; base none"));
    }

    #[test]
    fn a_line_ending_change_still_shows() {
        let d = unified("f", Some("a\n"), Some("a"));
        assert!(d.contains("-a\n+a\n"), "{d}");
    }
}
