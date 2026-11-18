//! Spec 067 T064: property tests over `queries.toml` and the runner's query checks.
//!
//! - Parsing arbitrary text never panics, and a file that parses only allows valid units and
//!   absolute, normalised paths.
//! - Whatever parameter a hostile Keel or relay sends, `check` only lets through a unit name of
//!   the safe alphabet that the file lists, or a normalised path at or below a listed path:
//!   never a `..`, an empty component, a NUL or a relative path.
//! - Without a trust file every query is refused.

use std::collections::BTreeMap;
use std::path::Path;

use keel_exec::frames::QueryRefusal;
use keel_runner::runner::queries::{self, Checked, QueryTrust};
use proptest::prelude::*;

const KINDS: [&str; 5] = [
    "facts",
    "applied_revision",
    "unit_state",
    "disk_health",
    "path_free",
];

fn trust() -> QueryTrust {
    QueryTrust::parse(
        r#"
kinds = ["facts", "applied_revision", "unit_state", "disk_health", "path_free"]
units = ["nginx.service", "postgresql@*"]
paths = ["/srv/data", "/var/log"]
"#,
    )
    .expect("trust")
}

/// Path-ish strings: absolute and relative, with `.`, `..`, empty components, NULs and the
/// allowed prefixes mixed in.
fn path_like() -> impl Strategy<Value = String> {
    let part = prop_oneof![
        Just("srv".to_owned()),
        Just("data".to_owned()),
        Just("var".to_owned()),
        Just("log".to_owned()),
        Just("etc".to_owned()),
        Just("shadow".to_owned()),
        Just("..".to_owned()),
        Just(".".to_owned()),
        Just(String::new()),
        Just("a\0b".to_owned()),
        "[a-z0-9._-]{1,8}",
    ];
    (any::<bool>(), prop::collection::vec(part, 0..6), any::<bool>()).prop_map(
        |(absolute, parts, trailing)| {
            let mut s = if absolute { "/".to_owned() } else { String::new() };
            s.push_str(&parts.join("/"));
            if trailing {
                s.push('/');
            }
            s
        },
    )
}

fn unit_like() -> impl Strategy<Value = String> {
    prop_oneof![
        ".{0,40}",
        "[A-Za-z0-9@._:-]{0,40}",
        "(nginx\\.service|postgresql@)[ -~]{0,12}",
        Just("-H".to_owned()),
        Just("--host=evil".to_owned()),
    ]
}

fn one(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

/// What `path` may be once `check` accepts it: normalised and at or below an allowed path.
fn path_is_safe(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\0')
        && path[1..]
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
        && (Path::new(path).starts_with("/srv/data") || Path::new(path).starts_with("/var/log"))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn parsing_any_text_never_panics(text in ".{0,200}") {
        let _ = QueryTrust::parse(&text);
    }

    #[test]
    fn a_parsed_file_only_holds_valid_patterns(
        units in prop::collection::vec(unit_like(), 0..4),
        paths in prop::collection::vec(path_like(), 0..4),
    ) {
        let text = format!(
            "kinds = [\"unit_state\", \"path_free\"]\nunits = {}\npaths = {}\n",
            toml_list(&units),
            toml_list(&paths),
        );
        if let Ok(t) = QueryTrust::parse(&text) {
            for u in &units {
                let literal = u.strip_suffix('*').unwrap_or(u);
                prop_assert!(queries::valid_unit(literal), "{u:?} parsed");
            }
            for p in &paths {
                prop_assert!(queries::valid_path(p), "{p:?} parsed");
            }
            // Anything it allows is itself a valid name or path.
            for u in &units {
                if t.allows_unit(u) {
                    prop_assert!(queries::valid_unit(u));
                }
            }
        }
    }

    #[test]
    fn a_path_parameter_never_escapes_the_listed_paths(path in path_like()) {
        match queries::check(Some(&trust()), "path_free", &one("path", &path)) {
            Ok(Checked::PathFree(p)) => {
                prop_assert_eq!(&p, &path);
                prop_assert!(path_is_safe(&p), "{p:?} was allowed");
            }
            Ok(other) => prop_assert!(false, "{other:?}"),
            Err(r) => prop_assert!(matches!(r, QueryRefusal::BadParams | QueryRefusal::NotAllowed)),
        }
    }

    #[test]
    fn a_unit_parameter_is_listed_and_of_the_safe_alphabet(unit in unit_like()) {
        match queries::check(Some(&trust()), "unit_state", &one("unit", &unit)) {
            Ok(Checked::UnitState(u)) => {
                prop_assert!(queries::valid_unit(&u));
                prop_assert!(u == "nginx.service" || u.starts_with("postgresql@"), "{u:?}");
            }
            Ok(other) => prop_assert!(false, "{other:?}"),
            Err(r) => prop_assert!(matches!(r, QueryRefusal::BadParams | QueryRefusal::NotAllowed)),
        }
    }

    #[test]
    fn without_a_trust_file_everything_is_refused(
        kind in prop_oneof![prop::sample::select(KINDS.to_vec()).prop_map(str::to_owned), ".{0,16}"],
        params in prop::collection::btree_map(".{0,8}", ".{0,16}", 0..3),
    ) {
        prop_assert_eq!(queries::check(None, &kind, &params), Err(QueryRefusal::NotAllowed));
    }

    #[test]
    fn unknown_kinds_and_extra_parameters_are_refused(
        kind in "[a-z_]{0,20}",
        extra in "[a-z]{1,8}",
    ) {
        let r = queries::check(Some(&trust()), &kind, &BTreeMap::new());
        if !KINDS.contains(&kind.as_str()) {
            prop_assert_eq!(r, Err(QueryRefusal::NotAllowed));
        }
        if extra != "unit" && extra != "path" {
            for k in KINDS {
                prop_assert!(queries::check(Some(&trust()), k, &one(&extra, "x")).is_err());
            }
        }
    }
}

/// `items` as a TOML array of basic strings.
fn toml_list(items: &[String]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|s| toml::Value::String(s.clone()).to_string())
        .collect();
    format!("[{}]", quoted.join(", "))
}
