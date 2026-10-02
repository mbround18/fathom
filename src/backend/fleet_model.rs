//! 046-fleet-model: the label grammar and the selector grammar, in one module.
//!
//! Two vocabularies live here because they share an error vocabulary (research §1, §2):
//!
//! - **Label validation** (FR-003). Kubernetes' own label rules, implemented here because there
//!   is nothing to borrow: `k8s-openapi` ships types and `kube` validates nothing client-side.
//!   Every error names the failing input *and* the rule it broke, because that string is shown
//!   to the operator unchanged.
//! - **Selector parsing** (FR-006). `kube_core::Selector`/`Expression` give us the
//!   representation, `Display` back to the string `ListParams::labels()` takes, and
//!   `SelectorExt::matches` for set semantics — but there is no `FromStr` anywhere in
//!   `kube-core-2.0.1/src/labels.rs`, so the operator-typed string has no library path in. This
//!   is that path: the `kubectl -l` subset, with the offending substring carried in the error.
//!
//! Matching is *not* re-implemented: `SelectorExt::matches` is library code, already tested
//! upstream, and re-implementing set semantics is how `in`/`notin` on a missing key goes subtly
//! wrong.
//!
//! This module is deliberately self-contained — it speaks only `&str`, `BTreeMap<String, String>`
//! and kube's own types, so the protocol types (`FleetMember`, `FleetGroup`) can evolve without
//! touching the grammar.

use std::collections::BTreeMap;
use std::fmt;

use kube::core::{Expression, Selector, SelectorExt};

/// Maximum length of a label name (a key's name part, or a value).
const NAME_MAX: usize = 63;
/// Maximum length of a key's optional DNS-subdomain prefix.
const PREFIX_MAX: usize = 253;

/// The rules a label can break. Each is phrased as the sentence the operator reads.
mod rule {
    pub const KEY_EMPTY: &str = "a label key must not be empty";
    pub const KEY_ONE_SLASH: &str = "a label key has at most one \"/\", separating an optional DNS-subdomain prefix from the name";
    pub const PREFIX_EMPTY: &str =
        "the prefix before \"/\" must not be empty; omit the \"/\" if there is no prefix";
    pub const PREFIX_TOO_LONG: &str = "the prefix before \"/\" must be 1-253 characters";
    pub const PREFIX_CHARS: &str = "the prefix before \"/\" must be a DNS subdomain: lowercase alphanumerics, \"-\" and \".\" only";
    pub const NAME_EMPTY: &str = "the name part must not be empty";
    pub const NAME_TOO_LONG: &str = "must be 1-63 characters";
    pub const NAME_ENDS: &str = "must begin and end with an alphanumeric character";
    pub const NAME_CHARS: &str = "may contain only alphanumerics, \"-\", \"_\" and \".\" between its first and last character";
    pub const VALUE_TOO_LONG: &str = "a label value must be 1-63 characters, or empty";
}

/// Which half of a label failed, so a caller can name the field in `FleetModelRejected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelPart {
    /// The label key.
    Key,
    /// The label value.
    Value,
}

impl LabelPart {
    /// The noun used in the operator-visible message.
    pub fn noun(self) -> &'static str {
        match self {
            LabelPart::Key => "label key",
            LabelPart::Value => "label value",
        }
    }
}

/// A label that does not satisfy [the Kubernetes rules](self).
///
/// `Display` names the failing input *and* the rule it broke, verbatim — FR-003 shows this string
/// to the operator unchanged, so neither half may be dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelError {
    /// Key or value.
    pub part: LabelPart,
    /// The input exactly as the operator supplied it.
    pub input: String,
    /// The rule it broke.
    pub rule: String,
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} \"{}\" is invalid: {}",
            self.part.noun(),
            self.input,
            self.rule
        )
    }
}

impl std::error::Error for LabelError {}

/// A selector string that is not in the supported `kubectl -l` subset.
///
/// `Display` always contains [`Self::offending`], so FR-006's promise that the operator sees the
/// parse error verbatim has content to be verbatim about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorParseError {
    /// The whole selector as typed.
    pub selector: String,
    /// The substring the parser choked on.
    pub offending: String,
    /// The rule it broke.
    pub rule: String,
}

impl fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "selector \"{}\" is invalid at \"{}\": {}",
            self.selector, self.offending, self.rule
        )
    }
}

impl std::error::Error for SelectorParseError {}

impl SelectorParseError {
    fn new(selector: &str, offending: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            selector: selector.to_string(),
            offending: offending.into(),
            rule: rule.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// T014 — label validation
// ---------------------------------------------------------------------------

/// Validate a label key: an optional DNS-subdomain prefix (1–253 chars, lowercase alphanumerics
/// plus `-` and `.`) followed by `/`, then a name of 1–63 characters, alphanumeric at both ends,
/// with `-`, `_` and `.` permitted in the interior.
pub fn validate_label_key(key: &str) -> Result<(), LabelError> {
    let err = |rule: &str| {
        Err(LabelError {
            part: LabelPart::Key,
            input: key.to_string(),
            rule: rule.to_string(),
        })
    };

    if key.is_empty() {
        return err(rule::KEY_EMPTY);
    }

    let mut parts = key.split('/');
    let (prefix, name) = match (parts.next(), parts.next(), parts.next()) {
        (Some(name), None, _) => (None, name),
        (Some(prefix), Some(name), None) => (Some(prefix), name),
        // Three or more segments: more than one "/".
        _ => return err(rule::KEY_ONE_SLASH),
    };

    if let Some(prefix) = prefix {
        if prefix.is_empty() {
            return err(rule::PREFIX_EMPTY);
        }
        if prefix.len() > PREFIX_MAX {
            return err(rule::PREFIX_TOO_LONG);
        }
        if !prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        {
            return err(rule::PREFIX_CHARS);
        }
    }

    match check_name(name) {
        Ok(()) => Ok(()),
        Err(rule) => err(rule),
    }
}

/// Validate a label value: empty, or the same 1–63-character name shape as a key's name part.
pub fn validate_label_value(value: &str) -> Result<(), LabelError> {
    if value.is_empty() {
        // An empty value is legal in a label map. (It is *not* legal as a selector requirement —
        // see `parse_selector`.)
        return Ok(());
    }
    check_name(value).map_err(|rule| LabelError {
        part: LabelPart::Value,
        input: value.to_string(),
        // "the name part" reads wrong for a value; the length rule gets the value wording.
        rule: if rule == rule::NAME_TOO_LONG {
            rule::VALUE_TOO_LONG.to_string()
        } else {
            rule.to_string()
        },
    })
}

/// Validate one key/value pair.
pub fn validate_label(key: &str, value: &str) -> Result<(), LabelError> {
    validate_label_key(key)?;
    validate_label_value(value)
}

/// Validate a whole label map, failing on the first offender. Callers validate *everything*
/// before writing anything (FR-003).
pub fn validate_labels(labels: &BTreeMap<String, String>) -> Result<(), LabelError> {
    for (key, value) in labels {
        validate_label(key, value)?;
    }
    Ok(())
}

/// The shared 1–63-character name shape. Returns the rule it broke.
fn check_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err(rule::NAME_EMPTY);
    }
    if name.chars().count() > NAME_MAX {
        return Err(rule::NAME_TOO_LONG);
    }
    let chars: Vec<char> = name.chars().collect();
    let first = chars[0];
    let last = chars[chars.len() - 1];
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err(rule::NAME_ENDS);
    }
    if !chars
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
    {
        return Err(rule::NAME_CHARS);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// T015 — selector parsing
// ---------------------------------------------------------------------------

/// Parse the `kubectl -l` subset into a [`Selector`].
///
/// Accepted: comma-separated requirements, each one of `k=v`, `k==v`, `k!=v`, `k in (a,b)`,
/// `k notin (a,b)`, `k` (exists), `!k` (does not exist). Whitespace around any token is
/// tolerated. Every key and every value is put through [`validate_label_key`] /
/// [`validate_label_value`], so a selector cannot name a label that could never be set.
///
/// **`env=` is rejected.** An empty label *value* is legal in a label map, but an empty value in
/// a *selector requirement* is not: it is nearly always a half-typed `env=prod`, and the fleet
/// model has no use for "equals the empty string" that `env` (exists) and `!env` do not already
/// cover. The same goes for `env!=` and for an empty member of a set, `env in (prod,)`.
///
/// An empty selector is rejected too, rather than quietly meaning "every member": a group whose
/// selector matches the whole fleet must say so by selecting on something.
pub fn parse_selector(input: &str) -> Result<Selector, SelectorParseError> {
    let requirements = split_requirements(input);
    if requirements.iter().all(|r| r.trim().is_empty()) {
        return Err(SelectorParseError::new(
            input,
            input,
            "a selector must contain at least one requirement",
        ));
    }

    let mut expressions: Vec<Expression> = Vec::with_capacity(requirements.len());
    for raw in requirements {
        let requirement = raw.trim();
        if requirement.is_empty() {
            return Err(SelectorParseError::new(
                input,
                raw,
                "an empty requirement between commas",
            ));
        }
        expressions.push(parse_requirement(input, requirement)?);
    }
    Ok(expressions.into_iter().collect())
}

/// Split on commas that are not inside a `( ... )` set.
fn split_requirements(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

fn parse_requirement(input: &str, requirement: &str) -> Result<Expression, SelectorParseError> {
    // `!k` — does not exist.
    if let Some(rest) = requirement.strip_prefix('!') {
        let key = rest.trim();
        validate_key_in_selector(input, requirement, key)?;
        return Ok(Expression::DoesNotExist(key.to_string()));
    }

    // The key runs up to the first character that cannot appear in a key. That boundary is what
    // separates `env` from `in (a,b)` without needing a lexer.
    let key_len = requirement
        .char_indices()
        .find(|(_, c)| !is_key_char(*c))
        .map(|(i, _)| i)
        .unwrap_or(requirement.len());
    let key = &requirement[..key_len];
    let rest = requirement[key_len..].trim();

    if rest.is_empty() {
        // Bare `k` — exists.
        validate_key_in_selector(input, requirement, key)?;
        return Ok(Expression::Exists(key.to_string()));
    }

    if let Some(value) = rest.strip_prefix("!=") {
        validate_key_in_selector(input, requirement, key)?;
        let value = non_empty_value(input, requirement, value.trim())?;
        return Ok(Expression::NotEqual(key.to_string(), value));
    }
    if let Some(value) = rest.strip_prefix("==").or_else(|| rest.strip_prefix('=')) {
        validate_key_in_selector(input, requirement, key)?;
        let value = non_empty_value(input, requirement, value.trim())?;
        return Ok(Expression::Equal(key.to_string(), value));
    }
    // `notin` before `in`: the latter is a prefix-free check only after the former is ruled out.
    if let Some(set) = rest.strip_prefix("notin") {
        validate_key_in_selector(input, requirement, key)?;
        let values = parse_set(input, requirement, set.trim())?;
        return Ok(Expression::NotIn(key.to_string(), values));
    }
    if let Some(set) = rest.strip_prefix("in") {
        validate_key_in_selector(input, requirement, key)?;
        let values = parse_set(input, requirement, set.trim())?;
        return Ok(Expression::In(key.to_string(), values));
    }

    Err(SelectorParseError::new(
        input,
        requirement,
        "expected one of \"=\", \"==\", \"!=\", \" in (...)\", \" notin (...)\", a bare key, or \"!key\"",
    ))
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/'
}

fn validate_key_in_selector(
    input: &str,
    requirement: &str,
    key: &str,
) -> Result<(), SelectorParseError> {
    validate_label_key(key).map_err(|e| {
        // The offending substring is the key when there is one to point at, and the whole
        // requirement when the key is what is missing.
        let offending = if key.is_empty() { requirement } else { key };
        SelectorParseError::new(input, offending, e.rule)
    })
}

fn non_empty_value(
    input: &str,
    requirement: &str,
    value: &str,
) -> Result<String, SelectorParseError> {
    if value.is_empty() {
        return Err(SelectorParseError::new(
            input,
            requirement,
            "a requirement must give a non-empty value; use a bare key for \"exists\" or \"!key\" for \"does not exist\"",
        ));
    }
    validate_label_value(value).map_err(|e| SelectorParseError::new(input, value, e.rule))?;
    Ok(value.to_string())
}

fn parse_set(
    input: &str,
    requirement: &str,
    set: &str,
) -> Result<std::collections::BTreeSet<String>, SelectorParseError> {
    let inner = set
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| {
            SelectorParseError::new(
                input,
                requirement,
                "the values of an \"in\"/\"notin\" requirement must be parenthesised, as in \"key in (a,b)\"",
            )
        })?;

    let mut values = std::collections::BTreeSet::new();
    for raw in inner.split(',') {
        let value = raw.trim();
        if value.is_empty() {
            return Err(SelectorParseError::new(
                input,
                requirement,
                "an \"in\"/\"notin\" set must list at least one value and no empty value",
            ));
        }
        validate_label_value(value).map_err(|e| SelectorParseError::new(input, value, e.rule))?;
        values.insert(value.to_string());
    }
    Ok(values)
}

// ---------------------------------------------------------------------------
// T016 — membership
// ---------------------------------------------------------------------------

/// Whether a member's labels satisfy a group's selector (FR-004).
///
/// Delegates to `kube::core::SelectorExt::matches` — the set semantics (`notin` on a missing key
/// is a *match*, `in` on a missing key is not) are library code, not ours.
pub fn selector_matches(selector: &Selector, labels: &BTreeMap<String, String>) -> bool {
    selector.matches(labels)
}

/// Every group whose selector matches these labels, in the order the groups were given.
///
/// A member appears in *every* matching group — group membership is computed, never stored
/// (FR-004). Kept generic over `(name, selector)` pairs so this module stays free of the fleet
/// protocol types.
pub fn groups_matching<'a, I>(groups: I, labels: &BTreeMap<String, String>) -> Vec<&'a str>
where
    I: IntoIterator<Item = (&'a str, &'a Selector)>,
{
    groups
        .into_iter()
        .filter(|(_, selector)| selector_matches(selector, labels))
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::ListParams;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // -------------------------------------------------------------------
    // T018 — the label validator
    // -------------------------------------------------------------------

    #[test]
    fn valid_labels_are_accepted() {
        let cases: &[(&str, &str, &str)] = &[
            ("env", "prod", "bare name, both ends alphanumeric"),
            ("a", "b", "single character key and value"),
            ("env", "", "an empty value is legal in a label map"),
            ("team.fathom.dev/env", "prod", "DNS-subdomain prefix"),
            (
                "k8s.io/role",
                "control-plane",
                "prefix plus hyphen in value",
            ),
            (
                "1-2.3_4",
                "9.8_7-6",
                "interior -, _ and . at both ends alnum",
            ),
            ("a9", &"x".repeat(63), "a 63-character value is the maximum"),
            (
                &format!("{}/{}", "a".repeat(253), "n".repeat(63)),
                "v",
                "prefix and name at their maxima",
            ),
        ];
        for (key, value, why) in cases {
            assert!(
                validate_label(key, value).is_ok(),
                "{why}: expected {key:?}={value:?} to be accepted, got {:?}",
                validate_label(key, value)
            );
        }
    }

    #[test]
    fn invalid_labels_name_the_input_and_the_rule() {
        // (key, value, the rule the message must name)
        let cases: &[(&str, &str, &str)] = &[
            ("", "prod", rule::KEY_EMPTY),
            ("a/b/c", "prod", rule::KEY_ONE_SLASH),
            ("/env", "prod", rule::PREFIX_EMPTY),
            (
                &format!("{}/env", "a".repeat(254)),
                "prod",
                rule::PREFIX_TOO_LONG,
            ),
            ("Team.Fathom.Dev/env", "prod", rule::PREFIX_CHARS),
            ("fathom.dev/", "prod", rule::NAME_EMPTY),
            (&"n".repeat(64), "prod", rule::NAME_TOO_LONG),
            ("-env", "prod", rule::NAME_ENDS),
            ("env-", "prod", rule::NAME_ENDS),
            ("en v", "prod", rule::NAME_CHARS),
            ("env", "-prod", rule::NAME_ENDS),
            ("env", "prod-", rule::NAME_ENDS),
            ("env", "pr od", rule::NAME_CHARS),
            ("env", &"v".repeat(64), rule::VALUE_TOO_LONG),
        ];
        for (key, value, expected_rule) in cases {
            let err = validate_label(key, value)
                .expect_err(&format!("expected {key:?}={value:?} to be rejected"));
            let message = err.to_string();
            assert_eq!(
                &err.rule, expected_rule,
                "wrong rule for {key:?}={value:?}: {message}"
            );
            assert!(
                message.contains(expected_rule),
                "message must name the rule verbatim, got: {message}"
            );
            // The message must also name the failing input verbatim.
            assert!(
                message.contains(&err.input),
                "message must name the failing input verbatim, got: {message}"
            );
            assert_eq!(
                err.input,
                if err.part == LabelPart::Key {
                    *key
                } else {
                    *value
                }
            );
        }
    }

    #[test]
    fn a_whole_map_fails_on_its_first_offender() {
        let ok = labels(&[("env", "prod"), ("team", "")]);
        assert!(validate_labels(&ok).is_ok());

        let bad = labels(&[("env", "prod"), ("Zone/eu", "1")]);
        let err = validate_labels(&bad).expect_err("invalid prefix must be rejected");
        assert_eq!(err.input, "Zone/eu");
        assert_eq!(err.part, LabelPart::Key);
    }

    // -------------------------------------------------------------------
    // T017 — the selector parser
    // -------------------------------------------------------------------

    #[test]
    fn every_accepted_form_round_trips_through_display() {
        // (input, the exact string Display produces)
        let cases: &[(&str, &str)] = &[
            ("env=prod", "env=prod"),
            ("env==prod", "env=prod"),
            ("env!=prod", "env!=prod"),
            ("env in (prod,stage)", "env in (prod,stage)"),
            // Display sorts a set: it is a BTreeSet upstream.
            ("env in (stage,prod)", "env in (prod,stage)"),
            ("env notin (dev)", "env notin (dev)"),
            ("env", "env"),
            ("!env", "!env"),
            // Whitespace around every token is tolerated.
            ("  env = prod  ", "env=prod"),
            ("env in ( prod , stage )", "env in (prod,stage)"),
            (" ! env ", "!env"),
            // Comma-separated requirements, in the order typed.
            (
                "env=prod,tier!=db,region in (eu,us),!legacy,managed",
                "env=prod,tier!=db,region in (eu,us),!legacy,managed",
            ),
            ("team.fathom.dev/env=prod", "team.fathom.dev/env=prod"),
        ];

        for (input, expected) in cases {
            let selector = parse_selector(input)
                .unwrap_or_else(|e| panic!("expected {input:?} to parse, got: {e}"));
            let rendered = selector.to_string();
            assert_eq!(&rendered, expected, "Display mismatch for {input:?}");

            // The rendered form is what `ListParams::labels()` takes, and it parses back to the
            // same selector — which is how 047 gets its scope expression for free.
            let _params = ListParams::default().labels(&rendered);
            let reparsed = parse_selector(&rendered)
                .unwrap_or_else(|e| panic!("Display output {rendered:?} must re-parse, got: {e}"));
            assert_eq!(
                reparsed, selector,
                "round-trip changed the selector for {input:?}"
            );
            assert_eq!(reparsed.to_string(), rendered);
        }
    }

    #[test]
    fn rejected_selectors_carry_the_offending_substring() {
        let long_name = "n".repeat(64);
        let two_slashes = "a/b/c=prod";
        // (input, the substring the message must contain, the rule fragment it must name)
        let cases: &[(&str, &str, &str)] = &[
            // `in` without a parenthesised set.
            ("env in prod", "env in prod", "parenthesised"),
            // No key.
            ("=prod", "=prod", rule::KEY_EMPTY),
            // `env=` — see parse_selector's doc comment: legal in a label map, not as a
            // requirement.
            ("env=", "env=", "non-empty value"),
            ("env!=", "env!=", "non-empty value"),
            ("env in (prod,)", "env in (prod,)", "empty value"),
            ("env in ()", "env in ()", "empty value"),
            // A 64-character label name.
            (
                &format!("{long_name}=prod"),
                &long_name,
                rule::NAME_TOO_LONG,
            ),
            // A key with two slashes.
            (two_slashes, "a/b/c", rule::KEY_ONE_SLASH),
            // An empty selector matches everything by accident; say no instead.
            ("", "", "at least one requirement"),
            ("   ", "   ", "at least one requirement"),
            ("env=prod,", "", "empty requirement"),
            // Nonsense operator.
            ("env ~ prod", "env ~ prod", "expected one of"),
        ];

        for (input, offending, rule_fragment) in cases {
            let err = parse_selector(input)
                .map(|s| s.to_string())
                .expect_err(&format!("expected {input:?} to be rejected, it parsed"));
            let message = err.to_string();
            assert!(
                err.offending.contains(offending.trim()) || offending.trim().is_empty(),
                "offending substring for {input:?} should contain {offending:?}, got {:?}",
                err.offending
            );
            assert!(
                message.contains(&err.offending),
                "the message must carry the offending substring, got: {message}"
            );
            assert!(
                message.contains(rule_fragment),
                "the message for {input:?} must name the rule ({rule_fragment:?}), got: {message}"
            );
            assert_eq!(
                err.selector, *input,
                "the error must quote the selector as typed"
            );
        }
    }

    // -------------------------------------------------------------------
    // T016 — membership
    // -------------------------------------------------------------------

    #[test]
    fn membership_uses_kube_set_semantics() {
        let member = labels(&[("env", "prod"), ("region", "eu")]);

        assert!(selector_matches(
            &parse_selector("env=prod").unwrap(),
            &member
        ));
        assert!(!selector_matches(
            &parse_selector("env=stage").unwrap(),
            &member
        ));
        assert!(selector_matches(
            &parse_selector("env in (prod,stage)").unwrap(),
            &member
        ));
        assert!(selector_matches(
            &parse_selector("env,region").unwrap(),
            &member
        ));
        assert!(selector_matches(&parse_selector("!tier").unwrap(), &member));

        // The subtle pair: on a *missing* key `in` does not match and `notin` does.
        assert!(!selector_matches(
            &parse_selector("tier in (db)").unwrap(),
            &member
        ));
        assert!(selector_matches(
            &parse_selector("tier notin (db)").unwrap(),
            &member
        ));
        // And `!=` on a missing key matches, matching kubectl.
        assert!(selector_matches(
            &parse_selector("tier!=db").unwrap(),
            &member
        ));
    }

    #[test]
    fn a_member_joins_every_group_whose_selector_matches() {
        let prod = parse_selector("env=prod").unwrap();
        let eu = parse_selector("region in (eu,uk)").unwrap();
        let legacy = parse_selector("legacy").unwrap();
        let groups = vec![("prod", &prod), ("eu", &eu), ("legacy", &legacy)];

        let member = labels(&[("env", "prod"), ("region", "eu")]);
        assert_eq!(groups_matching(groups.clone(), &member), vec!["prod", "eu"]);

        // A member matching nothing is simply in no group; it is still a member (FR-017).
        let orphan = labels(&[("env", "dev")]);
        assert!(groups_matching(groups, &orphan).is_empty());
    }
}
