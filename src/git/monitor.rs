use std::collections::{HashMap, HashSet};
use std::time::Duration;

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
            // Send a single update indicating gh is unavailable, then stop
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

        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {
                    self.poll_and_send().await;
                }
                _ = self.tx.closed() => {
                    break;
                }
            }
        }
    }

    async fn poll_and_send(&self) {
        let paths = self.paths_rx.borrow().clone();
        let unique_paths: Vec<String> = deduplicate_paths(&paths);

        if unique_paths.is_empty() {
            let _ = self
                .tx
                .send(PrMonitorUpdate {
                    results: HashMap::new(),
                })
                .await;
            return;
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

        // Collect results
        let mut results = HashMap::new();
        for handle in handles {
            if let Ok((path, result)) = handle.await {
                results.insert(path, result);
            }
        }

        let _ = self.tx.send(PrMonitorUpdate { results }).await;
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
