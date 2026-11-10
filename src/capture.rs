//! The desktop Screenshot portal. The editor flow only needs the small trait; the live adapter
//! uses zbus on its own current-thread runtime.

use std::collections::HashMap;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy, Variant};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Screenshot";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    Captured(PathBuf),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureError(pub String);

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CaptureError {}

pub trait ScreenshotPortal: Send + Sync {
    fn capture(&self) -> Result<CaptureOutcome, CaptureError>;
}

/// The editor flow's view of a capture: `Some` means open the editor, `None` means the user
/// cancelled and nothing should happen.
pub fn capture_path(portal: &dyn ScreenshotPortal) -> Result<Option<PathBuf>, CaptureError> {
    match portal.capture()? {
        CaptureOutcome::Captured(path) => Ok(Some(path)),
        CaptureOutcome::Cancelled => Ok(None),
    }
}

/// The live portal. One call opens GNOME's selection, window, and screen UI.
pub struct PortalScreenshot;

impl ScreenshotPortal for PortalScreenshot {
    fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
        let started = std::time::Instant::now();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| CaptureError(format!("cannot start the portal runtime: {error}")))?;
        let outcome = runtime.block_on(capture_interactive())?;
        if matches!(outcome, CaptureOutcome::Captured(_)) {
            eprintln!(
                "gnome-annotation: screenshot portal responded after {:.0} ms (includes the time you spent choosing)",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        Ok(outcome)
    }
}

async fn capture_interactive() -> Result<CaptureOutcome, CaptureError> {
    let started = tokio::time::timeout(RPC_TIMEOUT, async {
        let connection = Connection::session().await.map_err(portal_error)?;
        let portal = Proxy::new(&connection, DEST, PATH, INTERFACE)
            .await
            .map_err(portal_error)?;
        let sender = connection
            .unique_name()
            .ok_or_else(|| CaptureError("no session bus name".into()))?
            .trim_start_matches(':')
            .replace('.', "_");
        let token = format!(
            "gnome_annotation_{}_{}",
            std::process::id(),
            TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let request_path = format!("{PATH}/request/{sender}/{token}");
        let mut request = Proxy::new(&connection, DEST, request_path.clone(), REQUEST_INTERFACE)
            .await
            .map_err(portal_error)?;
        // The response can arrive before the method returns, so subscribe first.
        let mut responses = request
            .receive_signal("Response")
            .await
            .map_err(portal_error)?;
        let options = capture_options(&token);
        let handle: OwnedObjectPath = portal
            .call("Screenshot", &("", options))
            .await
            .map_err(portal_error)?;
        if handle.as_str() != request_path {
            // An older portal ignored handle_token and chose its own request path.
            request = Proxy::new(
                &connection,
                DEST,
                handle.as_str().to_owned(),
                REQUEST_INTERFACE,
            )
            .await
            .map_err(portal_error)?;
            responses = request
                .receive_signal("Response")
                .await
                .map_err(portal_error)?;
        }
        Ok::<_, CaptureError>((connection, responses))
    })
    .await
    .map_err(|_| CaptureError("the screenshot portal did not start in time".into()))??;

    let (_connection, mut responses) = started;
    let message = responses
        .next()
        .await
        .ok_or_else(|| CaptureError("the screenshot portal disconnected".into()))?;
    let (status, results): (u32, HashMap<String, OwnedVariant>) = message
        .body()
        .deserialize()
        .map_err(|error| CaptureError(format!("cannot read the portal response: {error}")))?;
    let uri = results
        .get("uri")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok());
    response_outcome(status, uri.as_deref())
}

/// The portal options. Interactive offers GNOME's selection, window, and screen UI; modal false
/// keeps the dialog from blocking the parent, as the parent spec requires.
fn capture_options(token: &str) -> HashMap<&str, Variant<'_>> {
    HashMap::from([
        ("handle_token", Variant::from(token)),
        ("interactive", Variant::from(true)),
        ("modal", Variant::from(false)),
    ])
}

fn portal_error(error: zbus::Error) -> CaptureError {
    use zbus::fdo;
    if let zbus::Error::FDO(fdo_error) = &error
        && matches!(
            **fdo_error,
            fdo::Error::ServiceUnknown(_)
                | fdo::Error::NameHasNoOwner(_)
                | fdo::Error::UnknownInterface(_)
                | fdo::Error::UnknownMethod(_)
                | fdo::Error::UnknownObject(_)
                | fdo::Error::SpawnExecFailed(_)
                | fdo::Error::SpawnForkFailed(_)
                | fdo::Error::SpawnChildExited(_)
                | fdo::Error::SpawnChildSignaled(_)
                | fdo::Error::SpawnFailed(_)
                | fdo::Error::SpawnServiceNotFound(_)
        )
    {
        return CaptureError("no XDG desktop portal Screenshot interface is available".into());
    }
    CaptureError(error.to_string())
}

fn response_outcome(status: u32, uri: Option<&str>) -> Result<CaptureOutcome, CaptureError> {
    match status {
        0 => {
            let uri = uri
                .ok_or_else(|| CaptureError("the screenshot portal returned no file URI".into()))?;
            Ok(CaptureOutcome::Captured(uri_to_path(uri)?))
        }
        1 | 2 => Ok(CaptureOutcome::Cancelled),
        other => Err(CaptureError(format!(
            "the screenshot portal ended with response {other}"
        ))),
    }
}

/// A local `file://` URI as a percent-decoded path.
pub fn uri_to_path(uri: &str) -> Result<PathBuf, CaptureError> {
    let invalid = || {
        CaptureError(format!(
            "the screenshot portal returned a non-local URI: {uri}"
        ))
    };
    let rest = uri.strip_prefix("file://").ok_or_else(invalid)?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(at) if rest[..at].eq_ignore_ascii_case("localhost") => &rest[at..],
        _ => return Err(invalid()),
    };
    let bytes = path.split(['?', '#']).next().unwrap_or_default().as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

static TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_request_is_interactive_and_not_modal() {
        let options = capture_options("gnome_annotation_1");
        assert_eq!(
            options.get("interactive"),
            Some(&Variant::from(true)),
            "GNOME's selection, window, and screen UI stays available"
        );
        assert_eq!(
            options.get("modal"),
            Some(&Variant::from(false)),
            "the parent spec asks for a non-modal capture"
        );
        assert_eq!(
            options.get("handle_token"),
            Some(&Variant::from("gnome_annotation_1"))
        );
    }

    #[test]
    fn response_zero_maps_the_uri_to_a_path() {
        assert_eq!(
            response_outcome(0, Some("file:///home/me/Shot%201.png")).unwrap(),
            CaptureOutcome::Captured(PathBuf::from("/home/me/Shot 1.png"))
        );
    }

    #[test]
    fn response_one_and_two_are_cancelled() {
        assert_eq!(
            response_outcome(1, None).unwrap(),
            CaptureOutcome::Cancelled
        );
        assert_eq!(
            response_outcome(2, None).unwrap(),
            CaptureOutcome::Cancelled
        );
    }

    #[test]
    fn response_zero_without_a_uri_is_an_error() {
        assert!(response_outcome(0, None).is_err());
    }

    #[test]
    fn an_unknown_response_is_an_error() {
        let error = response_outcome(3, Some("file:///tmp/a.png")).unwrap_err();
        assert!(error.to_string().contains("response 3"));
    }
}
