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
    fn a_line_ending_change_still_shows() {
        let d = unified("f", Some("a\n"), Some("a"));
        assert!(d.contains("-a\n+a\n"), "{d}");
    }
}
