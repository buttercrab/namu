//! Files a process reloads while it runs: certificates, keys, tokens. Kubernetes updates a
//! secret mounted as files in place (when cert-manager renews a certificate, or a token is
//! rotated), so what matters is that their contents changed, whichever way they did.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

/// How often watched files are looked at.
pub const WATCH_EVERY: Duration = Duration::from_secs(10);

/// Calls `reload` whenever the contents of any of `paths` change, looking every `every`. A
/// reload that fails (files caught halfway through an update, say) is logged and tried again at
/// the next look, and what was loaded before stays.
pub fn watch(
    what: &'static str,
    paths: Vec<PathBuf>,
    every: Duration,
    reload: impl Fn() -> anyhow::Result<()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let mut loaded = fingerprint(&paths);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            let now = fingerprint(&paths);
            if now == loaded {
                continue;
            }
            match reload() {
                Ok(()) => {
                    tracing::info!("reloaded {what}");
                    loaded = now;
                }
                Err(err) => tracing::warn!("reloading {what} failed, keeping the last: {err:#}"),
            }
        }
    })
}

/// What the files hold, as a hash each (`None` for one that cannot be read).
fn fingerprint(paths: &[PathBuf]) -> Vec<Option<u64>> {
    paths
        .iter()
        .map(|path| {
            let contents = std::fs::read(path).ok()?;
            let mut hasher = DefaultHasher::new();
            contents.hash(&mut hasher);
            Some(hasher.finish())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn a_change_reloads_and_a_failed_reload_is_tried_again() {
        let dir = std::env::temp_dir().join(format!("namu-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("token");
        std::fs::write(&file, "one").unwrap();
        let reloads = Arc::new(AtomicUsize::new(0));
        let every = Duration::from_millis(20);
        let watching = watch("a token", vec![file.clone()], every, {
            let reloads = reloads.clone();
            let file = file.clone();
            move || {
                reloads.fetch_add(1, Ordering::SeqCst);
                let text = std::fs::read_to_string(&file)?;
                anyhow::ensure!(text != "half", "half written");
                Ok(())
            }
        });
        let settle = || tokio::time::sleep(every * 5);

        settle().await;
        assert_eq!(reloads.load(Ordering::SeqCst), 0, "nothing changed");
        std::fs::write(&file, "two").unwrap();
        settle().await;
        assert_eq!(reloads.load(Ordering::SeqCst), 1);

        // Caught halfway: tried at every look until it loads.
        std::fs::write(&file, "half").unwrap();
        settle().await;
        let tries = reloads.load(Ordering::SeqCst);
        assert!(tries > 2, "tried again: {tries}");
        std::fs::write(&file, "three").unwrap();
        settle().await;
        let after = reloads.load(Ordering::SeqCst);
        settle().await;
        assert_eq!(reloads.load(Ordering::SeqCst), after, "loaded, so it rests");

        watching.abort();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
