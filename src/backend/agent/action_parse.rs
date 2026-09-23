//! Parses the ```fathom-action``` fenced-block convention out of a provider's raw
//! reply text (Phase 3, informational only — no execution here, see module doc on
//! `src/backend/agent/mod.rs`).
//!
//! Convention: the model may emit zero or more fenced code blocks tagged
//! `fathom-action` anywhere in its reply, each containing a single JSON object:
//! `{"action_kind": "...", "target": {"kind":"...","namespace":"...","name":"..."},
//! "params": {...}, "rationale": "..."}`. A malformed or incomplete block is skipped
//! (logged via `tracing::warn!`) rather than failing the whole turn — a parse failure
//! must never break the informational text reply (Principle I).

use serde::Deserialize;

use crate::protocol::v1::AgentObjectRef;

const FENCE_TAG: &str = "```fathom-action";
const FENCE_CLOSE: &str = "```";

/// A `SuggestedAction` proposal parsed out of raw model text, before an id/status have
/// been assigned (that happens in `src/backend/cmd/agent.rs`, which owns id generation
/// via `next_local_id`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedAction {
    pub action_kind: String,
    pub target: AgentObjectRef,
    pub params: serde_json::Value,
    pub rationale: String,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    action_kind: Option<String>,
    target: Option<RawTarget>,
    #[serde(default)]
    params: serde_json::Value,
    rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTarget {
    kind: Option<String>,
    namespace: Option<String>,
    name: Option<String>,
}

/// Scans `text` for ```fathom-action fenced blocks and parses each into a
/// `ParsedAction`. Blocks that are malformed JSON or missing a required non-empty
/// field are silently skipped (warn-logged), never surfaced as an error.
pub(crate) fn parse_suggested_actions(text: &str) -> Vec<ParsedAction> {
    let mut actions = Vec::new();
    for block in extract_fenced_blocks(text) {
        match parse_one_block(&block) {
            Some(action) => actions.push(action),
            None => tracing::warn!("Skipping malformed fathom-action block: {block}"),
        }
    }
    actions
}

/// Removes every ```fathom-action fenced block from `text`, leaving clean prose for
/// the operator. Pure text transformation, deliberately separate from parsing so a
/// parse failure never prevents stripping (or vice versa).
pub(crate) fn strip_action_blocks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(FENCE_TAG) {
        result.push_str(&rest[..start]);
        let after_tag = &rest[start + FENCE_TAG.len()..];
        match after_tag.find(FENCE_CLOSE) {
            Some(close) => {
                rest = &after_tag[close + FENCE_CLOSE.len()..];
            }
            None => {
                // Unterminated block: drop the rest of the text rather than leave a
                // dangling fence marker visible.
                rest = "";
                break;
            }
        }
    }
    result.push_str(rest);
    // Collapse runs of blank lines left behind by a removed block.
    let collapsed: Vec<&str> = result.lines().collect();
    let mut out = String::new();
    let mut prev_blank = false;
    for line in collapsed {
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = blank;
    }
    out.trim().to_owned()
}

fn extract_fenced_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(FENCE_TAG) {
        let after_tag = &rest[start + FENCE_TAG.len()..];
        // Skip an optional trailing newline right after the opening fence tag.
        let body_start = after_tag.strip_prefix('\n').unwrap_or(after_tag);
        match body_start.find(FENCE_CLOSE) {
            Some(close) => {
                blocks.push(body_start[..close].to_owned());
                rest = &body_start[close + FENCE_CLOSE.len()..];
            }
            None => break,
        }
    }
    blocks
}

fn parse_one_block(block: &str) -> Option<ParsedAction> {
    let raw: RawAction = match serde_json::from_str(block.trim()) {
        Ok(raw) => raw,
        Err(err) => {
            tracing::warn!("fathom-action block is not valid JSON: {err}");
            return None;
        }
    };

    let action_kind = non_empty(raw.action_kind)?;
    let rationale = non_empty(raw.rationale)?;
    let target = raw.target?;
    let kind = non_empty(target.kind)?;
    let namespace = non_empty(target.namespace)?;
    let name = non_empty(target.name)?;

    Some(ParsedAction {
        action_kind,
        target: AgentObjectRef {
            kind,
            namespace,
            name,
        },
        params: raw.params,
        rationale,
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_valid_block() {
        let text = "Here's what I'd do:\n```fathom-action\n{\"action_kind\":\"restart\",\"target\":{\"kind\":\"Pod\",\"namespace\":\"default\",\"name\":\"api-0\"},\"params\":{},\"rationale\":\"OOMKilled\"}\n```\nDone.";
        let actions = parse_suggested_actions(text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_kind, "restart");
        assert_eq!(actions[0].target.name, "api-0");
        assert_eq!(actions[0].rationale, "OOMKilled");
    }

    #[test]
    fn parses_multiple_blocks() {
        let text = "```fathom-action\n{\"action_kind\":\"restart\",\"target\":{\"kind\":\"Pod\",\"namespace\":\"default\",\"name\":\"api-0\"},\"params\":{},\"rationale\":\"a\"}\n```\nsome text\n```fathom-action\n{\"action_kind\":\"scale\",\"target\":{\"kind\":\"Deployment\",\"namespace\":\"default\",\"name\":\"api\"},\"params\":{\"replicas\":3},\"rationale\":\"b\"}\n```";
        let actions = parse_suggested_actions(text);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_kind, "restart");
        assert_eq!(actions[1].action_kind, "scale");
        assert_eq!(actions[1].params, serde_json::json!({"replicas": 3}));
    }

    #[test]
    fn skips_malformed_json_block_without_erroring() {
        let text = "```fathom-action\n{not valid json\n```\ntext after";
        let actions = parse_suggested_actions(text);
        assert!(actions.is_empty());
    }

    #[test]
    fn skips_block_missing_required_field() {
        let text = "```fathom-action\n{\"action_kind\":\"restart\",\"target\":{\"kind\":\"Pod\",\"namespace\":\"default\",\"name\":\"api-0\"},\"params\":{}}\n```";
        let actions = parse_suggested_actions(text);
        assert!(actions.is_empty(), "missing rationale must be skipped");
    }

    #[test]
    fn skips_block_with_empty_action_kind() {
        let text = "```fathom-action\n{\"action_kind\":\"\",\"target\":{\"kind\":\"Pod\",\"namespace\":\"default\",\"name\":\"api-0\"},\"params\":{},\"rationale\":\"x\"}\n```";
        let actions = parse_suggested_actions(text);
        assert!(actions.is_empty());
    }

    #[test]
    fn zero_blocks_yields_empty_vec_and_unchanged_text() {
        let text = "Just a plain informational answer with no proposals.";
        assert!(parse_suggested_actions(text).is_empty());
        assert_eq!(strip_action_blocks(text), text);
    }

    #[test]
    fn strips_blocks_leaving_clean_prose() {
        let text = "Before.\n\n```fathom-action\n{\"action_kind\":\"restart\",\"target\":{\"kind\":\"Pod\",\"namespace\":\"default\",\"name\":\"api-0\"},\"params\":{},\"rationale\":\"x\"}\n```\n\nAfter.";
        let stripped = strip_action_blocks(text);
        assert_eq!(stripped, "Before.\n\nAfter.");
        assert!(!stripped.contains("fathom-action"));
    }

    #[test]
    fn strips_multiple_blocks() {
        let text = "A\n```fathom-action\n{}\n```\nB\n```fathom-action\n{}\n```\nC";
        let stripped = strip_action_blocks(text);
        assert_eq!(stripped, "A\n\nB\n\nC");
    }
}
