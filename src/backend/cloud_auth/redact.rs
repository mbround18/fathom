//! Credential redaction for plugin output (FR-004, FR-009). Redacts by token *shape*
//! (JWT, `k8s-aws-v1.`, `ya29.`, bearer values, long opaque blobs) and, when known, by exact
//! secret value, then truncates on a char boundary.

const REDACTED: &str = "[REDACTED]";

/// Characters that separate "words" when scanning for token shapes.
fn is_delim(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | ',' | ':' | '=' | ';' | '{' | '}' | '[' | ']'
        )
}

fn looks_like_secret(word: &str) -> bool {
    if word.starts_with("eyJ") && word.matches('.').count() >= 2 {
        return true; // JWT
    }
    if word.starts_with("k8s-aws-v1.") || word.starts_with("ya29.") {
        return true;
    }
    // Long opaque base64/hex/url-safe blob.
    word.len() >= 40
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '='))
}

/// Replace secret-shaped words in `input` (and every literal in `secrets`) with `[REDACTED]`.
pub fn redact_with(input: &str, secrets: &[&str]) -> String {
    let mut text = input.to_owned();
    for secret in secrets.iter().filter(|s| s.len() >= 4) {
        text = text.replace(secret, REDACTED);
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if looks_like_secret(word) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if is_delim(c) {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// [`redact_with`] with no known literals.
pub fn redact(input: &str) -> String {
    redact_with(input, &[])
}

/// Truncate to at most `max_bytes` on a char boundary, marking the cut.
pub fn truncate(input: &str, max_bytes: usize) -> String {
    const MARK: &str = "...[truncated]";
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    let budget = max_bytes.saturating_sub(MARK.len());
    let mut end = budget.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &input[..end])
}

/// Redact, then truncate: the form used for every message that leaves the backend.
pub fn sanitize(input: &str, max_bytes: usize, secrets: &[&str]) -> String {
    truncate(&redact_with(input, secrets), max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";

    #[test]
    fn redacts_jwt_aws_and_google_shapes() {
        for tok in [JWT, "k8s-aws-v1.aHR0cHM6Ly9zdHM", "ya29.a0AfH6SMB"] {
            let out = redact(&format!("error: token {tok} rejected"));
            assert!(!out.contains(tok), "{out}");
            assert!(out.contains("[REDACTED]"));
            assert!(out.contains("rejected"));
        }
    }

    #[test]
    fn redacts_json_token_values_and_long_blobs() {
        let blob = "A".repeat(64);
        let out = redact(&format!("{{\"token\":\"{blob}\"}}"));
        assert!(!out.contains(&blob), "{out}");
        assert!(out.contains("token"));
    }

    #[test]
    fn redacts_known_literal() {
        let out = redact_with("password hunter22 leaked", &["hunter22"]);
        assert_eq!(out, "password [REDACTED] leaked");
    }

    #[test]
    fn plain_text_is_untouched() {
        let s = "Unable to locate credentials. You can configure credentials by running \"aws configure\".";
        assert_eq!(redact(s), s);
    }

    #[test]
    fn truncates_on_char_boundary() {
        let s = "é".repeat(100);
        let out = truncate(&s, 33);
        assert!(out.len() <= 33, "{}", out.len());
        assert!(out.ends_with("[truncated]"));
        assert_eq!(truncate("short", 33), "short");
    }
}
