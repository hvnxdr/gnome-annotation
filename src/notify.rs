//! Desktop notifications through `org.freedesktop.Notifications`. The daemon reports capture
//! failures here; tests record them behind the same trait.

use std::collections::HashMap;
use std::time::Duration;

const APP_NAME: &str = "gnome-annotation";
const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const TIMEOUT: Duration = Duration::from_secs(2);
const EXPIRE_MS: i32 = 5000;

pub trait Notifier: Send + Sync {
    fn notify(&self, summary: &str, body: &str);
}

/// The live notification server. A missing or failing daemon is logged, never fatal.
pub struct DbusNotifier;

impl Notifier for DbusNotifier {
    fn notify(&self, summary: &str, body: &str) {
        if let Err(error) = notify_blocking(summary, body) {
            eprintln!("gnome-annotation: cannot show a desktop notification: {error}");
        }
    }
}

fn notify_blocking(summary: &str, body: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start the notification runtime: {error}"))?;
    runtime.block_on(async {
        let connection = tokio::time::timeout(TIMEOUT, zbus::Connection::session())
            .await
            .map_err(|_| "the session bus did not answer".to_owned())?
            .map_err(|error| error.to_string())?;
        let proxy = tokio::time::timeout(
            TIMEOUT,
            zbus::Proxy::new(&connection, DEST, PATH, INTERFACE),
        )
        .await
        .map_err(|_| "the notification service did not answer".to_owned())?
        .map_err(|error| error.to_string())?;
        let actions: Vec<&str> = Vec::new();
        let hints: HashMap<&str, zbus::Variant> = HashMap::new();
        let args = (APP_NAME, 0u32, "", summary, body, actions, hints, EXPIRE_MS);
        let call = proxy.call::<_, _, u32>("Notify", &args);
        tokio::time::timeout(TIMEOUT, call)
            .await
            .map_err(|_| "the notification service did not answer".to_owned())?
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}
