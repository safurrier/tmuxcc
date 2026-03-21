use std::process::Command;

use super::types::*;

const GH_PR_JSON_FIELDS: &str = "number,title,state,url,headRefName,baseRefName,reviewDecision,statusCheckRollup,mergeable,isDraft,additions,deletions,comments";

/// Client for interacting with the `gh` CLI
pub struct GhClient;

impl GhClient {
    /// Check if `gh` CLI is installed and available
    pub fn is_available() -> bool {
        Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Look up PR info for a given working directory
    pub fn lookup_pr(path: &str) -> PrLookupResult {
        // Check if path is a git repo
        let git_check = Command::new("git")
            .args(["-C", path, "rev-parse", "--is-inside-work-tree"])
            .output();

        match git_check {
            Ok(output) if output.status.success() => {}
            _ => return PrLookupResult::NotGitRepo,
        }

        // Run gh pr view
        let output = match Command::new("gh")
            .args(["pr", "view", "--json", GH_PR_JSON_FIELDS])
            .current_dir(path)
            .output()
        {
            Ok(o) => o,
            Err(_) => return PrLookupResult::GhUnavailable,
        };

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_pr_json(&stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            classify_gh_error(output.status.code().unwrap_or(1), &stderr)
        }
    }

    /// Get the current git branch for a path
    pub fn current_branch(path: &str) -> Option<String> {
        let output = Command::new("git")
            .args(["-C", path, "branch", "--show-current"])
            .output()
            .ok()?;

        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if branch.is_empty() {
                None
            } else {
                Some(branch)
            }
        } else {
            None
        }
    }
}

/// Parse JSON output from `gh pr view --json`. Exposed for testing.
pub fn parse_pr_json(json: &str) -> PrLookupResult {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return PrLookupResult::Error(format!("JSON parse error: {}", e)),
    };

    let number = value["number"].as_u64().unwrap_or(0);
    let title = value["title"].as_str().unwrap_or("").to_string();
    let state = value["state"].as_str().unwrap_or("UNKNOWN").to_string();
    let url = value["url"].as_str().unwrap_or("").to_string();
    let head_ref = value["headRefName"].as_str().unwrap_or("").to_string();
    let base_ref = value["baseRefName"].as_str().unwrap_or("").to_string();
    let is_draft = value["isDraft"].as_bool().unwrap_or(false);
    let additions = value["additions"].as_u64().unwrap_or(0);
    let deletions = value["deletions"].as_u64().unwrap_or(0);

    // comments can be an array or a number depending on gh version
    let total_comments = if let Some(n) = value["comments"].as_u64() {
        n
    } else if let Some(arr) = value["comments"].as_array() {
        arr.len() as u64
    } else {
        0
    };

    let review_decision = match value["reviewDecision"].as_str() {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
        _ => ReviewDecision::Unknown,
    };

    let mergeable = match value["mergeable"].as_str() {
        Some("MERGEABLE") => MergeableState::Mergeable,
        Some("CONFLICTING") => MergeableState::Conflicting,
        _ => MergeableState::Unknown,
    };

    let checks = parse_status_checks(&value["statusCheckRollup"]);

    PrLookupResult::Found(PrInfo {
        number,
        title,
        state,
        url,
        head_ref,
        base_ref,
        is_draft,
        review_decision,
        mergeable,
        checks,
        total_comments,
        additions,
        deletions,
    })
}

fn parse_status_checks(value: &serde_json::Value) -> Vec<CiCheck> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    arr.iter()
        .filter_map(|item| {
            // Check runs have "name" + "status" + "conclusion"
            if let Some(name) = item["name"].as_str() {
                let status_str = item["status"].as_str().unwrap_or("");
                let conclusion_str = item["conclusion"].as_str().unwrap_or("");

                let status = match status_str {
                    "COMPLETED" => match conclusion_str {
                        "SUCCESS" => CiStatus::Success,
                        "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => {
                            CiStatus::Failure
                        }
                        "NEUTRAL" => CiStatus::Neutral,
                        "SKIPPED" | "CANCELLED" | "STALE" => CiStatus::Skipped,
                        _ => CiStatus::Neutral,
                    },
                    "IN_PROGRESS" => CiStatus::InProgress,
                    "QUEUED" | "WAITING" | "PENDING" | "REQUESTED" => CiStatus::Pending,
                    _ => CiStatus::Pending,
                };

                return Some(CiCheck {
                    name: name.to_string(),
                    status,
                });
            }

            // StatusContext entries have "context" + "state" (no "name"/"status")
            if let Some(context) = item["context"].as_str() {
                let state_str = item["state"].as_str().unwrap_or("");
                let status = match state_str {
                    "SUCCESS" => CiStatus::Success,
                    "FAILURE" | "ERROR" => CiStatus::Failure,
                    "PENDING" | "EXPECTED" => CiStatus::Pending,
                    _ => CiStatus::Pending,
                };
                return Some(CiCheck {
                    name: context.to_string(),
                    status,
                });
            }

            None
        })
        .collect()
}

/// Classify a non-zero exit from `gh pr view`. Exposed for testing.
pub fn classify_gh_error(exit_code: i32, stderr: &str) -> PrLookupResult {
    let lower = stderr.to_lowercase();
    if lower.contains("no pull requests found") {
        PrLookupResult::NoPr
    } else if lower.contains("gh auth login") || lower.contains("not logged in") {
        PrLookupResult::GhUnavailable
    } else {
        PrLookupResult::Error(format!("gh exited {} — {}", exit_code, stderr.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_PR_JSON: &str = r#"{
        "number": 42,
        "title": "Add auth middleware",
        "state": "OPEN",
        "url": "https://github.com/user/repo/pull/42",
        "headRefName": "feature/add-auth",
        "baseRefName": "main",
        "isDraft": false,
        "reviewDecision": "APPROVED",
        "mergeable": "MERGEABLE",
        "additions": 150,
        "deletions": 30,
        "comments": [{"body": "lgtm"}, {"body": "nice"}],
        "statusCheckRollup": [
            {"name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"},
            {"name": "test", "status": "COMPLETED", "conclusion": "SUCCESS"},
            {"name": "lint", "status": "COMPLETED", "conclusion": "FAILURE"},
            {"name": "deploy", "status": "IN_PROGRESS", "conclusion": ""}
        ]
    }"#;

    #[test]
    fn test_parse_full_pr_json() {
        let result = parse_pr_json(FULL_PR_JSON);
        let pr = match result {
            PrLookupResult::Found(pr) => pr,
            other => panic!("expected Found, got {:?}", other),
        };
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Add auth middleware");
        assert_eq!(pr.state, "OPEN");
        assert_eq!(pr.url, "https://github.com/user/repo/pull/42");
        assert_eq!(pr.head_ref, "feature/add-auth");
        assert_eq!(pr.base_ref, "main");
        assert!(!pr.is_draft);
        assert_eq!(pr.review_decision, ReviewDecision::Approved);
        assert_eq!(pr.mergeable, MergeableState::Mergeable);
        assert_eq!(pr.additions, 150);
        assert_eq!(pr.deletions, 30);
        assert_eq!(pr.total_comments, 2);
        assert_eq!(pr.checks.len(), 4);
        assert_eq!(pr.checks[0].name, "build");
        assert_eq!(pr.checks[0].status, CiStatus::Success);
        assert_eq!(pr.checks[2].status, CiStatus::Failure);
        assert_eq!(pr.checks[3].status, CiStatus::InProgress);
    }

    #[test]
    fn test_parse_minimal_pr_json() {
        let json = r#"{"number": 1, "title": "t", "state": "OPEN", "url": "https://x.com/1", "headRefName": "feat", "baseRefName": "main"}"#;
        let result = parse_pr_json(json);
        let pr = match result {
            PrLookupResult::Found(pr) => pr,
            other => panic!("expected Found, got {:?}", other),
        };
        assert_eq!(pr.number, 1);
        assert!(!pr.is_draft);
        assert_eq!(pr.review_decision, ReviewDecision::Unknown);
        assert_eq!(pr.mergeable, MergeableState::Unknown);
        assert!(pr.checks.is_empty());
        assert_eq!(pr.total_comments, 0);
        assert_eq!(pr.additions, 0);
        assert_eq!(pr.deletions, 0);
    }

    #[test]
    fn test_parse_draft_pr() {
        let json = r#"{"number": 5, "title": "wip", "state": "OPEN", "url": "u", "headRefName": "h", "baseRefName": "b", "isDraft": true}"#;
        let result = parse_pr_json(json);
        let pr = match result {
            PrLookupResult::Found(pr) => pr,
            other => panic!("expected Found, got {:?}", other),
        };
        assert!(pr.is_draft);
    }

    #[test]
    fn test_parse_all_ci_statuses() {
        let json = r#"{
            "number": 1, "title": "", "state": "OPEN", "url": "", "headRefName": "", "baseRefName": "",
            "statusCheckRollup": [
                {"name": "a", "status": "QUEUED", "conclusion": ""},
                {"name": "b", "status": "IN_PROGRESS", "conclusion": ""},
                {"name": "c", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "d", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"name": "e", "status": "COMPLETED", "conclusion": "NEUTRAL"},
                {"name": "f", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"name": "g", "status": "COMPLETED", "conclusion": "TIMED_OUT"},
                {"name": "h", "status": "WAITING", "conclusion": ""}
            ]
        }"#;
        let pr = match parse_pr_json(json) {
            PrLookupResult::Found(pr) => pr,
            other => panic!("expected Found, got {:?}", other),
        };
        assert_eq!(pr.checks.len(), 8);
        assert_eq!(pr.checks[0].status, CiStatus::Pending); // QUEUED
        assert_eq!(pr.checks[1].status, CiStatus::InProgress);
        assert_eq!(pr.checks[2].status, CiStatus::Success);
        assert_eq!(pr.checks[3].status, CiStatus::Failure);
        assert_eq!(pr.checks[4].status, CiStatus::Neutral);
        assert_eq!(pr.checks[5].status, CiStatus::Skipped);
        assert_eq!(pr.checks[6].status, CiStatus::Failure); // TIMED_OUT
        assert_eq!(pr.checks[7].status, CiStatus::Pending); // WAITING
    }

    #[test]
    fn test_parse_all_review_decisions() {
        let make = |rd: &str| {
            let json = format!(
                r#"{{"number":1,"title":"","state":"OPEN","url":"","headRefName":"","baseRefName":"","reviewDecision":"{}"}}"#,
                rd
            );
            match parse_pr_json(&json) {
                PrLookupResult::Found(pr) => pr.review_decision,
                _ => panic!("expected Found"),
            }
        };
        assert_eq!(make("APPROVED"), ReviewDecision::Approved);
        assert_eq!(make("CHANGES_REQUESTED"), ReviewDecision::ChangesRequested);
        assert_eq!(make("REVIEW_REQUIRED"), ReviewDecision::ReviewRequired);
        assert_eq!(make("SOMETHING_ELSE"), ReviewDecision::Unknown);
    }

    #[test]
    fn test_parse_all_mergeable_states() {
        let make = |ms: &str| {
            let json = format!(
                r#"{{"number":1,"title":"","state":"OPEN","url":"","headRefName":"","baseRefName":"","mergeable":"{}"}}"#,
                ms
            );
            match parse_pr_json(&json) {
                PrLookupResult::Found(pr) => pr.mergeable,
                _ => panic!("expected Found"),
            }
        };
        assert_eq!(make("MERGEABLE"), MergeableState::Mergeable);
        assert_eq!(make("CONFLICTING"), MergeableState::Conflicting);
        assert_eq!(make("UNKNOWN"), MergeableState::Unknown);
    }

    #[test]
    fn test_parse_empty_checks() {
        // empty array
        let json = r#"{"number":1,"title":"","state":"OPEN","url":"","headRefName":"","baseRefName":"","statusCheckRollup":[]}"#;
        let pr = match parse_pr_json(json) {
            PrLookupResult::Found(pr) => pr,
            _ => panic!("expected Found"),
        };
        assert!(pr.checks.is_empty());

        // null / missing
        let json2 = r#"{"number":1,"title":"","state":"OPEN","url":"","headRefName":"","baseRefName":"","statusCheckRollup":null}"#;
        let pr2 = match parse_pr_json(json2) {
            PrLookupResult::Found(pr) => pr,
            _ => panic!("expected Found"),
        };
        assert!(pr2.checks.is_empty());
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = parse_pr_json("not valid json {{{");
        assert!(matches!(result, PrLookupResult::Error(_)));
    }

    #[test]
    fn test_no_pr_stderr_detection() {
        let result = classify_gh_error(1, "no pull requests found for branch \"feature/add-auth\"");
        assert!(matches!(result, PrLookupResult::NoPr));
    }

    #[test]
    fn test_rate_limit_error() {
        let result = classify_gh_error(1, "API rate limit exceeded");
        assert!(matches!(result, PrLookupResult::Error(_)));
    }

    #[test]
    fn test_auth_error() {
        let result = classify_gh_error(
            1,
            "To get started with GitHub CLI, please run: gh auth login",
        );
        assert!(matches!(result, PrLookupResult::GhUnavailable));
    }

    #[test]
    fn test_parse_status_context_entries() {
        let json = r#"{
            "number": 10,
            "title": "test",
            "state": "OPEN",
            "url": "https://github.com/test/10",
            "headRefName": "feat",
            "baseRefName": "main",
            "statusCheckRollup": [
                {"context": "ci/circleci", "state": "SUCCESS"},
                {"context": "deploy/staging", "state": "FAILURE"},
                {"context": "license/cla", "state": "PENDING"}
            ]
        }"#;
        let pr = match parse_pr_json(json) {
            PrLookupResult::Found(pr) => pr,
            _ => panic!("expected Found"),
        };
        assert_eq!(pr.checks.len(), 3);
        assert_eq!(pr.checks[0].name, "ci/circleci");
        assert_eq!(pr.checks[0].status, CiStatus::Success);
        assert_eq!(pr.checks[1].name, "deploy/staging");
        assert_eq!(pr.checks[1].status, CiStatus::Failure);
        assert_eq!(pr.checks[2].name, "license/cla");
        assert_eq!(pr.checks[2].status, CiStatus::Pending);
    }

    #[test]
    fn test_parse_mixed_check_runs_and_status_contexts() {
        let json = r#"{
            "number": 11,
            "title": "mixed",
            "state": "OPEN",
            "url": "https://github.com/test/11",
            "headRefName": "feat",
            "baseRefName": "main",
            "statusCheckRollup": [
                {"name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"context": "ci/external", "state": "FAILURE"},
                {"name": "lint", "status": "COMPLETED", "conclusion": "ACTION_REQUIRED"},
                {"name": "stale-check", "status": "COMPLETED", "conclusion": "STALE"}
            ]
        }"#;
        let pr = match parse_pr_json(json) {
            PrLookupResult::Found(pr) => pr,
            _ => panic!("expected Found"),
        };
        assert_eq!(pr.checks.len(), 4);
        assert_eq!(pr.checks[0].status, CiStatus::Success);
        assert_eq!(pr.checks[1].status, CiStatus::Failure); // StatusContext
        assert_eq!(pr.checks[2].status, CiStatus::Failure); // ACTION_REQUIRED
        assert_eq!(pr.checks[3].status, CiStatus::Skipped); // STALE
    }
}
