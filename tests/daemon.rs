//! The resident daemon's state machine, driven through its public seam with a fake portal, a
//! recording notifier, and a fake UI host. No Rune, no display, no socket.

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use gnome_annotation::capture::{CaptureError, CaptureOutcome};
use gnome_annotation::daemon::{Daemon, TriggerOutcome};
use support::fakes::{FakePortal, RecordingHost, RecordingNotifier};

type TestDaemon = (
    Arc<Daemon>,
    Arc<FakePortal>,
    Arc<RecordingNotifier>,
    Arc<RecordingHost>,
);

fn daemon(outcomes: Vec<Result<CaptureOutcome, CaptureError>>) -> TestDaemon {
    let portal = Arc::new(FakePortal::new(outcomes));
    let notifier = Arc::new(RecordingNotifier::default());
    let host = Arc::new(RecordingHost::default());
    let daemon = Arc::new(Daemon::new(portal.clone(), notifier.clone(), host.clone()));
    (daemon, portal, notifier, host)
}

#[test]
fn a_captured_file_opens_the_editor() {
    let (daemon, portal, notifier, host) = daemon(vec![Ok(CaptureOutcome::Captured(
        PathBuf::from("/tmp/shot.png"),
    ))]);

    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/shot.png"))
    );
    assert_eq!(host.opened(), [PathBuf::from("/tmp/shot.png")]);
    assert_eq!(portal.calls(), 1);
    assert!(notifier.messages().is_empty());
}

#[test]
fn a_cancelled_capture_opens_no_editor() {
    let (daemon, portal, notifier, host) = daemon(vec![Ok(CaptureOutcome::Cancelled)]);

    assert_eq!(daemon.trigger(), TriggerOutcome::Cancelled);
    assert!(host.opened().is_empty());
    assert_eq!(portal.calls(), 1);
    assert!(notifier.messages().is_empty());
}

#[test]
fn a_portal_failure_notifies_and_stays_idle() {
    let (daemon, portal, notifier, host) = daemon(vec![
        Err(CaptureError("no screenshot portal".into())),
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/retry.png"))),
    ]);

    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Failed("no screenshot portal".into())
    );
    assert!(host.opened().is_empty());
    let messages = notifier.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0, "GNOME Annotation");
    assert!(
        messages[0].1.contains("no screenshot portal"),
        "the notification carries the error: {messages:?}"
    );

    // The failure leaves the daemon idle, so the next trigger captures again.
    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/retry.png"))
    );
    assert_eq!(portal.calls(), 2);
}

#[test]
fn a_capture_that_cannot_be_loaded_notifies_and_stays_idle() {
    let (daemon, portal, notifier, host) = daemon(vec![
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/broken.png"))),
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/good.png"))),
    ]);
    host.fail_open(Some("cannot decode /tmp/broken.png"));

    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Failed("cannot decode /tmp/broken.png".into())
    );
    assert!(host.opened().is_empty());
    assert_eq!(notifier.messages().len(), 1);

    host.fail_open(None);
    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/good.png"))
    );
    assert_eq!(portal.calls(), 2);
}

#[test]
fn a_second_trigger_while_editing_focuses_and_skips_the_portal() {
    let (daemon, portal, _notifier, host) = daemon(vec![
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/one.png"))),
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/two.png"))),
    ]);

    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/one.png"))
    );
    assert_eq!(daemon.trigger(), TriggerOutcome::Focused);

    assert_eq!(host.focused(), 1);
    assert_eq!(portal.calls(), 1, "the open editor keeps its capture");
    assert_eq!(host.opened(), [PathBuf::from("/tmp/one.png")]);
}

#[test]
fn closing_the_editor_returns_the_daemon_to_idle() {
    let (daemon, portal, _notifier, host) = daemon(vec![
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/one.png"))),
        Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/two.png"))),
    ]);

    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/one.png"))
    );
    daemon.editor_closed();
    assert_eq!(
        daemon.trigger(),
        TriggerOutcome::Opened(PathBuf::from("/tmp/two.png"))
    );
    assert_eq!(portal.calls(), 2);
    assert_eq!(host.focused(), 0);
}

#[test]
fn a_trigger_without_an_editor_is_accepted_asynchronously() {
    let (daemon, portal, _notifier, host) = daemon(vec![Ok(CaptureOutcome::Captured(
        PathBuf::from("/tmp/async.png"),
    ))]);

    assert_eq!(
        daemon.request(),
        gnome_annotation::daemon::RequestOutcome::Accepted
    );
    // The socket thread replies before the worker finishes; poll for the open.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while host.opened().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(host.opened(), [PathBuf::from("/tmp/async.png")]);
    assert_eq!(portal.calls(), 1);

    // The editor counts as open now, so a second request focuses.
    assert_eq!(
        daemon.request(),
        gnome_annotation::daemon::RequestOutcome::Accepted
    );
    assert_eq!(host.focused(), 1);
    assert_eq!(portal.calls(), 1);
}
