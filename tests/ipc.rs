//! The Unix socket boundary: one line per command, one daemon at a time, stale sockets
//! recovered, and a short-lived trigger client that starts the daemon when absent.

mod support;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gnome_annotation::capture::{CaptureError, CaptureOutcome, ScreenshotPortal};
use gnome_annotation::daemon::Daemon;
use gnome_annotation::ipc::{IpcError, Reply, SocketServer, TriggerClient, TriggerError};
use support::TempDir;
use support::fakes::{FakePortal, RecordingHost, RecordingNotifier};

fn daemon(outcomes: Vec<Result<CaptureOutcome, CaptureError>>) -> (Arc<Daemon>, Arc<RecordingHost>) {
    let host = Arc::new(RecordingHost::default());
    let daemon = Arc::new(Daemon::new(
        Arc::new(FakePortal::new(outcomes)),
        Arc::new(RecordingNotifier::default()),
        host.clone(),
    ));
    (daemon, host)
}

fn send(socket: &Path, command: &str) -> String {
    let mut stream = UnixStream::connect(socket).expect("connect to the daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(command.as_bytes()).unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).unwrap();
    reply
}

fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

#[test]
fn a_capture_command_is_accepted_and_opens_the_editor() {
    let dir = TempDir::new("ipc-capture");
    let socket = dir.path().join("gnome-annotation.sock");
    let (daemon, host) = daemon(vec![Ok(CaptureOutcome::Captured(PathBuf::from(
        "/tmp/socket.png",
    )))]);
    let _server = SocketServer::bind_at(socket.clone(), daemon).expect("bind the socket");

    assert_eq!(send(&socket, "capture\n"), "ok\n");
    assert!(
        wait_until(Duration::from_secs(2), || !host.opened().is_empty()),
        "the accepted capture opens the editor"
    );
    assert_eq!(host.opened(), [PathBuf::from("/tmp/socket.png")]);

    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "only the user can talk to the daemon");
}

#[test]
fn a_second_server_refuses_to_start_while_the_first_holds_the_lock() {
    let dir = TempDir::new("ipc-lock");
    let socket = dir.path().join("gnome-annotation.sock");
    let (first, _) = daemon(vec![Ok(CaptureOutcome::Cancelled)]);
    let server = SocketServer::bind_at(socket.clone(), first).expect("first daemon binds");

    let (second, _) = daemon(vec![Ok(CaptureOutcome::Cancelled)]);
    let error = SocketServer::bind_at(socket.clone(), second).unwrap_err();
    assert!(
        matches!(error, IpcError::AlreadyRunning),
        "the second daemon is refused: {error}"
    );

    drop(server);
    let (third, _) = daemon(vec![Ok(CaptureOutcome::Cancelled)]);
    assert!(
        SocketServer::bind_at(socket, third).is_ok(),
        "the lock is released when the first daemon exits"
    );
}

#[test]
fn a_stale_socket_is_replaced() {
    let dir = TempDir::new("ipc-stale");
    let socket = dir.path().join("gnome-annotation.sock");
    // A daemon that died without unlinking its socket leaves the path behind but no listener.
    drop(UnixListener::bind(&socket).expect("leave a stale socket"));
    assert!(socket.exists());

    let (daemon, _) = daemon(vec![Ok(CaptureOutcome::Cancelled)]);
    let _server = SocketServer::bind_at(socket.clone(), daemon).expect("stale socket recovers");
    assert_eq!(send(&socket, "capture\n"), "ok\n");
}

#[test]
fn an_unknown_command_is_rejected_without_capturing() {
    let dir = TempDir::new("ipc-unknown");
    let socket = dir.path().join("gnome-annotation.sock");
    let (daemon, host) = daemon(vec![Ok(CaptureOutcome::Captured(PathBuf::from(
        "/tmp/no.png",
    )))]);
    let _server = SocketServer::bind_at(socket.clone(), daemon).expect("bind the socket");

    let reply = send(&socket, "open\n");
    assert!(
        reply.starts_with("error"),
        "later commands are not silently accepted: {reply:?}"
    );
    assert!(host.opened().is_empty());
}

#[test]
fn a_capture_command_while_a_capture_is_in_flight_reports_busy() {
    struct GatePortal {
        open: Mutex<bool>,
        ready: Condvar,
    }
    impl ScreenshotPortal for GatePortal {
        fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.ready.wait(open).unwrap();
            }
            Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/gated.png")))
        }
    }

    let dir = TempDir::new("ipc-busy");
    let socket = dir.path().join("gnome-annotation.sock");
    let gate = Arc::new(GatePortal {
        open: Mutex::new(false),
        ready: Condvar::new(),
    });
    let host = Arc::new(RecordingHost::default());
    let daemon = Arc::new(Daemon::new(
        gate.clone(),
        Arc::new(RecordingNotifier::default()),
        host.clone(),
    ));
    let _server = SocketServer::bind_at(socket.clone(), daemon.clone()).expect("bind");

    assert_eq!(
        daemon.request(),
        gnome_annotation::daemon::RequestOutcome::Accepted
    );
    assert_eq!(send(&socket, "capture\n"), "busy\n");

    *gate.open.lock().unwrap() = true;
    gate.ready.notify_all();
    assert!(wait_until(Duration::from_secs(2), || !host.opened().is_empty()));
    assert_eq!(host.opened(), [PathBuf::from("/tmp/gated.png")]);
}

#[test]
fn the_trigger_client_reports_acceptance_from_a_running_daemon() {
    let dir = TempDir::new("ipc-client");
    let socket = dir.path().join("gnome-annotation.sock");
    let (daemon, host) = daemon(vec![Ok(CaptureOutcome::Captured(PathBuf::from(
        "/tmp/client.png",
    )))]);
    let _server = SocketServer::bind_at(socket.clone(), daemon).expect("bind the socket");

    let client = TriggerClient::new(socket, PathBuf::from("/nonexistent/gnome-annotation"));
    assert_eq!(client.run().unwrap(), Reply::Accepted);
    assert!(wait_until(Duration::from_secs(2), || !host.opened().is_empty()));
}

#[test]
fn the_trigger_client_starts_the_daemon_when_absent_and_exits_after_acceptance() {
    let dir = TempDir::new("ipc-spawn");
    let socket = dir.path().join("gnome-annotation.sock");
    let exe = PathBuf::from("/opt/gnome-annotation");
    let started = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let spawned = started.clone();
    let listener_socket = socket.clone();
    let client = TriggerClient::new(socket.clone(), exe.clone());

    let start = Instant::now();
    let reply = client
        .run_with(move |path| {
            spawned.lock().unwrap().push(path.to_owned());
            std::thread::spawn(move || {
                // The daemon takes a moment to bind; the client retries.
                std::thread::sleep(Duration::from_millis(60));
                let listener = UnixListener::bind(&listener_socket).unwrap();
                let (mut stream, _) = listener.accept().unwrap();
                let mut command = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut command)
                    .unwrap();
                assert_eq!(command, "capture\n");
                stream.write_all(b"ok\n").unwrap();
            });
            Ok(())
        })
        .unwrap();

    assert_eq!(reply, Reply::Accepted);
    assert_eq!(*started.lock().unwrap(), [exe]);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "the client returns as soon as the daemon accepts"
    );
}

#[test]
fn the_trigger_client_reports_when_no_daemon_can_be_reached() {
    let dir = TempDir::new("ipc-nodaemon");
    let socket = dir.path().join("gnome-annotation.sock");
    let client = TriggerClient::new(socket.clone(), PathBuf::from("/opt/gnome-annotation"));

    let start = Instant::now();
    let error = client.run_with(|_| Ok(())).unwrap_err();
    assert!(
        matches!(error, TriggerError::Unreachable { .. }),
        "a missing daemon is reported: {error}"
    );
    assert!(
        error.to_string().contains("gnome-annotation.sock"),
        "the error names the socket: {error}"
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_bad_reply_is_an_error_and_does_not_start_another_daemon() {
    let dir = TempDir::new("ipc-badreply");
    let socket = dir.path().join("gnome-annotation.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut command = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut command)
            .unwrap();
        stream.write_all(b"nope\n").unwrap();
    });

    let client = TriggerClient::new(socket, PathBuf::from("/opt/gnome-annotation"));
    let error = client
        .run_with(|_| panic!("a reachable daemon with a bad reply must not spawn another"))
        .unwrap_err();
    assert!(matches!(error, TriggerError::Protocol(_)), "{error}");
}
