//! Network trust (spec 052 R1, R2): which network a request comes from, and whether that
//! network is trusted for this person.
//!
//! - The first network a person ever uses is trusted.
//! - Any other network is unknown until the person answers "home or public". An answer never
//!   grants trust by itself: "home" matures after the maturity window *and* use on at least two
//!   distinct UTC days; "public" never matures.
//! - An admin whitelist, and the operator's always-trusted list, trust a network at once.
//!
//! Everything here is pure: callers pass `now`, and the server persists [`KnownNetwork`]s.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Most networks remembered per person; the least recently used non-whitelisted one goes first.
pub const MAX_NETWORKS_PER_PERSON: usize = 64;
/// Distinct days of use remembered per network. Only "at least two" matters.
pub const MAX_DAYS: usize = 8;
/// A non-whitelisted network unused this long is forgotten.
pub const PRUNE_AFTER_DAYS: i64 = 180;
/// Longest admin label on a whitelist entry, in characters.
pub const MAX_LABEL_CHARS: usize = 64;

/// An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) as the IPv4 address it is.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// How addresses group into networks: IPv4 `/32` and IPv6 `/64` by default (R1). Privacy
/// addresses rotate within a `/64`; a home IPv4 is usually one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grouping {
    pub v4_prefix: u8,
    pub v6_prefix: u8,
}

impl Default for Grouping {
    fn default() -> Self {
        Self {
            v4_prefix: 32,
            v6_prefix: 64,
        }
    }
}

/// A network: an address with its host bits cleared, and a prefix length. Serialises as the
/// canonical CIDR string, `203.0.113.7/32` or `2001:db8:1:2::/64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetworkKey {
    net: IpAddr,
    prefix: u8,
}

fn mask_v4(a: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let bits = u32::from(a);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    Ipv4Addr::from(bits & mask)
}

fn mask_v6(a: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let bits = u128::from(a);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    Ipv6Addr::from(bits & mask)
}

impl NetworkKey {
    /// `ip/prefix` with host bits cleared; `None` when the prefix is too long for the family.
    pub fn new(ip: IpAddr, prefix: u8) -> Option<Self> {
        match canonical(ip) {
            IpAddr::V4(a) if prefix <= 32 => Some(Self {
                net: IpAddr::V4(mask_v4(a, prefix)),
                prefix,
            }),
            IpAddr::V6(a) if prefix <= 128 => Some(Self {
                net: IpAddr::V6(mask_v6(a, prefix)),
                prefix,
            }),
            _ => None,
        }
    }

    /// The network `ip` belongs to under `grouping`.
    pub fn of(ip: IpAddr, grouping: Grouping) -> Self {
        let ip = canonical(ip);
        let prefix = match ip {
            IpAddr::V4(_) => grouping.v4_prefix.min(32),
            IpAddr::V6(_) => grouping.v6_prefix.min(128),
        };
        Self::new(ip, prefix).unwrap_or(Self { net: ip, prefix })
    }

    /// `a.b.c.d/n`, `x::/n`, or a bare address (a single-address network).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        match s.split_once('/') {
            Some((addr, len)) => {
                let ip: IpAddr = addr.parse().ok()?;
                if len.is_empty() || !len.bytes().all(|b| b.is_ascii_digit()) || len.len() > 3 {
                    return None;
                }
                let prefix: u8 = len.parse().ok()?;
                // A mapped address with a v6 prefix: keep the v4 meaning.
                let prefix = match (ip, canonical(ip)) {
                    (IpAddr::V6(_), IpAddr::V4(_)) => prefix.checked_sub(96)?,
                    _ => prefix,
                };
                Self::new(ip, prefix)
            }
            None => {
                let ip = canonical(s.parse().ok()?);
                let prefix = if ip.is_ipv4() { 32 } else { 128 };
                Self::new(ip, prefix)
            }
        }
    }

    pub fn net(&self) -> IpAddr {
        self.net
    }

    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, canonical(ip)) {
            (IpAddr::V4(n), IpAddr::V4(a)) => mask_v4(a, self.prefix) == n,
            (IpAddr::V6(n), IpAddr::V6(a)) => mask_v6(a, self.prefix) == n,
            _ => false,
        }
    }

    /// Whether every address of `other` is in `self`.
    pub fn covers(&self, other: &NetworkKey) -> bool {
        self.prefix <= other.prefix && self.contains(other.net)
    }

    /// Whether the two share any address.
    pub fn overlaps(&self, other: &NetworkKey) -> bool {
        self.covers(other) || other.covers(self)
    }
}

impl fmt::Display for NetworkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.net, self.prefix)
    }
}

/// Why a string is not a network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidNetwork(pub String);

impl fmt::Display for InvalidNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a network (like 203.0.113.0/24)", self.0)
    }
}

impl std::error::Error for InvalidNetwork {}

impl FromStr for NetworkKey {
    type Err = InvalidNetwork;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| InvalidNetwork(s.to_owned()))
    }
}

impl Serialize for NetworkKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NetworkKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(InvalidNetwork(s)))
    }
}

/// Where a remembered network came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The first network this person used: trusted.
    First,
    /// The person answered the question for it.
    Answer,
    /// An admin whitelisted it: trusted at once.
    Whitelist,
    /// Used, not yet answered.
    Seen,
}

/// The person's answer to "home or public".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Home,
    Public,
    #[default]
    None,
}

/// One remembered network of one person (data-model.md). Whitelist entries are their own rows
/// (`source: whitelist`, any prefix) beside the rows of networks actually used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownNetwork {
    pub key: NetworkKey,
    pub first_used: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    /// Distinct UTC days of use, sorted, the latest [`MAX_DAYS`] kept.
    #[serde(default)]
    pub days: Vec<NaiveDate>,
    pub source: Source,
    #[serde(default)]
    pub answer: Answer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whitelisted_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl KnownNetwork {
    pub fn is_whitelist(&self) -> bool {
        self.source == Source::Whitelist
    }

    fn touch(&mut self, now: DateTime<Utc>) {
        if now > self.last_used {
            self.last_used = now;
        }
        let day = now.date_naive();
        if let Err(at) = self.days.binary_search(&day) {
            self.days.insert(at, day);
        }
        if self.days.len() > MAX_DAYS {
            let drop = self.days.len() - MAX_DAYS;
            self.days.drain(..drop);
        }
    }
}

/// Why a network is or is not trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustReason {
    /// In the operator's `DEPTHS_TRUSTED_NETWORKS`.
    Always,
    /// The person's first network.
    First,
    /// An admin whitelisted it.
    Whitelist,
    /// Answered "home", used long enough on enough days.
    Matured,
    /// `DEPTHS_NETWORK_GATE=false`.
    GateOff,
    /// Not answered yet.
    Unknown,
    /// Answered "public": never matures.
    Public,
    /// Answered "home", not yet matured.
    Maturing,
}

/// The gate's answer for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustDecision {
    pub trusted: bool,
    pub reason: TrustReason,
    /// For a maturing "home" network: when the window ends (two distinct days are needed too).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matures_at: Option<DateTime<Utc>>,
    pub network: NetworkKey,
}

impl TrustDecision {
    /// Whether the person should be asked "home or public" for this network.
    pub fn needs_answer(&self) -> bool {
        self.reason == TrustReason::Unknown
    }
}

/// The operator's settings for the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustPolicy {
    /// `DEPTHS_NETWORK_GATE`: off trusts every network (still recorded).
    pub gate: bool,
    /// `DEPTHS_NETWORK_MATURITY_DAYS`: 0 means "home" never matures on its own.
    pub maturity_days: u32,
    pub grouping: Grouping,
    /// `DEPTHS_TRUSTED_NETWORKS`.
    pub always: Vec<NetworkKey>,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            gate: true,
            maturity_days: 7,
            grouping: Grouping::default(),
            always: Vec::new(),
        }
    }
}

impl TrustPolicy {
    pub fn key_of(&self, ip: IpAddr) -> NetworkKey {
        NetworkKey::of(ip, self.grouping)
    }
}

/// The used-network row for `key`, if any (not a whitelist row).
pub fn entry<'a>(networks: &'a [KnownNetwork], key: &NetworkKey) -> Option<&'a KnownNetwork> {
    networks.iter().find(|n| !n.is_whitelist() && n.key == *key)
}

/// R2: `gate_off || always || first || whitelisted || (home && window>0 && now-first_used >=
/// window && distinct_days >= 2)`. `public` never matures. Days are counted in UTC.
pub fn trust(
    policy: &TrustPolicy,
    ip: IpAddr,
    networks: &[KnownNetwork],
    now: DateTime<Utc>,
) -> TrustDecision {
    let network = policy.key_of(ip);
    let decide = |trusted, reason, matures_at| TrustDecision {
        trusted,
        reason,
        matures_at,
        network,
    };
    if !policy.gate {
        return decide(true, TrustReason::GateOff, None);
    }
    if policy.always.iter().any(|c| c.contains(ip)) {
        return decide(true, TrustReason::Always, None);
    }
    let row = entry(networks, &network);
    if row.is_some_and(|n| n.source == Source::First) {
        return decide(true, TrustReason::First, None);
    }
    if networks
        .iter()
        .any(|n| n.is_whitelist() && n.key.contains(ip))
    {
        return decide(true, TrustReason::Whitelist, None);
    }
    let Some(row) = row else {
        return decide(false, TrustReason::Unknown, None);
    };
    match row.answer {
        Answer::None => decide(false, TrustReason::Unknown, None),
        Answer::Public => decide(false, TrustReason::Public, None),
        Answer::Home => {
            if policy.maturity_days == 0 {
                return decide(false, TrustReason::Maturing, None);
            }
            let window = Duration::days(i64::from(policy.maturity_days));
            let at = row.first_used + window;
            if now >= at && row.days.len() >= 2 {
                decide(true, TrustReason::Matured, None)
            } else {
                decide(false, TrustReason::Maturing, Some(at))
            }
        }
    }
}

/// What [`record_use`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// The person's first network: now trusted.
    First,
    /// A network not seen before.
    New,
    /// Already known; its last use and days were updated.
    Seen,
}

/// Records a use of `key` at `now`. The first network of a person with no used networks becomes
/// `first` (whitelist rows do not count). Keeps at most [`MAX_NETWORKS_PER_PERSON`] rows.
pub fn record_use(
    networks: &mut Vec<KnownNetwork>,
    key: NetworkKey,
    now: DateTime<Utc>,
) -> Recorded {
    if let Some(n) = networks
        .iter_mut()
        .find(|n| !n.is_whitelist() && n.key == key)
    {
        n.touch(now);
        return Recorded::Seen;
    }
    let first = !networks.iter().any(|n| !n.is_whitelist());
    let mut row = KnownNetwork {
        key,
        first_used: now,
        last_used: now,
        days: Vec::new(),
        source: if first { Source::First } else { Source::Seen },
        answer: Answer::None,
        whitelisted_by: None,
        at: None,
        label: None,
    };
    row.touch(now);
    networks.push(row);
    cap(networks);
    if first {
        Recorded::First
    } else {
        Recorded::New
    }
}

/// Drops least recently used rows past the cap. Whitelist rows and the first network stay.
pub fn cap(networks: &mut Vec<KnownNetwork>) {
    while networks.len() > MAX_NETWORKS_PER_PERSON {
        let victim = networks
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.is_whitelist() && n.source != Source::First)
            .min_by_key(|(_, n)| n.last_used)
            .map(|(i, _)| i);
        match victim {
            Some(i) => {
                networks.remove(i);
            }
            None => break,
        }
    }
}

/// Forgets non-whitelisted rows unused for [`PRUNE_AFTER_DAYS`]. Returns how many went.
pub fn prune(networks: &mut Vec<KnownNetwork>, now: DateTime<Utc>) -> usize {
    let before = networks.len();
    let cutoff = now - Duration::days(PRUNE_AFTER_DAYS);
    networks.retain(|n| n.is_whitelist() || n.last_used >= cutoff);
    before - networks.len()
}

/// Why an answer was not recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerError {
    /// The person has not used that network.
    NotKnown,
    /// `Answer::None` is not an answer.
    NoAnswer,
}

/// Records the person's answer for a network they used. Never grants trust: a first network
/// stays first, and "home" still has to mature.
pub fn answer(
    networks: &mut [KnownNetwork],
    key: &NetworkKey,
    answer: Answer,
) -> Result<(), AnswerError> {
    if answer == Answer::None {
        return Err(AnswerError::NoAnswer);
    }
    let row = networks
        .iter_mut()
        .find(|n| !n.is_whitelist() && n.key == *key)
        .ok_or(AnswerError::NotKnown)?;
    row.answer = answer;
    if row.source == Source::Seen {
        row.source = Source::Answer;
    }
    Ok(())
}

/// A whitelist label as stored: trimmed, control characters dropped, at most
/// [`MAX_LABEL_CHARS`] characters; empty is none.
pub fn clean_label(label: &str) -> Option<String> {
    let s: String = label
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    let s = s.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

/// Adds (or relabels) a whitelist row for `cidr`.
pub fn whitelist(
    networks: &mut Vec<KnownNetwork>,
    cidr: NetworkKey,
    by: &str,
    label: Option<&str>,
    now: DateTime<Utc>,
) {
    let label = label.and_then(clean_label);
    if let Some(n) = networks
        .iter_mut()
        .find(|n| n.is_whitelist() && n.key == cidr)
    {
        n.whitelisted_by = Some(by.to_owned());
        n.at = Some(now);
        n.label = label;
        return;
    }
    networks.push(KnownNetwork {
        key: cidr,
        first_used: now,
        last_used: now,
        days: Vec::new(),
        source: Source::Whitelist,
        answer: Answer::None,
        whitelisted_by: Some(by.to_owned()),
        at: Some(now),
        label,
    });
}

/// What [`remove`] took away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    Whitelist,
    Forgotten,
}

/// Removes the whitelist row for `key` if there is one, otherwise forgets the used network.
pub fn remove(networks: &mut Vec<KnownNetwork>, key: &NetworkKey) -> Option<Removed> {
    if let Some(i) = networks
        .iter()
        .position(|n| n.is_whitelist() && n.key == *key)
    {
        networks.remove(i);
        return Some(Removed::Whitelist);
    }
    let i = networks.iter().position(|n| n.key == *key)?;
    networks.remove(i);
    Some(Removed::Forgotten)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn t(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn key(s: &str) -> NetworkKey {
        NetworkKey::parse(s).unwrap()
    }

    #[test]
    fn keys_group_and_normalise() {
        let g = Grouping::default();
        assert_eq!(
            NetworkKey::of(ip("203.0.113.7"), g).to_string(),
            "203.0.113.7/32"
        );
        assert_eq!(
            NetworkKey::of(ip("::ffff:203.0.113.7"), g).to_string(),
            "203.0.113.7/32"
        );
        assert_eq!(
            NetworkKey::of(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), g).to_string(),
            "2001:db8:1:2::/64"
        );
        let wide = Grouping {
            v4_prefix: 24,
            v6_prefix: 48,
        };
        assert_eq!(
            NetworkKey::of(ip("203.0.113.7"), wide).to_string(),
            "203.0.113.0/24"
        );
        assert_eq!(
            NetworkKey::of(ip("2001:db8:1:2::9"), wide).to_string(),
            "2001:db8:1::/48"
        );
    }

    #[test]
    fn parse_masks_and_refuses_nonsense() {
        assert_eq!(key("10.1.2.3/8").to_string(), "10.0.0.0/8");
        assert_eq!(key("10.1.2.3").to_string(), "10.1.2.3/32");
        assert_eq!(key("::1").to_string(), "::1/128");
        assert_eq!(key("::ffff:10.1.2.3/120").to_string(), "10.1.2.0/24");
        for bad in [
            "10.0.0.0/33",
            "::/129",
            "10.0.0.0/",
            "10.0.0.0/a",
            "nope",
            "",
            "10.0.0.0/-1",
        ] {
            assert!(NetworkKey::parse(bad).is_none(), "{bad}");
        }
        let k: NetworkKey = serde_json::from_str(r#""2001:db8::/32""#).unwrap();
        assert_eq!(serde_json::to_string(&k).unwrap(), r#""2001:db8::/32""#);
    }

    #[test]
    fn contains_and_covers() {
        assert!(key("100.64.0.0/10").contains(ip("100.100.1.1")));
        assert!(key("100.64.0.0/10").contains(ip("::ffff:100.100.1.1")));
        assert!(!key("100.64.0.0/10").contains(ip("100.128.0.1")));
        assert!(!key("10.0.0.0/8").contains(ip("::1")));
        assert!(key("10.0.0.0/8").covers(&key("10.1.0.0/16")));
        assert!(!key("10.1.0.0/16").covers(&key("10.0.0.0/8")));
        assert!(key("10.1.0.0/16").overlaps(&key("10.0.0.0/8")));
        assert!(key("0.0.0.0/0").contains(ip("8.8.8.8")));
    }

    fn policy() -> TrustPolicy {
        TrustPolicy::default()
    }

    fn used(
        key_s: &str,
        source: Source,
        answer: Answer,
        first: DateTime<Utc>,
        days: &[u32],
    ) -> KnownNetwork {
        KnownNetwork {
            key: key(key_s),
            first_used: first,
            last_used: first,
            days: days
                .iter()
                .map(|d| NaiveDate::from_ymd_opt(2026, 10, *d).unwrap())
                .collect(),
            source,
            answer,
            whitelisted_by: None,
            at: None,
            label: None,
        }
    }

    /// (case, networks, policy tweak, ip, now, trusted, reason).
    #[test]
    fn the_trust_rule() {
        let home = "198.51.100.9/32";
        let first = used("203.0.113.7/32", Source::First, Answer::None, t(1, 0), &[1]);
        type Row = (
            &'static str,
            Vec<KnownNetwork>,
            fn(&mut TrustPolicy),
            &'static str,
            DateTime<Utc>,
            bool,
            TrustReason,
        );
        let none: fn(&mut TrustPolicy) = |_| {};
        let rows: Vec<Row> = vec![
            (
                "first network",
                vec![first.clone()],
                none,
                "203.0.113.7",
                t(1, 1),
                true,
                TrustReason::First,
            ),
            (
                "unknown",
                vec![first.clone()],
                none,
                "198.51.100.9",
                t(1, 1),
                false,
                TrustReason::Unknown,
            ),
            (
                "seen, unanswered",
                vec![
                    first.clone(),
                    used(home, Source::Seen, Answer::None, t(1, 0), &[1, 2]),
                ],
                none,
                "198.51.100.9",
                t(20, 0),
                false,
                TrustReason::Unknown,
            ),
            (
                "public never matures",
                vec![
                    first.clone(),
                    used(home, Source::Answer, Answer::Public, t(1, 0), &[1, 2, 3]),
                ],
                none,
                "198.51.100.9",
                t(30, 0),
                false,
                TrustReason::Public,
            ),
            (
                "home, window not over",
                vec![
                    first.clone(),
                    used(home, Source::Answer, Answer::Home, t(1, 0), &[1, 2]),
                ],
                none,
                "198.51.100.9",
                t(7, 23),
                false,
                TrustReason::Maturing,
            ),
            (
                "home, window over, one day",
                vec![
                    first.clone(),
                    used(home, Source::Answer, Answer::Home, t(1, 0), &[1]),
                ],
                none,
                "198.51.100.9",
                t(9, 0),
                false,
                TrustReason::Maturing,
            ),
            (
                "home matured",
                vec![
                    first.clone(),
                    used(home, Source::Answer, Answer::Home, t(1, 0), &[1, 5]),
                ],
                none,
                "198.51.100.9",
                t(8, 0),
                true,
                TrustReason::Matured,
            ),
            (
                "home, maturity 0 never",
                vec![
                    first.clone(),
                    used(home, Source::Answer, Answer::Home, t(1, 0), &[1, 5]),
                ],
                |p| p.maturity_days = 0,
                "198.51.100.9",
                t(30, 0),
                false,
                TrustReason::Maturing,
            ),
            (
                "gate off",
                vec![first.clone()],
                |p| p.gate = false,
                "198.51.100.9",
                t(1, 1),
                true,
                TrustReason::GateOff,
            ),
            (
                "always trusted",
                vec![first.clone()],
                |p| p.always = vec![key("198.51.100.0/24")],
                "198.51.100.9",
                t(1, 1),
                true,
                TrustReason::Always,
            ),
            (
                "always, v4-mapped",
                vec![first.clone()],
                |p| p.always = vec![key("198.51.100.0/24")],
                "::ffff:198.51.100.9",
                t(1, 1),
                true,
                TrustReason::Always,
            ),
            (
                "no networks at all",
                vec![],
                none,
                "198.51.100.9",
                t(1, 1),
                false,
                TrustReason::Unknown,
            ),
        ];
        for (case, nets, tweak, addr, now, trusted, reason) in rows {
            let mut p = policy();
            tweak(&mut p);
            let d = trust(&p, ip(addr), &nets, now);
            assert_eq!((d.trusted, d.reason), (trusted, reason), "{case}");
        }
    }

    #[test]
    fn maturing_says_when() {
        let nets = vec![used(
            "198.51.100.9/32",
            Source::Answer,
            Answer::Home,
            t(1, 0),
            &[1],
        )];
        let d = trust(&policy(), ip("198.51.100.9"), &nets, t(2, 0));
        assert_eq!(d.matures_at, Some(t(8, 0)));
        assert!(!d.needs_answer());
    }

    #[test]
    fn a_whitelist_trusts_at_once_and_removal_reverts() {
        let mut nets = vec![used(
            "203.0.113.7/32",
            Source::First,
            Answer::None,
            t(1, 0),
            &[1],
        )];
        record_use(&mut nets, key("198.51.100.9/32"), t(2, 0));
        assert!(!trust(&policy(), ip("198.51.100.9"), &nets, t(2, 0)).trusted);
        whitelist(
            &mut nets,
            key("198.51.100.0/24"),
            "admin",
            Some(" office\n "),
            t(2, 1),
        );
        let d = trust(&policy(), ip("198.51.100.9"), &nets, t(2, 1));
        assert_eq!((d.trusted, d.reason), (true, TrustReason::Whitelist));
        assert_eq!(nets.last().unwrap().label.as_deref(), Some("office"));
        assert_eq!(
            remove(&mut nets, &key("198.51.100.0/24")),
            Some(Removed::Whitelist)
        );
        let d = trust(&policy(), ip("198.51.100.9"), &nets, t(2, 2));
        assert_eq!((d.trusted, d.reason), (false, TrustReason::Unknown));
        assert_eq!(
            remove(&mut nets, &key("198.51.100.9/32")),
            Some(Removed::Forgotten)
        );
        assert_eq!(remove(&mut nets, &key("198.51.100.9/32")), None);
    }

    #[test]
    fn record_first_then_new_then_seen() {
        let mut nets = Vec::new();
        assert_eq!(
            record_use(&mut nets, key("203.0.113.7/32"), t(1, 0)),
            Recorded::First
        );
        assert_eq!(
            record_use(&mut nets, key("198.51.100.9/32"), t(1, 0)),
            Recorded::New
        );
        assert_eq!(
            record_use(&mut nets, key("198.51.100.9/32"), t(1, 5)),
            Recorded::Seen
        );
        assert_eq!(
            record_use(&mut nets, key("198.51.100.9/32"), t(3, 0)),
            Recorded::Seen
        );
        let row = entry(&nets, &key("198.51.100.9/32")).unwrap();
        assert_eq!(row.days.len(), 2);
        assert_eq!(row.last_used, t(3, 0));
        assert_eq!(row.source, Source::Seen);
        // A whitelist row alone does not make the next network non-first.
        let mut only_wl = Vec::new();
        whitelist(&mut only_wl, key("10.0.0.0/8"), "admin", None, t(1, 0));
        assert_eq!(
            record_use(&mut only_wl, key("203.0.113.7/32"), t(1, 0)),
            Recorded::First
        );
    }

    #[test]
    fn days_are_utc_and_capped() {
        let mut nets = Vec::new();
        for d in 1..=12 {
            record_use(&mut nets, key("203.0.113.7/32"), t(d, 23));
        }
        assert_eq!(nets[0].days.len(), MAX_DAYS);
        assert_eq!(
            nets[0].days[0],
            NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
        );
    }

    #[test]
    fn answers_never_grant_trust() {
        let mut nets = Vec::new();
        record_use(&mut nets, key("203.0.113.7/32"), t(1, 0));
        record_use(&mut nets, key("198.51.100.9/32"), t(1, 0));
        answer(&mut nets, &key("198.51.100.9/32"), Answer::Home).unwrap();
        assert!(!trust(&policy(), ip("198.51.100.9"), &nets, t(1, 1)).trusted);
        // Answering on the first network keeps it first.
        answer(&mut nets, &key("203.0.113.7/32"), Answer::Public).unwrap();
        assert!(trust(&policy(), ip("203.0.113.7"), &nets, t(1, 1)).trusted);
        assert_eq!(
            answer(&mut nets, &key("192.0.2.1/32"), Answer::Home),
            Err(AnswerError::NotKnown)
        );
        assert_eq!(
            answer(&mut nets, &key("198.51.100.9/32"), Answer::None),
            Err(AnswerError::NoAnswer)
        );
    }

    #[test]
    fn cap_drops_least_recent_but_keeps_first_and_whitelist() {
        let mut nets = Vec::new();
        record_use(&mut nets, key("203.0.113.7/32"), t(1, 0));
        whitelist(&mut nets, key("10.0.0.0/8"), "admin", None, t(1, 0));
        for i in 0..MAX_NETWORKS_PER_PERSON {
            let k = NetworkKey::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, i as u8)), 32).unwrap();
            record_use(&mut nets, k, t(2, 0) + Duration::minutes(i as i64));
        }
        assert_eq!(nets.len(), MAX_NETWORKS_PER_PERSON);
        assert!(nets.iter().any(|n| n.source == Source::First));
        assert!(nets.iter().any(|n| n.is_whitelist()));
        assert!(entry(&nets, &key("192.0.2.0/32")).is_none());
        assert!(entry(&nets, &key("192.0.2.1/32")).is_none());
        assert!(entry(&nets, &key("192.0.2.2/32")).is_some());
    }

    #[test]
    fn prune_forgets_old_unwhitelisted() {
        let mut nets = Vec::new();
        record_use(&mut nets, key("203.0.113.7/32"), t(1, 0));
        whitelist(&mut nets, key("10.0.0.0/8"), "admin", None, t(1, 0));
        let later = t(1, 0) + Duration::days(PRUNE_AFTER_DAYS + 1);
        assert_eq!(prune(&mut nets, later), 1);
        assert_eq!(nets.len(), 1);
        assert!(nets[0].is_whitelist());
    }

    #[test]
    fn labels_are_plain_and_short() {
        assert_eq!(clean_label("   "), None);
        assert_eq!(clean_label("a\u{7}b"), Some("ab".to_owned()));
        assert_eq!(
            clean_label(&"x".repeat(100)).unwrap().chars().count(),
            MAX_LABEL_CHARS
        );
    }

    #[test]
    fn known_networks_round_trip_as_data_model_says() {
        let json = r#"{"key":"203.0.113.7/32","first_used":"2026-10-01T00:00:00Z","last_used":"2026-10-02T00:00:00Z","days":["2026-10-01","2026-10-02"],"source":"first","answer":"none"}"#;
        let n: KnownNetwork = serde_json::from_str(json).unwrap();
        assert_eq!(n.days.len(), 2);
        assert_eq!(serde_json::to_string(&n).unwrap(), json);
    }
}
