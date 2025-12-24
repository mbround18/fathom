use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NotificationLevel, Route, SystemEvent,
    section_keys,
};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn emit(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    route: Route,
    correlation: Correlation,
    event: BackendEvent,
) {
    let next = seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let _ = sender.send(BackendEventEnvelope::new(next, route, correlation, event));
}

pub(super) fn notify_info(seq: &AtomicU64, sender: &Sender<BackendEventEnvelope>, message: String) {
    emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::Notification {
            level: NotificationLevel::Info,
            message,
        }),
    );
}

pub(super) fn notify_error(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    message: String,
) {
    emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::Notification {
            level: NotificationLevel::Error,
            message,
        }),
    );
}
