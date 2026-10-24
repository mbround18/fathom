//! Masking `.env` values before they leave the server (FR-021).
//!
//! Masking is by key: a key whose name contains `PASS`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE` or
//! `CREDENTIAL` (any case) has its value replaced. Everything else (comments, blank lines, other
//! keys, the line layout) is kept, so the editor shows the file's shape. `RevealEnv` sends the
//! real text.

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

/// Whether `s` holds the closing `q` of a quoted value: one not escaped with a backslash.
fn closes(s: &str, q: char) -> bool {
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
}
