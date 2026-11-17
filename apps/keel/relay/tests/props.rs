//! Spec 067 T064: property tests over the relay's two input files, `relay.toml` and `hub_keys`.
//!
//! Arbitrary text never panics either parser, and anything accepted passes an independent check
//! (a plain TOML table with only the known keys; only `ssh-ed25519` lines). Structured inputs,
//! built from valid and invalid pieces, are accepted exactly when every piece is valid.
//!
//! `queries.toml` is not covered: it arrives with US5, which is not built yet.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use keel_relay::config::{Config, ConfigError};
use keel_relay::key::{KeyError, parse_hub_keys};
use proptest::prelude::*;

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIIEZ4xWsideTlTAh1vJelufYjc4v6+LaiYhsjHknykkI";
const RSA: &str = "AAAAB3NzaC1yc2EAAAADAQABAAAAgQD655ej3dk+g/NLllXUmvGWDOO/Tl6OnI07LimTcgEugUDh2Tp\
                   EAhwpp5OmQJu4Nczdp3RwCRobC0tVslh0ak14tN0wvOykbOsskireLK7IMb/3p9KXJoa++E60JF9fP2\
                   5+NA+WnN0YkU2ow1Z67IumjgHgBrB+l4HBXRjvf2KAyw==";

fn path() -> &'static Path {
    Path::new("/etc/keel-relay/relay.toml")
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

// --- relay.toml -----------------------------------------------------------------------------

const CONFIG_KEYS: &[&str] = &[
    "link_listen",
    "runner_listen",
    "hub_keys",
    "http_listen",
    "max_streams",
];

/// Panics unless an accepted `relay.toml` is what the contract allows.
fn accepted_config_is_valid(text: &str, config: &Config) {
    let table: toml::Table = toml::from_str(text).expect("accepted, but not TOML");
    let keys: BTreeSet<&str> = table.keys().map(String::as_str).collect();
    assert!(keys.contains("hub_keys"), "accepted without hub_keys");
    assert!(
        keys.iter().all(|k| CONFIG_KEYS.contains(k)),
        "accepted unknown keys: {keys:?}"
    );
    assert!(config.hub_keys.is_absolute());
    assert!(config.max_streams >= 1);
    assert_ne!(config.link_listen, config.runner_listen);
    if let Some(http) = config.http_listen {
        assert!(http != config.link_listen && http != config.runner_listen);
    }
}

fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let result = Config::parse(text, path());
    if let Ok(config) = &result {
        accepted_config_is_valid(text, config);
    }
    result
}

/// One `relay.toml` value: its TOML text, and what it means (`None`: the wrong kind).
#[derive(Debug, Clone)]
struct Choice<T: 'static> {
    text: &'static str,
    value: Option<T>,
}

fn choose<T: Clone + std::fmt::Debug + 'static>(
    pool: &'static [(&'static str, Option<T>)],
) -> impl Strategy<Value = Option<Choice<T>>> {
    prop::option::weighted(
        0.7,
        prop::sample::select(pool).prop_map(|(text, value)| Choice { text, value }),
    )
}

fn addr(s: &str) -> Option<SocketAddr> {
    Some(s.parse().unwrap())
}

type Addrs = [(&'static str, Option<SocketAddr>); 7];

fn addrs() -> &'static Addrs {
    static POOL: std::sync::OnceLock<Addrs> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        [
            ("\"0.0.0.0:7832\"", addr("0.0.0.0:7832")),
            ("\"0.0.0.0:7831\"", addr("0.0.0.0:7831")),
            ("\"127.0.0.1:9000\"", addr("127.0.0.1:9000")),
            ("\"[::1]:7832\"", addr("[::1]:7832")),
            ("\"localhost:80\"", None),
            ("\"0.0.0.0\"", None),
            ("7832", None),
        ]
    })
}

const HUB_KEYS: &[(&str, Option<&str>)] = &[
    ("\"/etc/keel-relay/hub_keys\"", Some("/etc/keel-relay/hub_keys")),
    ("\"/hub keys\"", Some("/hub keys")),
    ("\"hub_keys\"", Some("hub_keys")),
    ("\"\"", Some("")),
    ("5", None),
    ("[\"/a\"]", None),
];

const MAX_STREAMS: &[(&str, Option<u32>)] = &[
    ("1", Some(1)),
    ("256", Some(256)),
    ("4294967295", Some(u32::MAX)),
    ("0", Some(0)),
    ("-1", None),
    ("4294967296", None),
    ("\"8\"", None),
    ("1.5", None),
];

#[derive(Debug, Clone)]
struct Toml {
    link: Option<Choice<SocketAddr>>,
    runner: Option<Choice<SocketAddr>>,
    hub: Option<Choice<&'static str>>,
    http: Option<Choice<SocketAddr>>,
    max: Option<Choice<u32>>,
    extra: bool,
}

impl Toml {
    fn text(&self) -> String {
        let mut out = String::new();
        let mut line = |k: &str, text: Option<&str>| {
            if let Some(t) = text {
                out.push_str(&format!("{k} = {t}\n"));
            }
        };
        line("link_listen", self.link.as_ref().map(|c| c.text));
        line("runner_listen", self.runner.as_ref().map(|c| c.text));
        line("hub_keys", self.hub.as_ref().map(|c| c.text));
        line("http_listen", self.http.as_ref().map(|c| c.text));
        line("max_streams", self.max.as_ref().map(|c| c.text));
        if self.extra {
            out.push_str("query_listen = \"0.0.0.0:7833\"\n");
        }
        out
    }

    /// Whether the relay must accept this file, worked out from the contract.
    fn valid(&self) -> bool {
        fn get<T: Clone>(c: &Option<Choice<T>>, default: Option<T>) -> Result<Option<T>, ()> {
            match c {
                None => Ok(default),
                Some(Choice { value: None, .. }) => Err(()),
                Some(Choice { value: Some(v), .. }) => Ok(Some(v.clone())),
            }
        }
        let parsed = (|| {
            Ok::<_, ()>((
                get(&self.link, addr("0.0.0.0:7832"))?.unwrap(),
                get(&self.runner, addr("0.0.0.0:7831"))?.unwrap(),
                get(&self.hub, None)?,
                get(&self.http, None)?,
                get(&self.max, Some(256))?.unwrap(),
            ))
        })();
        let Ok((link, runner, hub, http, max)) = parsed else {
            return false;
        };
        !self.extra
            && hub.is_some_and(|h| PathBuf::from(h).is_absolute())
            && max >= 1
            && link != runner
            && http.is_none_or(|h| h != link && h != runner)
    }
}

fn toml_file() -> impl Strategy<Value = Toml> {
    (
        choose(addrs()),
        choose(addrs()),
        prop::option::weighted(
            0.9,
            prop::sample::select(HUB_KEYS).prop_map(|(text, value)| Choice { text, value }),
        ),
        choose(addrs()),
        choose(MAX_STREAMS),
        prop::bool::weighted(0.15),
    )
        .prop_map(|(link, runner, hub, http, max, extra)| Toml {
            link,
            runner,
            hub,
            http,
            max,
            extra,
        })
}

/// TOML-ish noise: lines of known and unknown keys with values of any shape.
fn toml_like() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        prop::sample::select(CONFIG_KEYS).prop_map(str::to_owned),
        "[a-z_]{1,8}",
        Just("[section]".to_owned()),
    ];
    let value = prop_oneof![
        Just("\"/etc/keel-relay/hub_keys\"".to_owned()),
        Just("\"127.0.0.1:1\"".to_owned()),
        Just("\"127.0.0.1:2\"".to_owned()),
        any::<i64>().prop_map(|n| n.to_string()),
        "\"[ -~]{0,12}\"",
        "[ -~]{0,12}",
    ];
    prop::collection::vec((key, value), 0..7).prop_map(|lines| {
        lines
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}\n"))
            .collect()
    })
}

// --- hub_keys -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Key,
    KeyWithComment,
    Indented,
    Blank,
    Comment,
    Garbage,
    Rsa,
    Ed25519TypeOnRsaBlob,
    TruncatedKey,
    TypeOnly,
}

impl Line {
    const ALL: [Line; 10] = [
        Line::Key,
        Line::KeyWithComment,
        Line::Indented,
        Line::Blank,
        Line::Comment,
        Line::Garbage,
        Line::Rsa,
        Line::Ed25519TypeOnRsaBlob,
        Line::TruncatedKey,
        Line::TypeOnly,
    ];

    fn text(self) -> String {
        match self {
            Line::Key => format!("ssh-ed25519 {KEY}"),
            Line::KeyWithComment => format!("ssh-ed25519 {KEY} keel@hub-1"),
            Line::Indented => format!("  \tssh-ed25519 {KEY}  "),
            Line::Blank => "   ".to_owned(),
            Line::Comment => "# rotated 2026-11-01".to_owned(),
            Line::Garbage => "not a key at all".to_owned(),
            Line::Rsa => format!("ssh-rsa {RSA} old"),
            Line::Ed25519TypeOnRsaBlob => format!("ssh-ed25519 {RSA}"),
            Line::TruncatedKey => format!("ssh-ed25519 {}", &KEY[..KEY.len() - 12]),
            Line::TypeOnly => "ssh-ed25519".to_owned(),
        }
    }

    fn is_key(self) -> bool {
        matches!(self, Line::Key | Line::KeyWithComment | Line::Indented)
    }

    fn is_skipped(self) -> bool {
        matches!(self, Line::Blank | Line::Comment)
    }
}

/// Panics unless an accepted `hub_keys` file is only ed25519 keys, blanks and comments.
fn accepted_hub_keys_are_valid(text: &str, keys: usize) {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(!lines.is_empty(), "accepted a file with no keys");
    assert_eq!(lines.len(), keys, "a line was skipped or doubled");
    for line in lines {
        assert_eq!(
            line.split_whitespace().next(),
            Some("ssh-ed25519"),
            "accepted {line:?}"
        );
    }
}

fn parse_keys(text: &str) -> Result<usize, KeyError> {
    let result = parse_hub_keys(text, Path::new("/etc/keel-relay/hub_keys")).map(|k| k.len());
    if let Ok(n) = result {
        accepted_hub_keys_are_valid(text, n);
    }
    result
}

// --- Properties -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_text_never_panics_the_config_parser(text in "\\PC{0,256}") {
        let _ = parse_config(&text);
    }

    #[test]
    fn toml_shaped_noise_is_accepted_only_within_the_contract(text in toml_like()) {
        let _ = parse_config(&text);
    }

    #[test]
    fn a_config_is_accepted_exactly_when_every_value_is_valid(file in toml_file()) {
        let text = file.text();
        let result = parse_config(&text);
        prop_assert_eq!(result.is_ok(), file.valid(), "{}\n{:?}", text, result);
        if let Ok(config) = result {
            if let Some(Choice { value: Some(v), .. }) = &file.link {
                prop_assert_eq!(config.link_listen, *v);
            }
            if let Some(Choice { value: Some(v), .. }) = &file.max {
                prop_assert_eq!(config.max_streams, *v);
            }
            prop_assert_eq!(config.http_listen, file.http.as_ref().and_then(|c| c.value));
        }
    }

    #[test]
    fn arbitrary_text_never_panics_the_hub_keys_parser(text in "\\PC{0,512}") {
        let _ = parse_keys(&text);
    }

    #[test]
    fn hub_keys_shaped_noise_is_accepted_only_within_the_contract(
        text in prop::collection::vec(
            prop_oneof![
                Just(format!("ssh-ed25519 {KEY}")),
                "ssh-ed25519 [A-Za-z0-9+/=]{0,80}( [ -~]{0,8})?",
                "[ -~]{0,40}",
                Just("#".to_owned()),
            ],
            0..6,
        ).prop_map(|l| l.join("\n"))
    ) {
        let _ = parse_keys(&text);
    }

    #[test]
    fn a_hub_keys_file_is_accepted_exactly_when_every_line_is_a_key_or_skipped(
        lines in prop::collection::vec(prop::sample::select(&Line::ALL[..]), 0..8),
        crlf in any::<bool>(),
    ) {
        let text = lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join(if crlf { "\r\n" } else { "\n" });
        let result = parse_keys(&text);
        let first_bad = lines.iter().position(|l| !l.is_key() && !l.is_skipped());
        let keys = lines.iter().filter(|l| l.is_key()).count();
        match (first_bad, keys) {
            (Some(i), _) => prop_assert!(
                matches!(result, Err(KeyError::HubKeyLine { line, .. }) if line == i + 1),
                "{:?}: {:?}", lines, result
            ),
            (None, 0) => prop_assert!(
                matches!(result, Err(KeyError::NoHubKeys { .. })),
                "{:?}: {:?}", lines, result
            ),
            (None, n) => prop_assert_eq!(result.ok(), Some(n), "{:?}", lines),
        }
    }
}
