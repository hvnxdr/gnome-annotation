//! Unix socket transport between the short-lived trigger client and the resident daemon.
//! One line per connection, one daemon at a time through a lock file, and a stale socket is
//! removed only after the lock proves no daemon owns it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::daemon::{Daemon, RequestOutcome};

const SOCKET_NAME: &str = "gnome-annotation.sock";
const LOCK_NAME: &str = "gnome-annotation.lock";
const RETRIES: usize = 20;
const RETRY_DELAY: Duration = Duration::from_millis(25);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub enum IpcError {
    AlreadyRunning,
    InvalidPath(PathBuf),
    Path { path: PathBuf, error: io::Error },
    Io(io::Error),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => f.write_str("another gnome-annotation daemon is already running"),
            Self::InvalidPath(path) => write!(f, "{} has no parent directory", path.display()),
            Self::Path { path, error } => write!(f, "cannot use {}: {error}", path.display()),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for IpcError {}

/// `$XDG_RUNTIME_DIR`, or a private per-user directory in the temp directory when unset.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let dir = std::env::temp_dir().join(format!("gnome-annotation-{}", unsafe {
        libc::getuid()
    }));
    let _ = fs::create_dir_all(&dir);
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    dir
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_NAME)
}

/// The listening daemon. Dropping it stops the accept loop and removes the socket, while the
/// lock file stays (empty) for the next run.
pub struct SocketServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    _lock: File,
}

impl SocketServer {
    pub fn bind(daemon: Arc<Daemon>) -> Result<Self, IpcError> {
        Self::bind_at(socket_path(), daemon)
    }

    pub fn bind_at(path: PathBuf, daemon: Arc<Daemon>) -> Result<Self, IpcError> {
        let parent = path
            .parent()
            .ok_or_else(|| IpcError::InvalidPath(path.clone()))?;
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|error| IpcError::Path {
                path: parent.to_owned(),
                error,
            })?;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
        let lock = acquire_lock(parent)?;

        // The lock is ours, so a socket left here belongs to a daemon that is gone.
        if path.exists() {
            fs::remove_file(&path).map_err(|error| IpcError::Path {
                path: path.clone(),
                error,
            })?;
        }
        let listener = UnixListener::bind(&path).map_err(|error| IpcError::Path {
            path: path.clone(),
            error,
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            IpcError::Path {
                path: path.clone(),
                error,
            }
        })?;
        listener.set_nonblocking(true).map_err(IpcError::Io)?;

        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let join = std::thread::Builder::new()
            .name("gnome-annotation-ipc".into())
            .spawn(move || serve(listener, daemon, server_stop))
            .map_err(IpcError::Io)?;
        Ok(Self {
            path,
            stop,
            join: Some(join),
            _lock: lock,
        })
    }
}

impl std::fmt::Debug for SocketServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketServer")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_lock(dir: &Path) -> Result<File, IpcError> {
    let path = dir.join(LOCK_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| IpcError::Path {
            path: path.clone(),
            error,
        })?;
    let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Err(IpcError::AlreadyRunning);
        }
        return Err(IpcError::Io(error));
    }
    Ok(file)
}

fn serve(listener: UnixListener, daemon: Arc<Daemon>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle(stream, &daemon),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                eprintln!("gnome-annotation: cannot accept a trigger: {error}");
                break;
            }
        }
    }
}

fn handle(mut stream: UnixStream, daemon: &Arc<Daemon>) {
    let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut command = String::new();
    if BufReader::new(reader).read_line(&mut command).is_err() {
        return;
    }
    let reply = match command.trim() {
        "capture" => match daemon.request() {
            RequestOutcome::Accepted => "ok\n",
            RequestOutcome::Busy => "busy\n",
        },
        "" => "error empty command\n",
        other => {
            let _ = stream.write_all(format!("error unknown command: {other}\n").as_bytes());
            return;
        }
    };
    let _ = stream.write_all(reply.as_bytes());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Accepted,
    Busy,
}

#[derive(Debug)]
pub enum TriggerError {
    Unreachable { socket: PathBuf, detail: String },
    Spawn { exe: PathBuf, error: io::Error },
    Protocol(String),
}

impl std::fmt::Display for TriggerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable { socket, detail } => {
                write!(f, "cannot reach the daemon at {}", socket.display())?;
                if !detail.is_empty() {
                    write!(f, ": {detail}")?;
                }
                Ok(())
            }
            Self::Spawn { exe, error } => {
                write!(f, "cannot start {}: {error}", exe.display())
            }
            Self::Protocol(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TriggerError {}

enum SendError {
    Unreachable(String),
    Protocol(String),
}

/// The short-lived Print client: connect, send `capture`, exit once the daemon accepts.
pub struct TriggerClient {
    socket: PathBuf,
    exe: PathBuf,
}

impl TriggerClient {
    pub fn new(socket: PathBuf, exe: PathBuf) -> Self {
        Self { socket, exe }
    }

    pub fn run(&self) -> Result<Reply, TriggerError> {
        self.run_with(spawn_detached)
    }

    /// `spawn` starts a detached daemon; tests replace it with an in-process listener.
    pub fn run_with(
        &self,
        spawn: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<Reply, TriggerError> {
        match send(&self.socket) {
            Ok(reply) => return Ok(reply),
            Err(SendError::Protocol(message)) => return Err(TriggerError::Protocol(message)),
            Err(SendError::Unreachable(_)) => {}
        }
        spawn(&self.exe).map_err(|error| TriggerError::Spawn {
            exe: self.exe.clone(),
            error,
        })?;
        let mut detail = String::new();
        for _ in 0..RETRIES {
            std::thread::sleep(RETRY_DELAY);
            match send(&self.socket) {
                Ok(reply) => return Ok(reply),
                Err(SendError::Protocol(message)) => return Err(TriggerError::Protocol(message)),
                Err(SendError::Unreachable(message)) => detail = message,
            }
        }
        Err(TriggerError::Unreachable {
            socket: self.socket.clone(),
            detail,
        })
    }
}

fn send(socket: &Path) -> Result<Reply, SendError> {
    let mut stream = UnixStream::connect(socket).map_err(|error| SendError::Unreachable(error.to_string()))?;
    stream
        .set_read_timeout(Some(CONNECTION_TIMEOUT))
        .map_err(|error| SendError::Unreachable(error.to_string()))?;
    stream
        .write_all(b"capture\n")
        .map_err(|error| SendError::Unreachable(error.to_string()))?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|error| SendError::Unreachable(error.to_string()))?;
    match reply.trim() {
        "ok" => Ok(Reply::Accepted),
        "busy" => Ok(Reply::Busy),
        other => Err(SendError::Protocol(format!(
            "the daemon replied with something unexpected: {other:?}"
        ))),
    }
}

/// Start the same binary in daemon mode with null stdio, out of the client's session.
pub fn spawn_detached(exe: &Path) -> io::Result<()> {
    let mut command = Command::new(exe);
    command
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // `pre_exec` runs after fork and before exec; setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop(child);
    Ok(())
}
