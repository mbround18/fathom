//! Minimal textual edits to repository files Keel proposes changes to (spec 066 R9): adding a
//! host to a YAML inventory group, an age recipient to a `.sops.yaml` creation rule, and a job to
//! `.keel/jobs.yaml`.
//!
//! The operator's files are mostly comments, so nothing here re-serialises YAML. Each edit
//! inserts (or, for a comma-separated age list, extends) lines in place, keeping comments, key
//! order, indentation and line endings, and is then checked by parsing the result and comparing
//! it with the original plus the one intended change. An edit that would need anything cleverer
//! (flow-style mappings or lists) is refused with [`EditError::UnsupportedLayout`], and the caller
//! falls back to "apply by hand".

use std::fmt::Write as _;

use serde_yaml::Value;

use crate::inventory::{Format, HostGroups, check_text};

/// Lines of context around a change in [`Edit::diff`].
const CONTEXT: usize = 3;

/// A computed edit: the whole new text and the change as unified diff hunks (no file headers;
/// see [`Edit::unified_diff`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub new_text: String,
    /// One `@@` hunk, lines prefixed with ` `, `-` or `+`, each ending in `\n`.
    pub diff: String,
}

impl Edit {
    /// The diff with `--- a/<path>` / `+++ b/<path>` headers, as `git apply` takes it.
    #[must_use]
    pub fn unified_diff(&self, path: &str) -> String {
        format!("--- a/{path}\n+++ b/{path}\n{}", self.diff)
    }
}

/// Why an edit was refused. [`EditError::code`] is the protocol refusal code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the inventory has no group named {0:?}")]
    GroupNotFound(String),
    #[error("{host:?} is already in the inventory (groups: {})", groups.join(", "))]
    HostExists { host: String, groups: Vec<String> },
    #[error("the file's layout is not one Keel can edit safely: {0}")]
    UnsupportedLayout(String),
    #[error("{0:?} is not a valid name here")]
    InvalidName(String),
    #[error("the file has no creation rule {0}")]
    RuleNotFound(usize),
    #[error("not an age public key")]
    InvalidRecipient,
    #[error("the rule already lists this recipient")]
    RecipientExists,
    #[error("the manifest already has a job named {0:?}")]
    JobExists(String),
    #[error("the job body is not a YAML mapping: {0}")]
    InvalidJob(String),
    #[error("the file does not parse: {0}")]
    Unparseable(String),
    /// The edited text did not parse back to the original plus the change. A bug, not input.
    #[error("the edit could not be verified: {0}")]
    Unverified(String),
}

impl EditError {
    /// The refusal code from contracts/protocol.md.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::GroupNotFound(_) => "group_not_found",
            Self::HostExists { .. } => "host_exists",
            Self::UnsupportedLayout(_) | Self::Unverified(_) => "unsupported_layout",
            Self::InvalidName(_) | Self::InvalidRecipient | Self::InvalidJob(_) => "invalid",
            Self::RuleNotFound(_) => "no_sops_rule",
            Self::RecipientExists => "recipient_exists",
            Self::JobExists(_) => "job_exists",
            Self::Unparseable(_) => "unparseable",
        }
    }
}

// ---- the line model ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Blank,
    Comment,
    /// `key:` or `- key:`, with the value after the colon (comment stripped, trimmed) if any.
    Key {
        key: String,
        inline: Option<String>,
    },
    /// `- scalar`.
    Item {
        value: String,
    },
    Other,
}

#[derive(Debug, Clone)]
struct Line {
    /// Leading spaces: the column of the dash for a dash line.
    indent: usize,
    /// The column a key starts at (after `- ` for a dash line).
    col: usize,
    dash: bool,
    kind: Kind,
}

impl Line {
    fn structural(&self) -> bool {
        !matches!(self.kind, Kind::Blank | Kind::Comment)
    }

    fn key(&self) -> Option<&str> {
        match &self.kind {
            Kind::Key { key, .. } => Some(key),
            _ => None,
        }
    }

    fn inline(&self) -> Option<&str> {
        match &self.kind {
            Kind::Key { inline, .. } => inline.as_deref(),
            _ => None,
        }
    }
}

/// Splits `s` at the first `#` that starts a comment (preceded by whitespace or at the start),
/// outside quotes.
fn strip_comment(s: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev_space = true;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev_space => return &s[..i],
            None => {}
        }
        prev_space = c.is_whitespace();
    }
    s
}

/// The key of `content` (`key: value` or `key:`), outside quotes, with quotes removed.
fn split_key(content: &str) -> Option<(String, Option<String>)> {
    let content = strip_comment(content);
    let bytes = content.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b':' && bytes.get(i + 1).is_none_or(|n| *n == b' ' || *n == b'\t') => {
                let raw = content[..i].trim();
                if raw.is_empty() {
                    return None;
                }
                let key = raw.trim_matches(|c| c == '"' || c == '\'').to_owned();
                let value = content[i + 1..].trim();
                return Some((key, (!value.is_empty()).then(|| value.to_owned())));
            }
            None => {}
        }
    }
    None
}

fn parse_line(raw: &str) -> Line {
    let indent = raw.len() - raw.trim_start_matches(' ').len();
    let rest = &raw[indent..];
    if rest.trim().is_empty() {
        return Line {
            indent,
            col: indent,
            dash: false,
            kind: Kind::Blank,
        };
    }
    if rest.starts_with('#') {
        return Line {
            indent,
            col: indent,
            dash: false,
            kind: Kind::Comment,
        };
    }
    let (dash, col, content) = if rest == "-" || rest.starts_with("- ") {
        let after = &rest[1..];
        let pad = after.len() - after.trim_start_matches(' ').len();
        (true, indent + 1 + pad, after.trim_start_matches(' '))
    } else {
        (false, indent, rest)
    };
    let kind = match split_key(content) {
        Some((key, inline)) => Kind::Key { key, inline },
        None if dash => Kind::Item {
            value: strip_comment(content).trim().to_owned(),
        },
        None => Kind::Other,
    };
    Line {
        indent,
        col,
        dash,
        kind,
    }
}

/// A file as lines, with its line ending and whether it ended in one.
struct Doc {
    raw: Vec<String>,
    lines: Vec<Line>,
    eol: &'static str,
    final_eol: bool,
}

impl Doc {
    fn new(text: &str) -> Self {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let final_eol = text.is_empty() || text.ends_with('\n');
        let body = text
            .strip_suffix(eol)
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        let raw: Vec<String> = if text.is_empty() {
            Vec::new()
        } else {
            body.split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
                .collect()
        };
        let lines = raw.iter().map(|l| parse_line(l)).collect();
        Self {
            raw,
            lines,
            eol,
            final_eol,
        }
    }

    /// The index after the last line of the block that line `i` opens: everything up to the first
    /// later structural line at or left of `i`'s key column (left of it, for a dash item, whose
    /// other keys share that column). Trailing blank and comment lines are not part of the block.
    fn block_end(&self, i: usize) -> usize {
        let (col, dash) = (self.lines[i].col, self.lines[i].dash);
        let mut last = i;
        for (j, l) in self.lines.iter().enumerate().skip(i + 1) {
            if !l.structural() {
                continue;
            }
            // A dash item's sibling keys sit at its key column; a plain key's block is deeper.
            if l.indent < col || (l.indent == col && (!dash || l.dash)) {
                break;
            }
            last = j;
        }
        last + 1
    }

    /// The structural lines directly inside line `i`'s block: those at the smallest column deeper
    /// than its key's.
    fn children(&self, i: usize) -> Vec<usize> {
        let end = self.block_end(i);
        let inner: Vec<usize> = (i + 1..end)
            .filter(|&j| self.lines[j].structural() && self.lines[j].indent > self.lines[i].col)
            .collect();
        let Some(min) = inner.iter().map(|&j| self.lines[j].indent).min() else {
            return Vec::new();
        };
        inner
            .into_iter()
            .filter(|&j| self.lines[j].indent == min)
            .collect()
    }

    /// The nearest key line before `i` with a smaller key column.
    fn parent(&self, i: usize) -> Option<usize> {
        let col = self.lines[i].indent;
        (0..i)
            .rev()
            .find(|&j| self.lines[j].key().is_some() && self.lines[j].col < col)
    }

    /// The smallest indentation step in the file, or 2.
    fn step(&self) -> usize {
        let mut best: Option<usize> = None;
        for (i, l) in self.lines.iter().enumerate() {
            if !l.structural() {
                continue;
            }
            if let Some(p) = self.parent(i) {
                let d = l.indent.saturating_sub(self.lines[p].col);
                if d > 0 {
                    best = Some(best.map_or(d, |b| b.min(d)));
                }
            }
        }
        best.unwrap_or(2)
    }

    /// Replaces lines `[at, until)` with `new`, returning the new text and the hunk.
    fn splice(&self, at: usize, until: usize, new: &[String]) -> Edit {
        let mut out: Vec<String> = Vec::with_capacity(self.raw.len() + new.len());
        out.extend_from_slice(&self.raw[..at]);
        out.extend_from_slice(new);
        out.extend_from_slice(&self.raw[until..]);
        let mut new_text = out.join(self.eol);
        if self.final_eol && !out.is_empty() {
            new_text.push_str(self.eol);
        }
        Edit {
            new_text,
            diff: hunk(&self.raw, at, until, new),
        }
    }
}

/// One unified-diff hunk for replacing `old[at..until]` with `new`.
fn hunk(old: &[String], at: usize, until: usize, new: &[String]) -> String {
    let start = at.saturating_sub(CONTEXT);
    let end = (until + CONTEXT).min(old.len());
    let old_len = end - start;
    let new_len = old_len - (until - at) + new.len();
    let pos = |len: usize| if len == 0 { start } else { start + 1 };
    let mut s = format!(
        "@@ -{},{old_len} +{},{new_len} @@\n",
        pos(old_len),
        pos(new_len)
    );
    for l in &old[start..at] {
        let _ = writeln!(s, " {l}");
    }
    for l in &old[at..until] {
        let _ = writeln!(s, "-{l}");
    }
    for l in new {
        let _ = writeln!(s, "+{l}");
    }
    for l in &old[until..end] {
        let _ = writeln!(s, " {l}");
    }
    s
}

fn is_flow(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.starts_with('{') || v.starts_with('['))
}

/// An inventory host name: letters, digits, `.`, `-`, `_`; 1 to 253; not starting with `-`.
fn valid_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && !host.starts_with(['-', '.'])
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

// ---- add_host ----------------------------------------------------------------------------------

/// Adds `host` to `group` in the YAML inventory `text` (FR-020).
///
/// The host goes in the group's `hosts:` block in sorted position (before the first host that
/// sorts after it, above that host's leading comments), or after the last host. A group without
/// `hosts:` gets one as its first child.
///
/// # Errors
///
/// `host` is not a valid name or is already in the inventory, `group` is not in it, the text
/// does not parse, or the group is written in flow style.
pub fn add_host(text: &str, group: &str, host: &str) -> Result<Edit, EditError> {
    place_host(text, group, host, false)
}

/// Adds a new `host` to every group in `groups` (in order, duplicates ignored), as one edit whose
/// diff covers all the insertions.
///
/// # Errors
///
/// As [`add_host`]; `groups` empty is [`EditError::InvalidName`].
pub fn add_host_groups(text: &str, groups: &[&str], host: &str) -> Result<Edit, EditError> {
    let mut seen: Vec<&str> = Vec::new();
    for g in groups {
        if !seen.contains(g) {
            seen.push(g);
        }
    }
    let Some((first, rest)) = seen.split_first() else {
        return Err(EditError::InvalidName(String::new()));
    };
    let mut edit = place_host(text, first, host, false)?;
    for g in rest {
        edit = place_host(&edit.new_text, g, host, true)?;
    }
    if rest.is_empty() {
        return Ok(edit);
    }
    let diff = insertion_diff(&Doc::new(text).raw, &Doc::new(&edit.new_text).raw);
    Ok(Edit {
        new_text: edit.new_text,
        diff,
    })
}

/// Hunks for `new`, which is `old` with lines inserted (`old` is a subsequence of `new`).
fn insertion_diff(old: &[String], new: &[String]) -> String {
    // inserted[k]: the new lines that go before old line k (k == old.len(): at the end).
    let mut inserted: Vec<Vec<&String>> = vec![Vec::new(); old.len() + 1];
    let mut i = 0;
    for l in new {
        if i < old.len() && *l == old[i] {
            i += 1;
        } else {
            inserted[i].push(l);
        }
    }
    let points: Vec<usize> = (0..=old.len())
        .filter(|&k| !inserted[k].is_empty())
        .collect();
    let mut s = String::new();
    let mut added_before = 0;
    let mut p = 0;
    while p < points.len() {
        // Merge insertion points whose context would overlap into one hunk.
        let mut q = p;
        while q + 1 < points.len() && points[q + 1] - points[q] <= 2 * CONTEXT {
            q += 1;
        }
        let start = points[p].saturating_sub(CONTEXT);
        let end = (points[q] + CONTEXT).min(old.len());
        let adds: usize = points[p..=q].iter().map(|&k| inserted[k].len()).sum();
        let old_len = end - start;
        let old_pos = if old_len == 0 { start } else { start + 1 };
        let new_pos = start + added_before + 1;
        let _ = writeln!(
            s,
            "@@ -{old_pos},{old_len} +{new_pos},{} @@",
            old_len + adds
        );
        for (k, before) in inserted.iter().enumerate().take(end + 1).skip(start) {
            for l in before {
                let _ = writeln!(s, "+{l}");
            }
            if k < end {
                let _ = writeln!(s, " {}", old[k]);
            }
        }
        added_before += adds;
        p = q + 1;
    }
    s
}

/// [`add_host`], or (`existing`) adding a host already in the inventory to one more group.
fn place_host(text: &str, group: &str, host: &str, existing: bool) -> Result<Edit, EditError> {
    if !valid_host(host) {
        return Err(EditError::InvalidName(host.to_owned()));
    }
    let before = check_text(text, Format::Yaml, None);
    if !before.is_valid() {
        return Err(EditError::Unparseable(
            before
                .errors
                .first()
                .map_or_else(String::new, |e| e.message.clone()),
        ));
    }
    if let Some(hg) = before
        .hosts
        .iter()
        .find(|h| h.host == host && (!existing || h.groups.iter().any(|g| g == group)))
    {
        return Err(EditError::HostExists {
            host: host.to_owned(),
            groups: hg.groups.clone(),
        });
    }
    let doc = Doc::new(text);
    let g = find_group(&doc, group).ok_or_else(|| EditError::GroupNotFound(group.to_owned()))?;
    if is_flow(doc.lines[g].inline()) {
        return Err(EditError::UnsupportedLayout(format!(
            "group {group:?} is a flow mapping"
        )));
    }
    let step = doc.step();
    let children = doc.children(g);
    let hosts_line = children
        .iter()
        .copied()
        .find(|&j| doc.lines[j].key() == Some("hosts"));
    let edit = match hosts_line {
        Some(h) => {
            if is_flow(doc.lines[h].inline()) {
                return Err(EditError::UnsupportedLayout(format!(
                    "the hosts of {group:?} are a flow mapping"
                )));
            }
            let existing = doc.children(h);
            let col = existing
                .first()
                .map_or(doc.lines[h].col + step, |&j| doc.lines[j].indent);
            let line = format!("{}{host}:", " ".repeat(col));
            // Sorted position only when the hosts are sorted; otherwise the end, so an
            // operator's own order is not second-guessed.
            let names: Vec<&str> = existing
                .iter()
                .filter_map(|&j| doc.lines[j].key())
                .collect();
            let sorted = names.windows(2).all(|w| w[0] <= w[1]);
            let at = match existing
                .iter()
                .copied()
                .filter(|_| sorted)
                .find(|&j| doc.lines[j].key().is_some_and(|k| k > host))
            {
                Some(next) => leading_comments(&doc, next),
                None => existing.last().map_or(h + 1, |&j| doc.block_end(j)),
            };
            doc.splice(at, at, &[line])
        }
        None => {
            let col = children
                .first()
                .map_or(doc.lines[g].col + step, |&j| doc.lines[j].indent);
            let lines = [
                format!("{}hosts:", " ".repeat(col)),
                format!("{}{host}:", " ".repeat(col + step)),
            ];
            doc.splice(g + 1, g + 1, &lines)
        }
    };
    verify_host(&before.hosts, &edit.new_text, group, host)?;
    Ok(edit)
}

/// The first line of the comment block directly above line `i` at its indentation or deeper.
fn leading_comments(doc: &Doc, i: usize) -> usize {
    let col = doc.lines[i].indent;
    let mut at = i;
    while at > 0 {
        let l = &doc.lines[at - 1];
        if l.kind == Kind::Comment && l.indent >= col {
            at -= 1;
        } else {
            break;
        }
    }
    at
}

/// The line defining `group`: a key under some `children:`, or a top-level key. A group listed
/// under several parents is defined where it has `hosts:`, else at its first appearance.
fn find_group(doc: &Doc, group: &str) -> Option<usize> {
    let candidates: Vec<usize> = doc
        .lines
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            l.key() == Some(group)
                && !l.dash
                && match doc.parent(*i) {
                    None => l.indent == 0,
                    Some(p) => doc.lines[p].key() == Some("children"),
                }
        })
        .map(|(i, _)| i)
        .collect();
    candidates
        .iter()
        .copied()
        .find(|&i| {
            doc.children(i)
                .iter()
                .any(|&j| doc.lines[j].key() == Some("hosts"))
        })
        .or_else(|| candidates.first().copied())
}

fn verify_host(
    before: &[HostGroups],
    new_text: &str,
    group: &str,
    host: &str,
) -> Result<(), EditError> {
    let after = check_text(new_text, Format::Yaml, None);
    let (added, rest): (Vec<&HostGroups>, Vec<&HostGroups>) =
        after.hosts.iter().partition(|h| h.host == host);
    let before: Vec<&HostGroups> = before.iter().filter(|h| h.host != host).collect();
    let same = rest.len() == before.len() && rest.iter().zip(&before).all(|(a, b)| a == b);
    match added.as_slice() {
        [h] if same && h.groups.iter().any(|g| g == group) => Ok(()),
        _ => Err(EditError::Unverified(format!(
            "after the edit, {host:?} is not exactly the one new host in {group:?}"
        ))),
    }
}

// ---- add_sops_recipient ------------------------------------------------------------------------

/// Whether `s` is an age X25519 public key: `age1` and 58 bech32 characters.
#[must_use]
pub fn is_age_recipient(s: &str) -> bool {
    s.len() == 62
        && s.starts_with("age1")
        && s[4..]
            .chars()
            .all(|c| "qpzry9x8gf2tvdw0s3jn54khce6mua7l".contains(c))
}

/// The age keys rule `index` of a parsed `.sops.yaml` names, in its own `age` and in its key
/// groups, as written (a comma-separated string or a list).
fn rule_age_keys(doc: &Value, index: usize) -> Option<Vec<String>> {
    let rule = doc.get("creation_rules")?.as_sequence()?.get(index)?;
    let mut keys = Vec::new();
    let mut take = |v: Option<&Value>| match v {
        Some(Value::String(s)) => keys.extend(
            s.split(',')
                .map(|k| k.trim().to_owned())
                .filter(|k| !k.is_empty()),
        ),
        Some(Value::Sequence(items)) => {
            keys.extend(items.iter().filter_map(Value::as_str).map(str::to_owned))
        }
        _ => {}
    };
    take(rule.get("age"));
    if let Some(groups) = rule.get("key_groups").and_then(Value::as_sequence) {
        for g in groups {
            take(g.get("age"));
        }
    }
    Some(keys)
}

/// Adds the age `recipient` to creation rule `rule_index` of `.sops.yaml` `text` (FR-022): to the
/// rule's own `age` when it has one, else to its first key group's `age`. Re-encrypting the
/// files the rule covers is the operator's step; this changes only the rules.
///
/// # Errors
///
/// The recipient is not an age key or is already listed, the rule does not exist or has no age
/// list, the text does not parse, or the list is in flow style.
pub fn add_sops_recipient(
    text: &str,
    rule_index: usize,
    recipient: &str,
) -> Result<Edit, EditError> {
    if !is_age_recipient(recipient) {
        return Err(EditError::InvalidRecipient);
    }
    let parsed: Value =
        serde_yaml::from_str(text).map_err(|e| EditError::Unparseable(e.to_string()))?;
    let before = rule_age_keys(&parsed, rule_index).ok_or(EditError::RuleNotFound(rule_index))?;
    if before.iter().any(|k| k == recipient) {
        return Err(EditError::RecipientExists);
    }
    let doc = Doc::new(text);
    let rules = doc
        .lines
        .iter()
        .position(|l| l.indent == 0 && l.key() == Some("creation_rules"))
        .ok_or(EditError::RuleNotFound(rule_index))?;
    let items: Vec<usize> = doc
        .children(rules)
        .into_iter()
        .filter(|&j| doc.lines[j].dash)
        .collect();
    let item = *items
        .get(rule_index)
        .ok_or(EditError::RuleNotFound(rule_index))?;
    let age = find_age(&doc, item).ok_or_else(|| {
        EditError::UnsupportedLayout("the rule has no age list to add to".to_owned())
    })?;
    let edit = match doc.lines[age].inline() {
        Some(v) if is_flow(Some(v)) => {
            return Err(EditError::UnsupportedLayout(
                "the age list is in flow style".to_owned(),
            ));
        }
        Some(_) => extend_inline(&doc, age, recipient)?,
        None => {
            let entries: Vec<usize> = doc
                .children(age)
                .into_iter()
                .filter(|&j| matches!(doc.lines[j].kind, Kind::Item { .. }))
                .collect();
            let indent = entries
                .first()
                .map_or(doc.lines[age].col + doc.step(), |&j| doc.lines[j].indent);
            let at = entries.last().map_or(age + 1, |&j| j + 1);
            doc.splice(at, at, &[format!("{}- {recipient}", " ".repeat(indent))])
        }
    };
    let after: Value =
        serde_yaml::from_str(&edit.new_text).map_err(|e| EditError::Unverified(e.to_string()))?;
    let mut want = before;
    want.push(recipient.to_owned());
    let mut got = rule_age_keys(&after, rule_index).unwrap_or_default();
    got.sort();
    want.sort();
    if got != want {
        return Err(EditError::Unverified(
            "the rule's age keys are not the old ones plus the new one".to_owned(),
        ));
    }
    Ok(edit)
}

/// The `age` key line of the rule item at `item`: its own, else the first key group's.
fn find_age(doc: &Doc, item: usize) -> Option<usize> {
    let end = doc.block_end(item);
    let col = doc.lines[item].col;
    let mine = (item..end).find(|&j| {
        doc.lines[j].col == col
            && doc.lines[j].key() == Some("age")
            && (j == item || !doc.lines[j].dash)
    });
    if mine.is_some() {
        return mine;
    }
    let groups = (item..end)
        .find(|&j| doc.lines[j].col == col && doc.lines[j].key() == Some("key_groups"))?;
    let first = *doc.children(groups).iter().find(|&&j| doc.lines[j].dash)?;
    let gend = doc.block_end(first);
    let gcol = doc.lines[first].col;
    (first..gend).find(|&j| doc.lines[j].col == gcol && doc.lines[j].key() == Some("age"))
}

/// Appends `,recipient` to an inline `age: a,b` value, inside its quotes if quoted.
fn extend_inline(doc: &Doc, at: usize, recipient: &str) -> Result<Edit, EditError> {
    let raw = &doc.raw[at];
    let body = strip_comment(raw).trim_end();
    let comment = &raw[body.len()..];
    let new = match body.chars().last() {
        Some(q @ ('"' | '\'')) => format!("{},{recipient}{q}{comment}", &body[..body.len() - 1]),
        Some(_) => format!("{body},{recipient}{comment}"),
        None => return Err(EditError::UnsupportedLayout("empty age value".to_owned())),
    };
    Ok(doc.splice(at, at + 1, &[new]))
}

// ---- add_job -----------------------------------------------------------------------------------

/// A manifest job name: lowercase letters, digits, `-` and `_`, starting with a letter or digit,
/// at most 63.
fn valid_job_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// Appends job `name` with body `job_yaml` (an unindented YAML mapping, as `DraftJob` makes it)
/// to the `jobs:` of manifest `text` (FR-024), after a blank line. Adds `jobs:` when absent.
///
/// # Errors
///
/// The name is invalid or taken, the body is not a mapping, the text does not parse, or `jobs`
/// is in flow style.
pub fn add_job(text: &str, name: &str, job_yaml: &str) -> Result<Edit, EditError> {
    if !valid_job_name(name) {
        return Err(EditError::InvalidName(name.to_owned()));
    }
    let body: Value =
        serde_yaml::from_str(job_yaml).map_err(|e| EditError::InvalidJob(e.to_string()))?;
    if !body.is_mapping() {
        return Err(EditError::InvalidJob("expected a mapping".to_owned()));
    }
    let before: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_yaml::from_str(text).map_err(|e| EditError::Unparseable(e.to_string()))?
    };
    if before.get("jobs").and_then(|j| j.get(name)).is_some() {
        return Err(EditError::JobExists(name.to_owned()));
    }
    let doc = Doc::new(text);
    let step = doc.step();
    let jobs = doc
        .lines
        .iter()
        .position(|l| l.indent == 0 && l.key() == Some("jobs"));
    let edit = match jobs {
        Some(j) => {
            if is_flow(doc.lines[j].inline()) {
                return Err(EditError::UnsupportedLayout(
                    "jobs is a flow mapping".to_owned(),
                ));
            }
            let existing = doc.children(j);
            let col = existing.first().map_or(step, |&k| doc.lines[k].indent);
            let at = doc.block_end(j);
            let mut lines = Vec::new();
            if !existing.is_empty() {
                lines.push(String::new());
            }
            lines.extend(job_lines(name, job_yaml, col, step));
            doc.splice(at, at, &lines)
        }
        None => {
            let at = doc.raw.len();
            let mut lines = Vec::new();
            if doc.raw.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("jobs:".to_owned());
            lines.extend(job_lines(name, job_yaml, step, step));
            doc.splice(at, at, &lines)
        }
    };
    let after: Value =
        serde_yaml::from_str(&edit.new_text).map_err(|e| EditError::Unverified(e.to_string()))?;
    let mut want = match before {
        Value::Mapping(m) => m,
        _ => serde_yaml::Mapping::new(),
    };
    let jobs_entry = want
        .entry(Value::from("jobs"))
        .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
    if jobs_entry.is_null() {
        *jobs_entry = Value::Mapping(serde_yaml::Mapping::new());
    }
    if let Value::Mapping(m) = jobs_entry {
        m.insert(Value::from(name), body);
    }
    if after != Value::Mapping(want) {
        return Err(EditError::Unverified(
            "the manifest is not the old one plus the job".to_owned(),
        ));
    }
    Ok(edit)
}

fn job_lines(name: &str, job_yaml: &str, col: usize, step: usize) -> Vec<String> {
    let mut lines = vec![format!("{}{name}:", " ".repeat(col))];
    let pad = " ".repeat(col + step);
    for l in job_yaml.trim_end().lines() {
        let l = l.strip_suffix('\r').unwrap_or(l);
        lines.push(if l.trim().is_empty() {
            String::new()
        } else {
            format!("{pad}{l}")
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the operator's `inventory/hosts.yml`: comments everywhere, a child group, a
    /// group with only a comment before its host.
    const INVENTORY: &str = "\
# The fleet.
all:
  children:
    # Boxes that run one compose stack.
    compose_hosts:
      hosts:
        audiobookshelf:
        keycloak:
        forgejo:
        pulse:
        rustfs:
      # Game server VMs.
      children:
        gameservers:
          hosts:
            steam:

    pve_hosts:
      hosts:
        pve:

    k3s_servers:
      hosts:
        # roanoke's control plane.
        ns5004891:
    keel_nodes:
    keel_servers:
      hosts:
        keel:
";

    fn added(edit: &Edit) -> Vec<&str> {
        edit.diff.lines().filter(|l| l.starts_with('+')).collect()
    }

    fn removed(edit: &Edit) -> usize {
        edit.diff.lines().filter(|l| l.starts_with('-')).count()
    }

    #[test]
    fn unsorted_hosts_get_the_new_one_last_and_nothing_else_changes() {
        let e = add_host(INVENTORY, "compose_hosts", "jellyfin").unwrap();
        assert_eq!(added(&e), ["+        jellyfin:"]);
        assert_eq!(removed(&e), 0);
        let want = INVENTORY.replace("        rustfs:\n", "        rustfs:\n        jellyfin:\n");
        assert_eq!(e.new_text, want);
    }

    #[test]
    fn sorted_hosts_get_sorted_position() {
        let sorted = INVENTORY.replace(
            "        keycloak:\n        forgejo:\n",
            "        forgejo:\n        keycloak:\n",
        );
        let e = add_host(&sorted, "compose_hosts", "jellyfin").unwrap();
        let want = sorted.replace(
            "        forgejo:\n",
            "        forgejo:\n        jellyfin:\n",
        );
        assert_eq!(e.new_text, want);
    }

    #[test]
    fn appends_after_the_last_host_not_into_the_next_comment() {
        let e = add_host(INVENTORY, "compose_hosts", "zulip").unwrap();
        let want = INVENTORY.replace("        rustfs:\n", "        rustfs:\n        zulip:\n");
        assert_eq!(e.new_text, want);
    }

    #[test]
    fn child_group() {
        let e = add_host(INVENTORY, "gameservers", "minecraft").unwrap();
        let want = INVENTORY.replace(
            "            steam:\n",
            "            minecraft:\n            steam:\n",
        );
        assert_eq!(e.new_text, want);
    }

    #[test]
    fn goes_above_the_comment_that_belongs_to_the_next_host() {
        let e = add_host(INVENTORY, "k3s_servers", "alpha").unwrap();
        let want = INVENTORY.replace(
            "        # roanoke's control plane.\n",
            "        alpha:\n        # roanoke's control plane.\n",
        );
        assert_eq!(e.new_text, want);
    }

    #[test]
    fn group_without_hosts_gets_a_block() {
        let e = add_host(INVENTORY, "keel_nodes", "steam2").unwrap();
        let want = INVENTORY.replace(
            "    keel_nodes:\n",
            "    keel_nodes:\n      hosts:\n        steam2:\n",
        );
        assert_eq!(e.new_text, want);
        assert_eq!(added(&e).len(), 2);
    }

    #[test]
    fn existing_host_and_missing_group_are_refused() {
        match add_host(INVENTORY, "pve_hosts", "steam") {
            Err(EditError::HostExists { groups, .. }) => {
                assert!(groups.contains(&"gameservers".to_owned()))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            add_host(INVENTORY, "nope", "x"),
            Err(EditError::GroupNotFound("nope".into()))
        );
        assert!(matches!(
            add_host(INVENTORY, "pve_hosts", "bad name"),
            Err(EditError::InvalidName(_))
        ));
    }

    #[test]
    fn one_host_into_several_groups_is_one_edit() {
        let e = add_host_groups(
            INVENTORY,
            &["pve_hosts", "keel_servers", "pve_hosts"],
            "box",
        )
        .unwrap();
        let want = INVENTORY
            .replace("        pve:\n", "        box:\n        pve:\n")
            .replace("        keel:\n", "        box:\n        keel:\n");
        assert_eq!(e.new_text, want);
        assert_eq!(added(&e), ["+        box:", "+        box:"]);
        assert_eq!(removed(&e), 0);
        // Two hunks; the second's new start counts the first insertion.
        assert!(e.diff.starts_with("@@ -17,6 +17,7 @@\n"), "{}", e.diff);
        assert!(e.diff.contains("@@ -26,4 +27,5 @@\n"), "{}", e.diff);
        assert!(matches!(
            add_host_groups(INVENTORY, &["pve_hosts", "compose_hosts"], "steam"),
            Err(EditError::HostExists { .. })
        ));
        assert!(matches!(
            add_host_groups(INVENTORY, &[], "x"),
            Err(EditError::InvalidName(_))
        ));
    }

    #[test]
    fn crlf_and_missing_final_newline_are_kept() {
        let crlf = INVENTORY.replace('\n', "\r\n");
        let e = add_host(&crlf, "pve_hosts", "pve2").unwrap();
        assert_eq!(
            e.new_text,
            crlf.replace("        pve:\r\n", "        pve:\r\n        pve2:\r\n")
        );
        let bare = INVENTORY.trim_end();
        let e = add_host(bare, "keel_servers", "zz").unwrap();
        assert_eq!(e.new_text, format!("{bare}\n        zz:"));
    }

    #[test]
    fn flow_style_is_refused() {
        let text = "all:\n  children:\n    web:\n      hosts: {a: , b: }\n";
        assert!(matches!(
            add_host(text, "web", "c"),
            Err(EditError::UnsupportedLayout(_))
        ));
    }

    #[test]
    fn host_with_vars_is_skipped_whole() {
        let text = "all:\n  children:\n    web:\n      hosts:\n        a:\n          port: 1\n        # c's note\n        c:\n";
        let e = add_host(text, "web", "b").unwrap();
        assert_eq!(
            e.new_text,
            "all:\n  children:\n    web:\n      hosts:\n        a:\n          port: 1\n        b:\n        # c's note\n        c:\n"
        );
        let e = add_host(text, "web", "z").unwrap();
        assert!(e.new_text.ends_with("        c:\n        z:\n"));
    }

    #[test]
    fn diff_is_a_valid_hunk() {
        let e = add_host(INVENTORY, "pve_hosts", "pve2").unwrap();
        assert!(e.diff.starts_with("@@ -18,6 +18,7 @@\n"), "{}", e.diff);
        let u = e.unified_diff("inventory/hosts.yml");
        assert!(u.starts_with("--- a/inventory/hosts.yml\n+++ b/inventory/hosts.yml\n@@"));
    }

    const SOPS: &str = "\
# Who can decrypt secrets.
creation_rules:
  # Keel's server.
  - path_regex: ^secrets/keel\\.sops\\.ya?ml$
    key_groups:
      - age:
          - age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq
          - age1pppppppppppppppppppppppppppppppppppppppppppppppppppppppppp
  - path_regex: ^secrets/.*\\.sops\\.ya?ml$
    age: age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq # workstation
";

    const NEW: &str = "age1zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";

    #[test]
    fn recipient_into_a_key_group_list() {
        let e = add_sops_recipient(SOPS, 0, NEW).unwrap();
        assert_eq!(added(&e), [format!("+          - {NEW}").as_str()]);
        assert_eq!(removed(&e), 0);
    }

    #[test]
    fn recipient_into_a_comma_string_keeps_the_comment() {
        let e = add_sops_recipient(SOPS, 1, NEW).unwrap();
        assert_eq!(removed(&e), 1);
        assert!(
            e.new_text.contains(&format!("qq,{NEW} # workstation\n")),
            "{}",
            e.new_text
        );
        let quoted = SOPS
            .replace("age: age1q", "age: \"age1q")
            .replace("qq # workstation", "qq\" # workstation");
        let e = add_sops_recipient(&quoted, 1, NEW).unwrap();
        assert!(
            e.new_text.contains(&format!(",{NEW}\" # workstation")),
            "{}",
            e.new_text
        );
    }

    #[test]
    fn recipient_refusals() {
        assert_eq!(
            add_sops_recipient(SOPS, 0, "nope"),
            Err(EditError::InvalidRecipient)
        );
        assert_eq!(
            add_sops_recipient(SOPS, 2, NEW),
            Err(EditError::RuleNotFound(2))
        );
        let existing = "age1pppppppppppppppppppppppppppppppppppppppppppppppppppppppppp";
        assert_eq!(
            add_sops_recipient(SOPS, 0, existing),
            Err(EditError::RecipientExists)
        );
        let flow = "creation_rules:\n  - age: [age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq]\n";
        assert!(matches!(
            add_sops_recipient(flow, 0, NEW),
            Err(EditError::UnsupportedLayout(_))
        ));
    }

    const JOBS: &str = "\
version: 1
inventories:
  fleet:
    path: inventory/hosts.yml
jobs:
  debian:
    playbook: playbooks/debian.yaml
    profiles: [debian]
    inventory: fleet

# Trailing note.
";

    #[test]
    fn job_is_appended_inside_jobs() {
        let draft = "playbook: playbooks/x.yaml\nprofiles: []\npaths:\n  - playbooks/x.yaml\n";
        let e = add_job(JOBS, "x", draft).unwrap();
        assert_eq!(removed(&e), 0);
        assert!(e.new_text.contains(
            "    inventory: fleet\n\n  x:\n    playbook: playbooks/x.yaml\n    profiles: []\n    paths:\n      - playbooks/x.yaml\n\n# Trailing note.\n"
        ), "{}", e.new_text);
        assert_eq!(
            add_job(JOBS, "debian", draft),
            Err(EditError::JobExists("debian".into()))
        );
        assert!(matches!(
            add_job(JOBS, "Bad", draft),
            Err(EditError::InvalidName(_))
        ));
        assert!(matches!(
            add_job(JOBS, "y", "- a\n"),
            Err(EditError::InvalidJob(_))
        ));
    }

    #[test]
    fn job_without_jobs_key() {
        let e = add_job("version: 1\n", "x", "playbook: a.yml\n").unwrap();
        assert_eq!(
            e.new_text,
            "version: 1\n\njobs:\n  x:\n    playbook: a.yml\n"
        );
    }

    #[test]
    fn age_recipient_shape() {
        assert!(is_age_recipient(NEW));
        assert!(!is_age_recipient("age1b"));
        assert!(!is_age_recipient(&NEW.replace('z', "b")));
    }
}
