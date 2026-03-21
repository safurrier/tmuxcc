use ratatui::style::Color;
use std::fmt;

/// Status of a CI check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Pending,
    InProgress,
    Success,
    Failure,
    Neutral,
    Skipped,
}

impl CiStatus {
    /// Returns the ratatui color for rendering a dot
    pub fn dot_color(&self) -> Color {
        match self {
            CiStatus::Success => Color::Green,
            CiStatus::Failure => Color::Red,
            CiStatus::InProgress => Color::Yellow,
            CiStatus::Pending => Color::DarkGray,
            CiStatus::Neutral => Color::Gray,
            CiStatus::Skipped => Color::DarkGray,
        }
    }
}

/// A single CI check result
#[derive(Debug, Clone)]
pub struct CiCheck {
    pub name: String,
    pub status: CiStatus,
}

/// Review decision for a PR
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    Unknown,
}

impl ReviewDecision {
    /// Returns (icon, color) for display
    pub fn icon(&self) -> (&'static str, Color) {
        match self {
            ReviewDecision::Approved => ("✓", Color::Green),
            ReviewDecision::ChangesRequested => ("✗", Color::Red),
            ReviewDecision::ReviewRequired => ("○", Color::Yellow),
            ReviewDecision::Unknown => ("?", Color::DarkGray),
        }
    }
}

impl fmt::Display for ReviewDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewDecision::Approved => write!(f, "approved"),
            ReviewDecision::ChangesRequested => write!(f, "changes requested"),
            ReviewDecision::ReviewRequired => write!(f, "review required"),
            ReviewDecision::Unknown => write!(f, "unknown"),
        }
    }
}

/// Whether a PR can be merged
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeableState {
    Mergeable,
    Conflicting,
    Unknown,
}

impl MergeableState {
    /// Returns (icon, text, color) for display
    pub fn display(&self) -> (&'static str, &'static str, Color) {
        match self {
            MergeableState::Mergeable => ("✓", "mergeable", Color::Green),
            MergeableState::Conflicting => ("✗", "conflicts", Color::Red),
            MergeableState::Unknown => ("?", "unknown", Color::DarkGray),
        }
    }
}

/// Full PR information parsed from `gh pr view`
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub head_ref: String,
    pub base_ref: String,
    pub is_draft: bool,
    pub review_decision: ReviewDecision,
    pub mergeable: MergeableState,
    pub checks: Vec<CiCheck>,
    pub total_comments: u64,
    pub additions: u64,
    pub deletions: u64,
}

impl PrInfo {
    /// Format branch display: "head → base"
    pub fn branch_display(&self) -> String {
        format!("{} → {}", self.head_ref, self.base_ref)
    }

    /// Count of failed CI checks
    pub fn failed_checks(&self) -> Vec<&CiCheck> {
        self.checks
            .iter()
            .filter(|c| c.status == CiStatus::Failure)
            .collect()
    }

    /// Summary CI status
    pub fn ci_summary(&self) -> CiStatus {
        if self.checks.is_empty() {
            return CiStatus::Pending;
        }
        if self.checks.iter().any(|c| c.status == CiStatus::Failure) {
            return CiStatus::Failure;
        }
        if self.checks.iter().any(|c| c.status == CiStatus::InProgress) {
            return CiStatus::InProgress;
        }
        if self.checks.iter().any(|c| c.status == CiStatus::Pending) {
            return CiStatus::Pending;
        }
        CiStatus::Success
    }
}

/// Result of looking up a PR for a given path
#[derive(Debug, Clone)]
pub enum PrLookupResult {
    Found(PrInfo),
    NoPr,
    NotGitRepo,
    GhUnavailable,
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ci_status_dot_color() {
        assert_eq!(CiStatus::Success.dot_color(), Color::Green);
        assert_eq!(CiStatus::Failure.dot_color(), Color::Red);
        assert_eq!(CiStatus::InProgress.dot_color(), Color::Yellow);
        assert_eq!(CiStatus::Pending.dot_color(), Color::DarkGray);
        assert_eq!(CiStatus::Neutral.dot_color(), Color::Gray);
        assert_eq!(CiStatus::Skipped.dot_color(), Color::DarkGray);
    }

    #[test]
    fn test_review_decision_icon() {
        assert_eq!(ReviewDecision::Approved.icon(), ("✓", Color::Green));
        assert_eq!(ReviewDecision::ChangesRequested.icon(), ("✗", Color::Red));
        assert_eq!(ReviewDecision::ReviewRequired.icon(), ("○", Color::Yellow));
        assert_eq!(ReviewDecision::Unknown.icon(), ("?", Color::DarkGray));
    }

    #[test]
    fn test_mergeable_state_display() {
        assert_eq!(
            MergeableState::Mergeable.display(),
            ("✓", "mergeable", Color::Green)
        );
        assert_eq!(
            MergeableState::Conflicting.display(),
            ("✗", "conflicts", Color::Red)
        );
        assert_eq!(
            MergeableState::Unknown.display(),
            ("?", "unknown", Color::DarkGray)
        );
    }

    #[test]
    fn test_pr_info_branch_display() {
        let pr = PrInfo {
            number: 42,
            title: "Test".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com".to_string(),
            head_ref: "feature/x".to_string(),
            base_ref: "main".to_string(),
            is_draft: false,
            review_decision: ReviewDecision::Approved,
            mergeable: MergeableState::Mergeable,
            checks: vec![],
            total_comments: 0,
            additions: 0,
            deletions: 0,
        };
        assert_eq!(pr.branch_display(), "feature/x → main");
    }

    #[test]
    fn test_ci_summary() {
        let make_pr = |checks: Vec<CiStatus>| PrInfo {
            number: 1,
            title: String::new(),
            state: "OPEN".to_string(),
            url: String::new(),
            head_ref: String::new(),
            base_ref: String::new(),
            is_draft: false,
            review_decision: ReviewDecision::Unknown,
            mergeable: MergeableState::Unknown,
            checks: checks
                .into_iter()
                .map(|s| CiCheck {
                    name: "test".to_string(),
                    status: s,
                })
                .collect(),
            total_comments: 0,
            additions: 0,
            deletions: 0,
        };

        assert_eq!(make_pr(vec![]).ci_summary(), CiStatus::Pending);
        assert_eq!(
            make_pr(vec![CiStatus::Success, CiStatus::Success]).ci_summary(),
            CiStatus::Success
        );
        assert_eq!(
            make_pr(vec![CiStatus::Success, CiStatus::Failure]).ci_summary(),
            CiStatus::Failure
        );
        assert_eq!(
            make_pr(vec![CiStatus::Success, CiStatus::InProgress]).ci_summary(),
            CiStatus::InProgress
        );
        assert_eq!(
            make_pr(vec![CiStatus::Success, CiStatus::Pending]).ci_summary(),
            CiStatus::Pending
        );
    }
}
