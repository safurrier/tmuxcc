use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use super::gh_client::GhClient;
use super::types::PrLookupResult;

/// Update sent from the PR monitor to the UI
#[derive(Debug, Clone)]
pub struct PrMonitorUpdate {
    /// Map from agent working directory path to PR lookup result
    pub results: HashMap<String, PrLookupResult>,
}

/// Background task that polls `gh pr view` for monitored agent paths
pub struct PrMonitorTask {
    tx: mpsc::Sender<PrMonitorUpdate>,
    poll_interval: Duration,
    paths_rx: watch::Receiver<Vec<String>>,
}

/// Rate limit backoff duration (5 minutes)
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(300);

impl PrMonitorTask {
    pub fn new(
        tx: mpsc::Sender<PrMonitorUpdate>,
        poll_interval: Duration,
        paths_rx: watch::Receiver<Vec<String>>,
    ) -> Self {
        Self {
            tx,
            poll_interval,
            paths_rx,
        }
    }

    /// Run the monitor loop. Call from a tokio::spawn.
    pub async fn run(self) {
        // Check if gh is available before starting
        let gh_available = tokio::task::spawn_blocking(GhClient::is_available)
            .await
            .unwrap_or(false);

        if !gh_available {
            tracing::warn!("gh CLI not available — PR monitoring disabled");
            let _ = self
                .tx
                .send(PrMonitorUpdate {
                    results: HashMap::new(),
                })
                .await;
            return;
        }

        // Immediate first poll
        self.poll_and_send().await;

        // Use interval instead of sleep so watch channel wakes don't starve the timer
        let mut interval = tokio::time::interval(self.poll_interval);
        interval.tick().await; // consume the immediate first tick

        let mut paths_rx = self.paths_rx.clone();
        let mut last_paths: Vec<String> = deduplicate_paths(&self.paths_rx.borrow());
        let mut rate_limited_until: Option<Instant> = None;
        loop {
            tokio::select! {
                // Periodic polling — always fires on schedule
                _ = interval.tick() => {
                    // Skip if rate limited
                    if let Some(until) = rate_limited_until {
                        if Instant::now() < until {
                            continue;
                        }
                        rate_limited_until = None;
                    }
                    let rate_limited = self.poll_and_send().await;
                    last_paths = deduplicate_paths(&self.paths_rx.borrow());
                    if rate_limited {
                        tracing::warn!("GitHub API rate limited — backing off for 5 minutes");
                        rate_limited_until = Some(Instant::now() + RATE_LIMIT_BACKOFF);
                    }
                }
                // Watch for path changes — poll immediately if paths actually changed
                result = paths_rx.changed() => {
                    if result.is_ok() {
                        let new_paths = deduplicate_paths(&self.paths_rx.borrow());
                        if new_paths != last_paths {
                            last_paths.clone_from(&new_paths);
                            // Poll immediately on real path changes (unless rate limited)
                            if rate_limited_until.map_or(true, |until| Instant::now() >= until) {
                                if rate_limited_until.is_some() {
                                    rate_limited_until = None;
                                }
                                let rate_limited = self.poll_and_send().await;
                                last_paths = deduplicate_paths(&self.paths_rx.borrow());
                                if rate_limited {
                                    tracing::warn!("GitHub API rate limited — backing off for 5 minutes");
                                    rate_limited_until = Some(Instant::now() + RATE_LIMIT_BACKOFF);
                                }
                                // Reset interval so the next periodic poll is a full interval away
                                interval.reset();
                            }
                        }
                    }
                }
                _ = self.tx.closed() => {
                    break;
                }
            }
        }
    }

    /// Poll all paths and send results. Returns true if rate limited.
    async fn poll_and_send(&self) -> bool {
        let paths = self.paths_rx.borrow().clone();
        let unique_paths: Vec<String> = deduplicate_paths(&paths);

        if unique_paths.is_empty() {
            let _ = self
                .tx
                .send(PrMonitorUpdate {
                    results: HashMap::new(),
                })
                .await;
            return false;
        }

        // Spawn blocking tasks for each unique path
        let mut handles = Vec::new();
        for path in unique_paths {
            let handle = tokio::task::spawn_blocking(move || {
                let result = GhClient::lookup_pr(&path);
                (path, result)
            });
            handles.push(handle);
        }

        // Collect results and check for rate limiting
        let mut results = HashMap::new();
        let mut hit_rate_limit = false;
        for handle in handles {
            if let Ok((path, result)) = handle.await {
                if let PrLookupResult::Error(ref msg) = result {
                    if msg.to_lowercase().contains("rate limit") {
                        hit_rate_limit = true;
                    }
                }
                results.insert(path, result);
            }
        }

        let _ = self.tx.send(PrMonitorUpdate { results }).await;
        hit_rate_limit
    }
}

/// Deduplicate paths, preserving order of first occurrence
fn deduplicate_paths(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            unique.push(path.clone());
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_deduplication() {
        let paths = vec![
            "/home/user/project-a".to_string(),
            "/home/user/project-b".to_string(),
            "/home/user/project-a".to_string(), // duplicate
        ];
        let unique = deduplicate_paths(&paths);
        assert_eq!(unique.len(), 2);
        assert_eq!(unique[0], "/home/user/project-a");
        assert_eq!(unique[1], "/home/user/project-b");
    }

    #[test]
    fn test_empty_paths() {
        let paths: Vec<String> = vec![];
        let unique = deduplicate_paths(&paths);
        assert!(unique.is_empty());
    }
}
