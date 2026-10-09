//! Best-effort secret scrubbing for text that leaves the machine inside an Ask AI prompt.
//!
//! `grounding.rs` puts pod log lines and Kubernetes event messages verbatim into the system
//! prompt, and for a hosted provider (Anthropic / OpenAI-compatible / GitHub) that prompt is
//! sent to a third party. Workloads routinely log things they should not — bearer tokens,
//! JWTs, API keys, `password=` query strings, the occasional private key — so every log line
//! and event message passes through [`redact_text`] first.
//!
//! This is pattern matching, not a secret scanner: it catches the obvious shapes and keeps
//! the rest of the line intact so the model still sees the context. The UI's disclosure
//! says exactly that ("obvious secrets are scrubbed, best-effort"). The matching is
//! hand-written because `regex` is not a dependency of this crate. (`cloud_auth::redact` is the
//! narrower sibling for exec-plugin output: token shapes and long blobs only, no `key=value` or
//! PEM handling, which is what log lines need most.)

/// What a scrubbed token is replaced with.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// Scrub a multi-line block: PEM private-key bodies first (they span lines), then every line
/// through [`redact_line`]. Line structure is preserved exactly (including a trailing newline).
pub(crate) fn redact_text(text: &str) -> String {
    let text = redact_pem_blocks(text);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let (line, tail, nl) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..], true),
            None => (rest, "", false),
        };
        out.push_str(&redact_line(line));
        if nl {
            out.push('\n');
        }
        rest = tail;
    }
    out
}

/// Scrub a single line. The surrounding text is kept; only the sensitive token becomes
/// [`REDACTED`]. A line with nothing sensitive comes back unchanged.
pub(crate) fn redact_line(line: &str) -> String {
    let line = redact_bearer(line);
    let line = redact_key_values(&line);
    redact_token_words(&line)
}

// ---------------------------------------------------------------------------------------------
// PEM blocks

const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_END: &str = "-----END ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";

/// Replace the body of every `-----BEGIN ... PRIVATE KEY-----` … `-----END ... PRIVATE KEY-----`
/// block with a single `[REDACTED]` line. An unterminated block is scrubbed to the end of the
/// text, since the key material is exactly what follows the header.
fn redact_pem_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(begin) = find_pem_header(rest, PEM_BEGIN) else {
            out.push_str(rest);
            return out;
        };
        let header_end = begin.1;
        out.push_str(&rest[..header_end]);
        out.push('\n');
        out.push_str(REDACTED);
        let after_header = &rest[header_end..];
        match find_pem_header(after_header, PEM_END) {
            Some((end_start, end_end)) => {
                out.push('\n');
                out.push_str(&after_header[end_start..end_end]);
                rest = &after_header[end_end..];
            }
            None => return out,
        }
    }
}

/// Byte range of the first `<marker>... PRIVATE KEY-----` header in `s`.
fn find_pem_header(s: &str, marker: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(rel) = s[from..].find(marker) {
        let start = from + rel;
        let label_start = start + marker.len();
        // The label is a short run of uppercase words; bound the search to the same line.
        let line_end = s[label_start..]
            .find('\n')
            .map(|i| label_start + i)
            .unwrap_or(s.len());
        if let Some(rel_tail) = s[label_start..line_end].find(PEM_KEY_TAIL) {
            return Some((start, label_start + rel_tail + PEM_KEY_TAIL.len()));
        }
        from = label_start;
    }
    None
}

// ---------------------------------------------------------------------------------------------
// `Bearer <token>` (covers `Authorization: Bearer <token>` and a bare `Bearer <token>`)

fn redact_bearer(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if starts_with_ci(&bytes[i..], b"bearer") && (i == 0 || !is_ident(bytes[i - 1])) {
            let mut j = i + "bearer".len();
            let ws_start = j;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let tok_start = j;
            while j < bytes.len() && is_value_byte(bytes[j]) {
                j += 1;
            }
            if j > ws_start && j > tok_start && !is_placeholder(&line[tok_start..j]) {
                out.push_str(&line[i..tok_start]);
                out.push_str(REDACTED);
                i = j;
                continue;
            }
        }
        push_byte(&mut out, line, &mut i);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// `password=…`, `secret: …`, `"api_key": "…"` and friends

const SECRET_KEYS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "apikey"];

fn redact_key_values(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(key_len) = SECRET_KEYS
            .iter()
            .find(|k| starts_with_ci(&bytes[i..], k.as_bytes()))
            .map(|k| k.len())
        {
            let mut j = i + key_len;
            // The key must end here (so `tokens` or `secretary` are not keys), apart from an
            // optional closing quote as in `"token": "…"`.
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            let key_end = j;
            if key_end == i + key_len && j < bytes.len() && is_ident(bytes[j]) {
                push_byte(&mut out, line, &mut i);
                continue;
            }
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                j += 1;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    j += 1;
                }
                let val_start = j;
                while j < bytes.len() && is_value_byte(bytes[j]) {
                    j += 1;
                }
                if j > val_start && !is_placeholder(&line[val_start..j]) {
                    out.push_str(&line[i..val_start]);
                    out.push_str(REDACTED);
                    i = j;
                    continue;
                }
            }
        }
        push_byte(&mut out, line, &mut i);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Free-standing tokens recognisable by shape: JWTs, `sk-…`, `AKIA…`, GitHub tokens

fn redact_token_words(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if !is_word_byte(bytes[i]) {
            push_byte(&mut out, line, &mut i);
            continue;
        }
        let start = i;
        while i < bytes.len() && is_word_byte(bytes[i]) {
            i += 1;
        }
        // A sentence-final `.` is punctuation, not part of the token.
        let mut end = i;
        while end > start && bytes[end - 1] == b'.' {
            end -= 1;
        }
        let word = &line[start..end];
        if looks_like_secret_word(word) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        out.push_str(&line[end..i]);
    }
    out
}

fn looks_like_secret_word(word: &str) -> bool {
    is_jwt(word)
        || has_prefixed_key(word, "sk-", 8)
        || is_aws_access_key_id(word)
        || has_prefixed_key(word, "ghp_", 8)
        || has_prefixed_key(word, "gho_", 8)
        || has_prefixed_key(word, "github_pat_", 8)
}

/// `eyJ<base64url>.<base64url>.<base64url>` — a JSON web token: header and payload both start
/// with `{"` which base64-encodes to `eyJ`.
fn is_jwt(word: &str) -> bool {
    let mut parts = word.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    h.starts_with("eyJ")
        && p.starts_with("eyJ")
        && [h, p, s]
            .iter()
            .all(|seg| seg.len() >= 4 && seg.bytes().all(is_base64url))
}

fn has_prefixed_key(word: &str, prefix: &str, min_rest: usize) -> bool {
    word.strip_prefix(prefix).is_some_and(|rest| {
        rest.len() >= min_rest && rest.bytes().all(|b| is_base64url(b) || b == b'-')
    })
}

/// `AKIA` followed by exactly 16 uppercase letters or digits.
fn is_aws_access_key_id(word: &str) -> bool {
    word.len() == 20
        && word.starts_with("AKIA")
        && word
            .bytes()
            .skip(4)
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

// ---------------------------------------------------------------------------------------------
// Byte helpers (ASCII-only decisions; every slice boundary used above is an ASCII position, so
// it is always a `char` boundary).

fn starts_with_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len() && hay[..needle.len()].eq_ignore_ascii_case(needle)
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_base64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_word_byte(b: u8) -> bool {
    is_base64url(b) || b == b'.'
}

/// What a value runs up to: whitespace, quotes and the separators of query strings, JSON and
/// `key=value` lists. Anything else (including `[`/`]`, so an upstream `[REDACTED]` survives
/// whole) is treated as part of the value.
fn is_value_byte(b: u8) -> bool {
    !(b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b',' | b';' | b'&' | b'}'))
}

/// Already scrubbed (or scrubbed upstream) — never redact a redaction marker.
fn is_placeholder(value: &str) -> bool {
    value == REDACTED || value.eq_ignore_ascii_case("<redacted>") || value == "***"
}

/// Copy the (possibly multi-byte) char starting at `*i` and advance past it.
fn push_byte(out: &mut String, line: &str, i: &mut usize) {
    let ch = line[*i..].chars().next().expect("index is a char boundary");
    out.push(ch);
    *i += ch.len_utf8();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benign_line_passes_unchanged() {
        let line = "2026-10-09T12:00:00Z INFO server listening on 0.0.0.0:8080 (pid 1, tokens=3)";
        assert_eq!(redact_line(line), line);
        let text = "GET /healthz 200 1.2ms\nreconciled deployment default/api in 0.4s\n";
        assert_eq!(redact_text(text), text);
    }

    #[test]
    fn authorization_bearer_header() {
        assert_eq!(
            redact_line("Authorization: Bearer abc.DEF-123_xyz"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            redact_line("curl -H 'authorization: bearer s3cr3t' https://x"),
            "curl -H 'authorization: bearer [REDACTED]' https://x"
        );
        assert_eq!(
            redact_line("Bearer tok123 rejected"),
            "Bearer [REDACTED] rejected"
        );
        // No token after the word: nothing to scrub.
        assert_eq!(redact_line("expected a Bearer"), "expected a Bearer");
    }

    #[test]
    fn jwt() {
        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert_eq!(
            redact_line(&format!("got id token {jwt} from issuer.")),
            "got id token [REDACTED] from issuer."
        );
        // A dotted version string is not a JWT.
        assert_eq!(redact_line("image v1.2.3 pulled"), "image v1.2.3 pulled");
    }

    #[test]
    fn provider_api_keys() {
        assert_eq!(
            redact_line("key sk-ant-api03-AbCdEfGhIjKlMnOp set"),
            "key [REDACTED] set"
        );
        assert_eq!(
            redact_line("OPENAI sk-proj-1234567890abcdef"),
            "OPENAI [REDACTED]"
        );
        // Too short to be a key.
        assert_eq!(redact_line("task sk-1 done"), "task sk-1 done");
    }

    #[test]
    fn aws_access_key_id() {
        assert_eq!(
            redact_line("using AKIAIOSFODNN7EXAMPLE for s3"),
            "using [REDACTED] for s3"
        );
        assert_eq!(
            redact_line("AKIAIOSFODNN7EXAMPLE1 is 21 chars"),
            "AKIAIOSFODNN7EXAMPLE1 is 21 chars"
        );
    }

    #[test]
    fn key_value_pairs() {
        assert_eq!(
            redact_line("postgres://u:x@db?password=hunter2&sslmode=require"),
            "postgres://u:x@db?password=[REDACTED]&sslmode=require"
        );
        assert_eq!(redact_line("PASSWD: hunter2"), "PASSWD: [REDACTED]");
        assert_eq!(
            redact_line("client_secret=abc123;"),
            "client_secret=[REDACTED];"
        );
        assert_eq!(
            redact_line(r#"{"access_token": "abc.def", "expires": 3600}"#),
            r#"{"access_token": "[REDACTED]", "expires": 3600}"#
        );
        assert_eq!(redact_line("api_key = k-1"), "api_key = [REDACTED]");
        assert_eq!(redact_line("ApiKey:k2"), "ApiKey:[REDACTED]");
        // Words that merely start with a key are not keys.
        assert_eq!(
            redact_line("tokens=3 secretary=jo"),
            "tokens=3 secretary=jo"
        );
        // An empty value stays empty.
        assert_eq!(redact_line("password="), "password=");
    }

    #[test]
    fn private_key_block() {
        let text = "starting\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nAAAA\n-----END RSA PRIVATE KEY-----\ndone\n";
        assert_eq!(
            redact_text(text),
            "starting\n-----BEGIN RSA PRIVATE KEY-----\n[REDACTED]\n-----END RSA PRIVATE KEY-----\ndone\n"
        );
        // Unterminated: everything after the header is key material.
        assert_eq!(
            redact_text("-----BEGIN PRIVATE KEY-----\nMIIE\nmore"),
            "-----BEGIN PRIVATE KEY-----\n[REDACTED]"
        );
        // A certificate is not a private key.
        let cert = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        assert_eq!(redact_text(cert), cert);
    }

    #[test]
    fn github_tokens() {
        assert_eq!(redact_line("ghp_abcdefghijklmnop1234"), "[REDACTED]");
        assert_eq!(
            redact_line("token gho_ABCDEFGH12345678 ok"),
            "token [REDACTED] ok"
        );
        assert_eq!(redact_line("github_pat_11ABCDEF_abcdefghij"), "[REDACTED]");
    }

    #[test]
    fn already_redacted_and_unicode_are_left_alone() {
        assert_eq!(redact_line("password=[REDACTED]"), "password=[REDACTED]");
        assert_eq!(
            redact_line("Bearer *** — héllo wörld"),
            "Bearer *** — héllo wörld"
        );
        assert_eq!(
            redact_line("naïve token=ünïcode done"),
            "naïve token=[REDACTED] done"
        );
    }
}
