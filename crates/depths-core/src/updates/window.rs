//! Maintenance windows in host local wall time (research R5).
//!
//! A window is a set of weekdays and a local `start`–`end`; `end <= start` crosses midnight. The
//! zone and the clock are injected, so DST is tested with a fake zone and no timezone crate:
//! - a start in a skipped hour starts at the first valid instant after the gap;
//! - an ambiguous start takes the earliest instant; the end resolves the same way;
//! - an occurrence whose end resolves at or before its start is empty, and the next one is used;
//! - each occurrence is keyed by the local date it starts on, and runs once.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Host local time. The server's adapter is `chrono::Local`.
pub trait LocalZone: Send + Sync {
    fn to_local(&self, utc: DateTime<Utc>) -> NaiveDateTime;
    fn from_local(&self, local: NaiveDateTime) -> LocalResult<DateTime<Utc>>;
}

/// The current instant. Injected so tests control time.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// A day of the week, `mon`..`sun` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];

    pub fn of(date: NaiveDate) -> Self {
        match date.weekday() {
            chrono::Weekday::Mon => Self::Mon,
            chrono::Weekday::Tue => Self::Tue,
            chrono::Weekday::Wed => Self::Wed,
            chrono::Weekday::Thu => Self::Thu,
            chrono::Weekday::Fri => Self::Fri,
            chrono::Weekday::Sat => Self::Sat,
            chrono::Weekday::Sun => Self::Sun,
        }
    }
}

/// A local wall time `HH:MM`, 00:00–23:59. Serialised as that string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HhMm {
    hour: u8,
    minute: u8,
}

impl HhMm {
    pub fn new(hour: u8, minute: u8) -> Option<Self> {
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    pub fn time(self) -> NaiveTime {
        NaiveTime::from_hms_opt(self.hour.into(), self.minute.into(), 0)
            .expect("validated hour and minute")
    }
}

impl FromStr for HhMm {
    type Err = WindowError;
    fn from_str(s: &str) -> Result<Self, WindowError> {
        let b = s.as_bytes();
        if b.len() != 5
            || b[2] != b':'
            || !b
                .iter()
                .enumerate()
                .all(|(i, c)| i == 2 || c.is_ascii_digit())
        {
            return Err(WindowError::Time);
        }
        let hour = (b[0] - b'0') * 10 + (b[1] - b'0');
        let minute = (b[3] - b'0') * 10 + (b[4] - b'0');
        Self::new(hour, minute).ok_or(WindowError::Time)
    }
}

impl fmt::Display for HhMm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl Serialize for HhMm {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HhMm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Why a window is refused (data-model.md "Host update settings").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    Time,
    NoDays,
    DuplicateDay,
    EmptyOrFullDay,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Time => "times are HH:MM, 00:00 to 23:59",
            Self::NoDays => "a window needs at least one day",
            Self::DuplicateDay => "a day is listed twice",
            Self::EmptyOrFullDay => "start and end are the same; a full day is 00:00 to 23:59",
        })
    }
}

impl std::error::Error for WindowError {}

/// A maintenance window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub days: Vec<Weekday>,
    pub start: HhMm,
    pub end: HhMm,
}

/// The local date a window occurrence starts on: one occurrence runs once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OccurrenceKey(pub NaiveDate);

/// One opening of a window, resolved to UTC. `start < end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub key: OccurrenceKey,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Occurrence {
    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.start <= t && t < self.end
    }
}

/// Resolve a local wall time: the earliest instant when repeated, the first valid instant after
/// the gap when skipped.
fn resolve(zone: &dyn LocalZone, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    let mut t = local;
    // Gaps are at most a few hours; a day bounds the search.
    for _ in 0..=(24 * 60) {
        match zone.from_local(t) {
            LocalResult::Single(u) => return Some(u),
            LocalResult::Ambiguous(a, b) => return Some(a.min(b)),
            LocalResult::None => t += Duration::minutes(1),
        }
    }
    None
}

impl Window {
    /// Refuse what data-model.md refuses: no days, a repeated day, `end == start`.
    pub fn validate(&self) -> Result<(), WindowError> {
        if self.days.is_empty() {
            return Err(WindowError::NoDays);
        }
        let mut seen = [false; 7];
        for d in &self.days {
            let i = *d as usize;
            if seen[i] {
                return Err(WindowError::DuplicateDay);
            }
            seen[i] = true;
        }
        if self.start == self.end {
            return Err(WindowError::EmptyOrFullDay);
        }
        Ok(())
    }

    pub fn crosses_midnight(&self) -> bool {
        self.end <= self.start
    }

    /// The occurrence that starts on local `date`, if the window has that weekday and the
    /// occurrence is not empty after DST resolution.
    pub fn occurrence_on(&self, date: NaiveDate, zone: &dyn LocalZone) -> Option<Occurrence> {
        if !self.days.contains(&Weekday::of(date)) {
            return None;
        }
        let end_date = if self.crosses_midnight() {
            date.succ_opt()?
        } else {
            date
        };
        let start = resolve(zone, date.and_time(self.start.time()))?;
        let end = resolve(zone, end_date.and_time(self.end.time()))?;
        (end > start).then_some(Occurrence {
            key: OccurrenceKey(date),
            start,
            end,
        })
    }

    /// The occurrence open at `now`, if any.
    pub fn current(&self, now: DateTime<Utc>, zone: &dyn LocalZone) -> Option<Occurrence> {
        let today = zone.to_local(now).date();
        // An occurrence open now started today or, crossing midnight, yesterday.
        [today.pred_opt(), Some(today)]
            .into_iter()
            .flatten()
            .filter_map(|d| self.occurrence_on(d, zone))
            .find(|o| o.contains(now))
    }

    /// The occurrence open at `now`, or else the next one to open.
    pub fn next_occurrence(&self, now: DateTime<Utc>, zone: &dyn LocalZone) -> Option<Occurrence> {
        if let Some(open) = self.current(now, zone) {
            return Some(open);
        }
        let today = zone.to_local(now).date();
        // Eight days covers every weekday plus an occurrence emptied by DST.
        (0..=8)
            .filter_map(|n| today.checked_add_signed(Duration::days(n)))
            .filter_map(|d| self.occurrence_on(d, zone))
            .find(|o| o.start > now)
    }

    /// The occurrence to run now: open, and not already run. `last_run` is the key of the last
    /// occurrence that ran; a clock jumping backwards cannot run one twice.
    pub fn due(
        &self,
        now: DateTime<Utc>,
        zone: &dyn LocalZone,
        last_run: Option<OccurrenceKey>,
    ) -> Option<Occurrence> {
        self.current(now, zone)
            .filter(|o| last_run.is_none_or(|k| o.key > k))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// A fixed instant.
    pub(crate) struct FixedClock(pub DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    /// A zone at UTC+1, on UTC+2 between 2026-03-29 01:00Z and 2026-10-25 01:00Z: local
    /// 02:00–03:00 is skipped on 2026-03-29 and repeated on 2026-10-25.
    pub(crate) struct DstZone;

    pub(crate) fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    impl DstZone {
        fn offset_hours(utc_t: DateTime<Utc>) -> i64 {
            if utc_t >= utc("2026-03-29T01:00:00Z") && utc_t < utc("2026-10-25T01:00:00Z") {
                2
            } else {
                1
            }
        }
    }

    impl LocalZone for DstZone {
        fn to_local(&self, u: DateTime<Utc>) -> NaiveDateTime {
            u.naive_utc() + Duration::hours(Self::offset_hours(u))
        }

        fn from_local(&self, local: NaiveDateTime) -> LocalResult<DateTime<Utc>> {
            let mut found: Vec<DateTime<Utc>> = [2, 1]
                .into_iter()
                .map(|h| (local - Duration::hours(h)).and_utc())
                .filter(|u| self.to_local(*u) == local)
                .collect();
            found.sort();
            match found.as_slice() {
                [] => LocalResult::None,
                [one] => LocalResult::Single(*one),
                [a, b] => LocalResult::Ambiguous(*a, *b),
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{DstZone, FixedClock, utc};
    use super::*;

    fn w(days: &[Weekday], start: &str, end: &str) -> Window {
        Window {
            days: days.to_vec(),
            start: start.parse().unwrap(),
            end: end.parse().unwrap(),
        }
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn fake_zone_has_the_gap_and_the_repeat() {
        let z = DstZone;
        let gap = date("2026-03-29").and_hms_opt(2, 30, 0).unwrap();
        assert_eq!(z.from_local(gap), LocalResult::None);
        let rep = date("2026-10-25").and_hms_opt(2, 30, 0).unwrap();
        assert_eq!(
            z.from_local(rep),
            LocalResult::Ambiguous(utc("2026-10-25T00:30:00Z"), utc("2026-10-25T01:30:00Z"))
        );
    }

    #[test]
    fn start_in_the_skipped_hour_starts_after_the_gap() {
        let o = w(&Weekday::ALL, "02:30", "04:00")
            .occurrence_on(date("2026-03-29"), &DstZone)
            .unwrap();
        assert_eq!(o.key, OccurrenceKey(date("2026-03-29")));
        // 03:00 local summer time, the first valid instant.
        assert_eq!(o.start, utc("2026-03-29T01:00:00Z"));
        assert_eq!(o.end, utc("2026-03-29T02:00:00Z"));
    }

    #[test]
    fn ambiguous_start_takes_the_earliest_instant() {
        let o = w(&Weekday::ALL, "02:30", "04:00")
            .occurrence_on(date("2026-10-25"), &DstZone)
            .unwrap();
        assert_eq!(o.start, utc("2026-10-25T00:30:00Z"));
        assert_eq!(o.end, utc("2026-10-25T03:00:00Z"));
    }

    #[test]
    fn window_across_midnight_on_both_dst_days() {
        let win = w(&Weekday::ALL, "23:00", "01:00");
        let spring = win.occurrence_on(date("2026-03-28"), &DstZone).unwrap();
        assert_eq!(spring.key, OccurrenceKey(date("2026-03-28")));
        assert_eq!(
            (spring.start, spring.end),
            (utc("2026-03-28T22:00:00Z"), utc("2026-03-29T00:00:00Z"))
        );
        let fall = win.occurrence_on(date("2026-10-24"), &DstZone).unwrap();
        assert_eq!(
            (fall.start, fall.end),
            (utc("2026-10-24T21:00:00Z"), utc("2026-10-24T23:00:00Z"))
        );
        // Open just after midnight, keyed by the day it started.
        let now = utc("2026-03-28T23:30:00Z");
        assert_eq!(
            win.current(now, &DstZone).unwrap().key,
            OccurrenceKey(date("2026-03-28"))
        );
    }

    #[test]
    fn occurrence_emptied_by_the_gap_is_skipped_for_the_next() {
        // 02:10–02:50 lies wholly in the skipped hour: start and end both resolve to 03:00.
        let win = w(&Weekday::ALL, "02:10", "02:50");
        assert_eq!(win.occurrence_on(date("2026-03-29"), &DstZone), None);
        let next = win
            .next_occurrence(utc("2026-03-28T12:00:00Z"), &DstZone)
            .unwrap();
        assert_eq!(next.key, OccurrenceKey(date("2026-03-30")));
        assert_eq!(next.start, utc("2026-03-30T00:10:00Z"));
    }

    #[test]
    fn clock_jumping_back_inside_a_window_does_not_run_it_twice() {
        let win = w(&Weekday::ALL, "02:30", "04:00");
        let first = FixedClock(utc("2026-10-25T02:00:00Z"));
        let ran = win.due(first.now(), &DstZone, None).unwrap();
        assert_eq!(ran.key, OccurrenceKey(date("2026-10-25")));
        // Back an hour: still inside the same occurrence, already run.
        let back = FixedClock(utc("2026-10-25T01:00:00Z"));
        assert!(win.current(back.now(), &DstZone).is_some());
        assert_eq!(win.due(back.now(), &DstZone, Some(ran.key)), None);
        // The next day's occurrence runs.
        let next = utc("2026-10-26T02:00:00Z");
        assert_eq!(
            win.due(next, &DstZone, Some(ran.key)).unwrap().key,
            OccurrenceKey(date("2026-10-26"))
        );
    }

    #[test]
    fn next_occurrence_respects_days_and_reports_an_open_window() {
        // 2026-10-28 is a Wednesday.
        let win = w(&[Weekday::Mon], "03:00", "05:00");
        let next = win
            .next_occurrence(utc("2026-10-28T12:00:00Z"), &DstZone)
            .unwrap();
        assert_eq!(next.key, OccurrenceKey(date("2026-11-02")));
        assert_eq!(next.start, utc("2026-11-02T02:00:00Z"));
        let open = win
            .next_occurrence(utc("2026-11-02T03:00:00Z"), &DstZone)
            .unwrap();
        assert!(open.contains(utc("2026-11-02T03:00:00Z")));
        assert_eq!(win.current(utc("2026-11-02T04:00:00Z"), &DstZone), None);
    }

    #[test]
    fn validation_and_wire_shape() {
        assert_eq!(
            w(&[], "03:00", "05:00").validate(),
            Err(WindowError::NoDays)
        );
        assert_eq!(
            w(&[Weekday::Mon, Weekday::Mon], "03:00", "05:00").validate(),
            Err(WindowError::DuplicateDay)
        );
        assert_eq!(
            w(&[Weekday::Mon], "03:00", "03:00").validate(),
            Err(WindowError::EmptyOrFullDay)
        );
        assert_eq!(w(&[Weekday::Mon], "00:00", "23:59").validate(), Ok(()));
        for bad in ["24:00", "3:00", "03:60", "03-00", "0a:00", ""] {
            assert!(bad.parse::<HhMm>().is_err(), "{bad}");
        }
        let json = r#"{"days":["mon","sun"],"start":"23:00","end":"01:00"}"#;
        let parsed: Window = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, w(&[Weekday::Mon, Weekday::Sun], "23:00", "01:00"));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
        assert!(
            serde_json::from_str::<Window>(
                r#"{"days":["mon"],"start":"03:00","end":"05:00","x":1}"#
            )
            .is_err()
        );
    }
}
