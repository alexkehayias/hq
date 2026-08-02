//! Channel: pub/sub stream of string events over a Unix domain socket.
//!
//! ## Security model
//!
//! - On Linux, the publisher binds an **abstract socket** (path starts with
//!   `\0`) — no filesystem exposure, so symlink attacks are impossible and
//!   the kernel cleans up the name when the publisher's file descriptor closes.
//! - On macOS and other Unixes, the publisher binds a filesystem path in
//!   `$HQ_STORAGE_PATH/channels/<id>.sock` with 0o700 permissions. Only the
//!   owner can connect.
//! - On every platform, the publisher verifies each incoming connection's
//!   peer credentials (`peer_cred`) and rejects connections from processes
//!   owned by a different user. This is defense in depth on top of file
//!   permissions.
//!
//! ## Trust boundary
//!
//! Unix sockets carry no encryption. The trust boundary is "same user on the
//! same host" — any process owned by the publisher's user can subscribe.
//! Events are visible to any such process; do not send secrets if untrusted
//! local processes share the host.

use anyhow::{anyhow, bail, Result};
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::env;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{stdin, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::warn;

/// Validate a channel ID. Must be non-empty and alphanumeric with dashes/underscores.
///
/// This prevents path traversal (`../`), embedded null bytes, and slash separators
/// from reaching socket path construction.
pub fn validate_channel_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("channel ID cannot be empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("channel ID must be alphanumeric with dashes/underscores only");
    }
    Ok(())
}

/// Construct the socket path for a channel ID.
///
/// On Linux: uses the abstract socket namespace — path bytes start with `\0`,
/// so there is no filesystem exposure and symlink attacks are impossible.
///
/// On macOS/other Unix: filesystem path in `$HQ_STORAGE_PATH/channels/<id>.sock`.
pub fn socket_path(id: &str) -> Result<PathBuf> {
    validate_channel_id(id)?;

    #[cfg(target_os = "linux")]
    {
        let mut bytes = Vec::new();
        bytes.push(0u8); // abstract socket marker
        bytes.extend_from_slice(b"hq-channel-");
        bytes.extend_from_slice(id.as_bytes());
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let storage = env::var("HQ_STORAGE_PATH").unwrap_or("./".to_string());
        Ok(PathBuf::from(storage).join("channels").join(format!("{}.sock", id)))
    }
}

/// RAII guard that removes a socket file on Drop.
///
/// On macOS (filesystem paths), this ensures the `.sock` file is removed even
/// if the publisher panics or exits via Ctrl-C. On Linux (abstract sockets),
/// there is no file to clean up — the kernel handles it when the fd closes.
struct SocketGuard {
    path: Option<PathBuf>,
}

impl SocketGuard {
    fn new_file(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    #[cfg(target_os = "linux")]
    fn no_file() -> Self {
        Self { path: None }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            // Best-effort cleanup; don't panic in Drop.
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Transform a single event. Currently an identity function.
///
/// This is the stub for future transforms that modify or enrich events before
/// they are emitted by the channel.
pub async fn transform(input: String) -> String {
    input
}

/// Build a stream of events read from stdin, with the channel transform applied.
///
/// Each line read from stdin becomes one event. The stream ends when stdin
/// reaches EOF.
pub fn event_stream() -> BoxStream<'static, String> {
    let s = stream! {
        let stdin = stdin();
        let mut reader = BufReader::new(stdin);
        let mut buf = String::new();
        loop {
            buf.clear();
            match reader.read_line(&mut buf).await {
                Ok(0) => return,
                Ok(_) => {
                    let line = buf.strip_suffix('\n').unwrap_or(&buf).to_string();
                    yield transform(line).await;
                }
                Err(_) => return,
            }
        }
    };
    Box::pin(s)
}

/// Current process UID (for peer authentication).
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Wait for SIGTERM so the publisher can clean up on `kill` (not just Ctrl-C).
pub async fn sigterm() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        }
    }
    #[cfg(not(unix))]
    {
        std::future::pending::<()>().await;
    }
}

/// Bind the publisher's Unix socket, handling stale socket files on macOS.
///
/// On Linux (abstract sockets), `bind` either succeeds or returns EADDRINUSE
/// if another publisher owns the name — there are no stale files to recover.
///
/// On macOS (filesystem paths), `bind` may fail because a previous publisher
/// crashed without cleaning up. We detect this by attempting to connect: if the
/// connection succeeds, another publisher is live (channel in use); if it fails,
/// the socket file is stale and we remove + retry.
async fn bind_socket(path: &Path) -> Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Stale socket? Try connecting to see if a publisher is live.
            match UnixStream::connect(path).await {
                Ok(_) => bail!("channel already in use (socket is live)"),
                Err(_) => {
                    // Stale — remove and retry
                    let _ = tokio::fs::remove_file(path).await;
                    UnixListener::bind(path)
                        .map_err(|e| anyhow!("failed to bind after stale cleanup: {}", e))
                }
            }
        }
        Err(e) => bail!("failed to bind socket: {}", e),
    }
}

/// Run the channel publisher.
///
/// Binds a Unix domain socket at `socket_path(id)`, accepts subscriber
/// connections (verifying peer UID matches our own), and broadcasts each stdin
/// event to all active subscribers as a newline-delimited string.
pub async fn run(id: &str) -> Result<()> {
    let path = socket_path(id)?;

    // Ensure channels dir exists with 0o700 perms (macOS filesystem paths).
    #[cfg(not(target_os = "linux"))]
    {
        let storage = env::var("HQ_STORAGE_PATH").unwrap_or("./".to_string());
        let channels_dir = PathBuf::from(&storage).join("channels");
        tokio::fs::create_dir_all(&channels_dir).await?;
        let perms = std::fs::Permissions::from_mode(0o700);
        tokio::fs::set_permissions(&channels_dir, perms).await?;
    }

    let listener = bind_socket(&path).await?;

    // Set 0o700 perms on the socket file (macOS).
    #[cfg(not(target_os = "linux"))]
    {
        let perms = std::fs::Permissions::from_mode(0o700);
        tokio::fs::set_permissions(&path, perms).await?;
    }

    // RAII cleanup guard: removes the socket file on Drop (macOS).
    #[cfg(target_os = "linux")]
    let _guard = SocketGuard::no_file();
    #[cfg(not(target_os = "linux"))]
    let _guard = SocketGuard::new_file(path.clone());

    // Subscriber registry: one mpsc::Sender per connected subscriber.
    type Subs = Arc<Mutex<Vec<mpsc::Sender<String>>>>;
    let subscribers: Subs = Arc::new(Mutex::new(Vec::new()));

    // Accept loop (spawned task). For each accepted connection:
    // - Verify peer UID matches our UID (reject if not).
    // - Register a bounded mpsc::Sender; spawn a writer task that drains it
    //   and writes `msg + \n` to the subscriber's stream.
    let accept_subscribers = subscribers.clone();
    let our_uid = current_uid();
    let accept_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    // Peer authentication: verify same UID.
                    match stream.peer_cred() {
                        Ok(creds) if creds.uid() == our_uid => {
                            let (tx, mut rx) = mpsc::channel::<String>(128);
                            accept_subscribers.lock().unwrap().push(tx.clone());
                            let mut stream = stream;
                            tokio::spawn(async move {
                                while let Some(msg) = rx.recv().await {
                                    if stream.write_all(msg.as_bytes()).await.is_err()
                                        || stream
                                            .write_all(b"\n")
                                            .await
                                            .is_err()
                                    {
                                        break;
                                    }
                                }
                            });
                        }
                        Ok(_) => warn!("rejected subscriber: wrong UID"),
                        Err(e) => warn!("failed to read peer creds: {}", e),
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Read stdin, broadcast each event to all subscribers.
    let mut events = event_stream();
    let main_loop = async {
        while let Some(event) = events.next().await {
            // Clone senders out of the mutex so we don't hold it during
            // `send().await` (which blocks on backpressure).
            let subs: Vec<mpsc::Sender<String>> = subscribers.lock().unwrap().clone();
            for tx in subs.iter() {
                let _ = tx.send(event.clone()).await;
            }
        }
    };

    // Ctrl-C / SIGTERM handling: race main loop against signals so the
    // socket file is cleaned up via `_guard`'s Drop even on interrupt or
    // `kill` (SIGTERM). SIGKILL cannot be caught — that's the only case
    // where cleanup is skipped (stale socket recovery handles it on next bind).
    tokio::select! {
        _ = main_loop => {}
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nShutting down...");
        }
        _ = sigterm() => {
            eprintln!("\nShutting down (SIGTERM)...");
        }
    }

    // Cleanup: stop accept task, drop subscribers (they see EOF).
    accept_handle.abort();

    Ok(())
}