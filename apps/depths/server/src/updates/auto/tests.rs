//! The Auto queue (T059, T064) with a fake clock and a UTC zone. DST resolution of windows is
//! covered by depths-core's window tests with its DST zone.

use std::sync::Mutex;

use chrono::NaiveDate;
use depths_core::updates::window::Clock;
use depths_protocol::{
    EffectivePolicy, PolicySource, Provider, ServiceUpdate, TimeOfDay, WatchtowerAnswer, Weekday,
};

use super::*;
use crate::updates::store::{StoredActor, StoredPolicy, WatchtowerAck};
use crate::updates::watchtower::Seen;

struct TestClock(Mutex<DateTime<Utc>>);

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

impl TestClock {
    fn set(&self, t: DateTime<Utc>) {
        *self.0.lock().unwrap() = t;
    }
}

struct Utc0;

impl LocalZone for Utc0 {
    fn to_local(&self, utc: DateTime<Utc>) -> NaiveDateTime {
        utc.naive_utc()
    }
    fn from_local(&self, local: NaiveDateTime) -> LocalResult<DateTime<Utc>> {
        LocalResult::Single(local.and_utc())
    }
}

/// 2026-10-26 is a Monday.
fn at(day: u32, h: u32, m: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(h, m, 0)
        .unwrap()
        .and_utc()
}

fn window(days: &[Weekday], start: &str, end: &str) -> MaintenanceWindow {
    MaintenanceWindow {
        days: days.to_vec(),
        start: TimeOfDay::new(start).unwrap(),
        end: TimeOfDay::new(end).unwrap(),
    }
}

fn actor(subject: &str) -> Actor {
    Actor {
        provider: Provider::Github,
        subject: subject.into(),
        username: subject.into(),
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    u: Arc<Updates>,
}

fn fx_with(auto: bool, garbage: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = crate::updates::tests::config(dir.path(), "on");
    cfg.updates.auto = auto;
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    if garbage {
        std::fs::write(cfg.data_dir.join(crate::updates::store::UPDATES_FILE), "{").unwrap();
    }
    let clock = Arc::new(TestClock(Mutex::new(at(26, 3, 30))));
    let u = Updates::open(&cfg, clock.clone(), || {
        Arc::new(crate::updates::tests::NoNetwork)
    })
    .unwrap()
    .unwrap();
    Fx {
        _dir: dir,
        clock,
        u,
    }
}

fn fx() -> Fx {
    fx_with(true, false)
}

impl Fx {
    /// Host window: every day 03:00 to 05:00 (the default).
    fn policy(&self, stack: &str, policy: UpdatePolicy, by: &str, w: Option<MaintenanceWindow>) {
        self.u
            .store()
            .update_decisions(|d| {
                d.policies.insert(
                    stack.into(),
                    StoredPolicy {
                        policy,
                        window: w,
                        set_by: StoredActor::from(actor(by)),
                        set_at: "2026-10-01T00:00:00Z".into(),
                    },
                );
            })
            .unwrap();
    }

    fn service(&self, stack: &str, state: UpdateState, effective: UpdatePolicy) {
        self.u
            .store()
            .update_observed(|o| {
                o.services.insert(
                    format!("{stack}/app"),
                    ServiceUpdate {
                        stack: stack.into(),
                        service: "app".into(),
                        reference: "nginx:1".into(),
                        running: None,
                        available: None,
                        state,
                        reason: None,
                        effective: EffectivePolicy {
                            policy: effective,
                            source: PolicySource::StackPolicy,
                            excluded: None,
                            labels: vec![],
                        },
                        last_checked: None,
                        last_outcome: None,
                        replicas: 1,
                    },
                )
            })
            .unwrap();
    }

    fn auto_stack(&self, stack: &str) {
        self.policy(stack, UpdatePolicy::Auto, "admin", None);
        self.service(stack, UpdateState::Available, UpdatePolicy::Auto);
    }

    fn plan(&self, last: &BTreeMap<String, OccurrenceKey>) -> Plan {
        plan(&self.u, self.clock.now(), &Utc0, &only_admin, last)
    }

    fn due(&self) -> Vec<String> {
        self.plan(&BTreeMap::new())
            .due
            .into_iter()
            .map(|d| d.stack)
            .collect()
    }
}

fn only_admin(a: &Actor) -> bool {
    a.subject == "admin" || a.subject == "other-admin"
}

#[test]
fn candidates_are_auto_stacks_with_an_auto_update_available_in_name_order() {
    let f = fx();
    f.auto_stack("c");
    f.auto_stack("a");
    f.policy("b", UpdatePolicy::Notify, "admin", None);
    f.service("b", UpdateState::Available, UpdatePolicy::Notify);
    f.policy("d", UpdatePolicy::Auto, "admin", None);
    f.service("d", UpdateState::Held, UpdatePolicy::Auto);
    f.policy("e", UpdatePolicy::Auto, "admin", None);
    f.service("e", UpdateState::Rejected, UpdatePolicy::Auto);
    f.policy("g", UpdatePolicy::Auto, "admin", None);
    // A label lowered it to Notify.
    f.service("g", UpdateState::Available, UpdatePolicy::Notify);
    f.policy("h", UpdatePolicy::Auto, "admin", None);
    f.service("h", UpdateState::UpToDate, UpdatePolicy::Auto);
    assert_eq!(f.due(), ["a", "c"]);
}

#[test]
fn nothing_starts_outside_the_window_and_one_occurrence_runs_once() {
    let f = fx();
    f.auto_stack("a");
    f.clock.set(at(26, 5, 0));
    assert!(f.due().is_empty(), "05:00 is closed");
    let p = f.plan(&BTreeMap::new());
    assert_eq!(
        p.status.next_window.as_deref(),
        Some("2026-10-27T03:00:00Z")
    );

    f.clock.set(at(26, 4, 0));
    let first = f.plan(&BTreeMap::new());
    assert_eq!(first.due.len(), 1);
    let ran: BTreeMap<_, _> = [("a".to_owned(), first.due[0].occurrence.key)].into();
    assert!(f.plan(&ran).due.is_empty(), "already ran this occurrence");
    f.clock.set(at(27, 3, 0));
    assert_eq!(f.plan(&ran).due.len(), 1, "the next day's opening runs");
}

#[test]
fn a_stack_window_replaces_the_host_window() {
    let f = fx();
    f.policy(
        "a",
        UpdatePolicy::Auto,
        "admin",
        Some(window(&[Weekday::Sat], "22:00", "02:00")),
    );
    f.service("a", UpdateState::Available, UpdatePolicy::Auto);
    f.auto_stack("b");
    // Monday 03:30: the host window is open, a's is not.
    assert_eq!(f.due(), ["b"]);
    // Sunday 01:00: a's Saturday window crosses midnight; the host's is closed.
    f.clock.set(at(25, 1, 0));
    assert_eq!(f.due(), ["a"]);
}

#[test]
fn when_the_window_closes_the_stack_in_progress_finishes_and_the_rest_wait() {
    let f = fx();
    f.auto_stack("a");
    f.auto_stack("b");
    f.auto_stack("c");
    f.clock.set(at(26, 4, 50));
    let mut last = BTreeMap::new();
    let mut ran = Vec::new();
    let clock = f.clock.clone();
    tick(&f.u, &Utc0, &only_admin, &mut last, &mut |s, by| {
        ran.push((s.to_owned(), by.subject.clone()));
        // "a" takes twenty minutes: the window closes at 05:00.
        clock.set(at(26, 5, 10));
    });
    assert_eq!(ran, [("a".to_owned(), "admin".to_owned())]);
    assert!(last.contains_key("a"));
    assert!(!last.contains_key("b"), "b and c wait for the next opening");

    f.clock.set(at(27, 3, 0));
    let mut ran = Vec::new();
    tick(&f.u, &Utc0, &only_admin, &mut last, &mut |s, _| {
        ran.push(s.to_owned())
    });
    assert_eq!(ran, ["a", "b", "c"], "one at a time, in name order");
}

#[test]
fn a_rejected_digest_waits_and_a_newer_one_is_tried() {
    let f = fx();
    f.policy("a", UpdatePolicy::Auto, "admin", None);
    // The scheduler marks a service whose available digest was rolled back as Rejected.
    f.service("a", UpdateState::Rejected, UpdatePolicy::Auto);
    assert!(f.due().is_empty());
    // A newer digest is Available again.
    f.service("a", UpdateState::Available, UpdatePolicy::Auto);
    assert_eq!(f.due(), ["a"]);
}

#[test]
fn auto_off_pauses_everything() {
    let f = fx_with(false, false);
    f.auto_stack("a");
    let p = f.plan(&BTreeMap::new());
    assert_eq!(p.status.paused, Some(AutoPause::Disabled));
    assert!(p.due.is_empty());
    assert!(
        p.status.next_window.is_some(),
        "the page still shows when it would run"
    );
}

#[test]
fn an_unreadable_updates_json_pauses_everything() {
    let f = fx_with(true, true);
    let p = f.plan(&BTreeMap::new());
    assert_eq!(p.status.paused, Some(AutoPause::SettingsUnreadable));
    assert!(p.due.is_empty());
}

#[test]
fn an_unacknowledged_watchtower_pauses_until_answered() {
    let f = fx();
    f.auto_stack("a");
    f.u.set_watchtower(vec![Seen {
        container_id: "wt1".into(),
        image_id: "sha256:w".into(),
        name: "watchtower".into(),
        image: "containrrr/watchtower".into(),
    }]);
    let p = f.plan(&BTreeMap::new());
    assert_eq!(
        p.status.paused,
        Some(AutoPause::WatchtowerRunning {
            containers: vec!["watchtower".into()]
        })
    );
    assert!(p.due.is_empty());
    f.u.store()
        .update_decisions(|d| {
            d.watchtower_acks.push(WatchtowerAck {
                container_id: "wt1".into(),
                image_id: "sha256:w".into(),
                name: "watchtower".into(),
                answer: WatchtowerAnswer::MonitorOnly,
                by: StoredActor::from(actor("admin")),
                at: "2026-10-01T00:00:00Z".into(),
            });
        })
        .unwrap();
    assert_eq!(f.due(), ["a"]);
}

#[test]
fn a_policy_whose_author_is_no_longer_an_admin_is_paused_until_another_admin_saves_it() {
    let f = fx();
    f.policy("a", UpdatePolicy::Auto, "demoted", None);
    f.service("a", UpdateState::Available, UpdatePolicy::Auto);
    f.auto_stack("b");
    let p = f.plan(&BTreeMap::new());
    assert_eq!(p.status.paused_policies, ["a"]);
    assert_eq!(p.due.len(), 1);
    assert_eq!(p.due[0].stack, "b");

    f.policy("a", UpdatePolicy::Auto, "other-admin", None);
    let p = f.plan(&BTreeMap::new());
    assert!(p.status.paused_policies.is_empty());
    assert_eq!(p.due[0].set_by, actor("other-admin"));
}

#[test]
fn the_author_is_checked_again_before_each_stack() {
    let f = fx();
    f.auto_stack("a");
    f.auto_stack("b");
    let demoted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = demoted.clone();
    let is_admin = move |_: &Actor| !flag.load(std::sync::atomic::Ordering::SeqCst);
    let mut ran = Vec::new();
    tick(&f.u, &Utc0, &is_admin, &mut BTreeMap::new(), &mut |s, _| {
        ran.push(s.to_owned());
        demoted.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!(ran, ["a"]);
}

#[test]
fn tick_records_the_status_for_the_page() {
    let f = fx_with(false, false);
    f.auto_stack("a");
    tick(
        &f.u,
        &Utc0,
        &only_admin,
        &mut BTreeMap::new(),
        &mut |_, _| panic!("paused"),
    );
    assert_eq!(f.u.auto_status().paused, Some(AutoPause::Disabled));
}

#[test]
fn protocol_windows_convert_to_core_windows() {
    let w = to_core(&window(&[Weekday::Mon, Weekday::Sun], "22:30", "01:00")).unwrap();
    assert!(w.crosses_midnight());
    assert_eq!(w.days.len(), 2);
}
