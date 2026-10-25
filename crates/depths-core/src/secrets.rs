//! Masking `.env` values before they leave the server (FR-021).
//!
//! Masking is by key: a key whose name contains `PASS`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE` or
//! `CREDENTIAL` (any case) has its value replaced. Everything else (comments, blank lines, other
//! keys, the line layout) is kept, so the editor shows the file's shape. `RevealEnv` sends the
//! real text (admin-only). Saving a draft that still shows the mask goes through [`unmask_env`],
//! so a user who edits `.env` without revealing it does not write asterisks over the secrets.

use std::collections::{HashMap, VecDeque};

/// Key fragments that mark a value as secret.
pub const SECRET_MARKERS: [&str; 6] = ["PASS", "SECRET", "TOKEN", "KEY", "PRIVATE", "CREDENTIAL"];

/// What a masked value reads as.
pub const MASK: &str = "********";

/// Whether `key` names a secret.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|m| k.contains(m))
}

/// `text` with every secret-looking value replaced by [`MASK`], or `None` when nothing in it
/// looks secret (the text can go out as it is).
pub fn mask_env(text: &str) -> Option<String> {
    let mut masked = false;
    // Inside a secret's quoted multi-line value: its quote. Those lines are dropped, so the
    // whole value reads as one [`MASK`].
    let mut inside: Option<char> = None;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => match b.strip_suffix('\r') {
                Some(b2) => (b2, "\r\n"),
                None => (b, "\n"),
            },
            None => (line, ""),
        };
        if let Some(q) = inside {
            if closes(body, q) {
                inside = None;
            }
            continue;
        }
        let trimmed = body.trim_start();
        let assignment = (!trimmed.starts_with('#'))
            .then(|| body.split_once('='))
            .flatten();
        match assignment {
            Some((lhs, value)) => {
                let key = lhs.trim().trim_start_matches("export ").trim();
                if is_secret_key(key) {
                    masked = true;
                    out.push_str(lhs);
                    out.push('=');
                    out.push_str(MASK);
                    let v = value.trim_start();
                    if let Some(q @ ('"' | '\'')) = v.chars().next()
                        && !closes(&v[1..], q)
                    {
                        inside = Some(q);
                    }
                } else {
                    out.push_str(body);
                }
            }
            None => out.push_str(body),
        }
        out.push_str(eol);
    }
    masked.then_some(out)
}

/// One `KEY=value` assignment as [`mask_env`] reads it: the key, the text before `=`, and the
/// value as written (a secret's quoted multi-line value whole, with its inner line ends).
struct Assignment<'a> {
    key: &'a str,
    lhs: &'a str,
    value: String,
}

/// One piece of an env file: an assignment with its final line end, or a line kept as it is.
enum Piece<'a> {
    Assign(Assignment<'a>, &'a str),
    Other(&'a str),
}

/// The file split the way [`mask_env`] splits it: a secret's quoted value spans its lines, any
/// other line stands alone.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut lines = text.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let (body, eol) = split_eol(line);
        let trimmed = body.trim_start();
        let assignment = (!trimmed.starts_with('#'))
            .then(|| body.split_once('='))
            .flatten();
        let Some((lhs, value)) = assignment else {
            out.push(Piece::Other(line));
            continue;
        };
        let key = lhs.trim().trim_start_matches("export ").trim();
        let mut value = value.to_owned();
        let mut end = eol;
        let v = value.trim_start().to_owned();
        if is_secret_key(key)
            && let Some(q @ ('"' | '\'')) = v.chars().next()
            && !closes(&v[1..], q)
        {
            for next in lines.by_ref() {
                let (b, e) = split_eol(next);
                value.push_str(end);
                value.push_str(b);
                end = e;
                if closes(b, q) {
                    break;
                }
            }
        }
        out.push(Piece::Assign(Assignment { key, lhs, value }, end));
    }
    out
}

/// A line's body and its line end (`\r\n`, `\n` or none).
fn split_eol(line: &str) -> (&str, &str) {
    match line.strip_suffix('\n') {
        Some(b) => match b.strip_suffix('\r') {
            Some(b2) => (b2, "\r\n"),
            None => (b, "\n"),
        },
        None => (line, ""),
    }
}

/// The secret-looking keys `text` assigns, in file order, each once. For the audit entry of a
/// reveal: key names only, never values.
pub fn secret_keys(text: &str) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    for p in pieces(text) {
        if let Piece::Assign(a, _) = p
            && is_secret_key(a.key)
            && !seen.iter().any(|k| k == a.key)
        {
            seen.push(a.key.to_owned());
        }
    }
    seen
}

/// `draft` with the real value put back on every line that still reads `KEY=********` for a
/// secret-looking `KEY` that `current` assigns, so the masked text a user saves without revealing
/// keeps the secrets it showed as asterisks. A renamed key, a changed value and a key `current`
/// does not assign are left as written. A key assigned more than once is matched in order.
pub fn unmask_env(draft: &str, current: Option<&str>) -> String {
    let Some(current) = current else {
        return draft.to_owned();
    };
    let mut originals: HashMap<&str, VecDeque<String>> = HashMap::new();
    for p in pieces(current) {
        if let Piece::Assign(a, _) = p
            && is_secret_key(a.key)
        {
            originals.entry(a.key).or_default().push_back(a.value);
        }
    }
    let mut out = String::with_capacity(draft.len());
    for p in pieces(draft) {
        match p {
            Piece::Other(line) => out.push_str(line),
            Piece::Assign(a, eol) => {
                let restored = (a.value.trim() == MASK && is_secret_key(a.key))
                    .then(|| originals.get_mut(a.key))
                    .flatten()
                    .and_then(|q| {
                        if q.len() > 1 {
                            q.pop_front()
                        } else {
                            q.front().cloned()
                        }
                    });
                out.push_str(a.lhs);
                out.push('=');
                out.push_str(restored.as_deref().unwrap_or(&a.value));
                out.push_str(eol);
            }
        }
    }
    out
}

/// Whether `s` holds the closing `q` of a quoted value: one not escaped with a backslash.
pub fn closes(s: &str, q: char) -> bool {
    let mut escaped = false;
    for c in s.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            c if c == q && !escaped => return true,
            _ => escaped = false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_values_are_masked_and_the_rest_kept() {
        let text =
            "# db\nDB_PASSWORD=hunter2\nPORT=8080\nexport API_Key = abc=def\n\nnot an assignment\n";
        let m = mask_env(text).unwrap();
        assert!(!m.contains("hunter2") && !m.contains("abc=def"), "{m}");
        assert_eq!(
            m,
            "# db\nDB_PASSWORD=********\nPORT=8080\nexport API_Key =********\n\nnot an assignment\n"
        );
    }

    #[test]
    fn every_marker_matches_in_any_case() {
        for k in [
            "pass",
            "MY_SECRET",
            "gh_token",
            "ssh_key",
            "private_x",
            "AWS_CREDENTIALS",
        ] {
            assert!(is_secret_key(k), "{k}");
        }
        assert!(!is_secret_key("PORT"));
    }

    #[test]
    fn nothing_secret_is_none() {
        assert_eq!(mask_env("PORT=1\n# TOKEN=x in a comment\n"), None);
    }

    #[test]
    fn a_multi_line_secret_is_masked_whole() {
        // T099 F5: Compose reads a quoted value across lines. Masking the first line only sent
        // the body of a PEM key to everyone who can view the stack, without the audited reveal.
        let text = "A=1\nPRIVATE_KEY=\"-----BEGIN KEY-----\nMIIEsecret\nYWJj==\n-----END KEY-----\"\nB=2\nSSH_KEY='x\ny'\nTOKEN=\"one line\"\nC=3\n";
        let m = mask_env(text).unwrap();
        for leak in [
            "MIIE", "YWJj", "BEGIN", "END KEY", "x\n", "\ny'", "one line",
        ] {
            assert!(!m.contains(leak), "{leak:?} leaked: {m}");
        }
        assert_eq!(
            m,
            "A=1\nPRIVATE_KEY=********\nB=2\nSSH_KEY=********\nTOKEN=********\nC=3\n"
        );
        // An unterminated quote masks to the end: nothing after it can be told from the value.
        let m = mask_env("API_TOKEN=\"abc\nmore\nPORT=1\n").unwrap();
        assert_eq!(m, "API_TOKEN=********\n");
    }

    #[test]
    fn crlf_and_no_trailing_newline_are_kept() {
        assert_eq!(
            mask_env("A_TOKEN=x\r\nB=y").unwrap(),
            "A_TOKEN=********\r\nB=y"
        );
    }

    #[test]
    fn a_masked_draft_saved_unchanged_keeps_every_secret() {
        let current = "# db\nDB_PASSWORD=hunter2\nPORT=8080\nexport API_Key = abc=def\n\
                       PRIVATE_KEY=\"-----BEGIN KEY-----\nMIIE\n-----END KEY-----\"\nB=2";
        let masked = mask_env(current).unwrap();
        assert_eq!(unmask_env(&masked, Some(current)), current);
    }

    #[test]
    fn an_edit_around_masked_secrets_keeps_them_and_takes_the_edit() {
        let current = "DB_PASSWORD=hunter2\r\nAPI_TOKEN=t0k\r\nPORT=8080\r\n";
        let draft =
            "DB_PASSWORD=********\r\nAPI_TOKEN=new-token\r\nPORT=9090\r\nNEW_SECRET=********\r\n";
        assert_eq!(
            unmask_env(draft, Some(current)),
            "DB_PASSWORD=hunter2\r\nAPI_TOKEN=new-token\r\nPORT=9090\r\nNEW_SECRET=********\r\n"
        );
    }

    #[test]
    fn a_renamed_key_does_not_take_the_old_value() {
        let current = "DB_PASSWORD=hunter2\n";
        assert_eq!(
            unmask_env("OTHER_PASSWORD=********\n", Some(current)),
            "OTHER_PASSWORD=********\n"
        );
        // A key that does not look secret was never masked: asterisks there are the value.
        assert_eq!(
            unmask_env("PORT=********\n", Some("PORT=1\n")),
            "PORT=********\n"
        );
    }

    #[test]
    fn repeated_keys_are_matched_in_order_and_no_current_keeps_the_draft() {
        let current = "A_KEY=one\nA_KEY=two\n";
        assert_eq!(
            unmask_env(
                "A_KEY=********\nA_KEY=********\nA_KEY=********\n",
                Some(current)
            ),
            "A_KEY=one\nA_KEY=two\nA_KEY=two\n"
        );
        assert_eq!(unmask_env("A_KEY=********\n", None), "A_KEY=********\n");
    }

    #[test]
    fn secret_keys_names_each_secret_once_without_values() {
        assert_eq!(
            secret_keys("DB_PASSWORD=a\nPORT=1\nPRIVATE_KEY=\"x\nTOKEN=y\"\nDB_PASSWORD=b\n"),
            ["DB_PASSWORD", "PRIVATE_KEY"]
        );
    }
}
