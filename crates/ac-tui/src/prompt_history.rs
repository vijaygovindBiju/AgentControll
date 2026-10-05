//! Command and prompt history management for the TUI.
//!
//! Provides history navigation (Up / Down) with draft preservation,
//! boundary checks, and consecutive duplicate suppression.

/// Reusable prompt and command history state machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptHistory {
    /// Oldest to newest list of submitted prompts.
    entries: Vec<String>,
    /// Active navigation position in `entries`.
    /// `None` indicates the user is at the live/draft line (not viewing history).
    /// `Some(idx)` indicates the user is viewing `entries[idx]`.
    nav_index: Option<usize>,
    /// The preserved draft input typed before navigating into history.
    draft: String,
}

impl PromptHistory {
    /// Create a new, empty prompt history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a newly submitted prompt.
    ///
    /// - Leading/trailing whitespace is trimmed for deduplication and emptiness checks.
    /// - Empty strings are ignored.
    /// - Consecutive duplicate prompts are suppressed.
    /// - Resets navigation state and clears the draft.
    pub fn record_submission(&mut self, prompt: &str) {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            self.reset_nav();
            return;
        }
        if self.entries.last().map(|s| s.as_str()) == Some(trimmed) {
            self.reset_nav();
            return;
        }
        self.entries.push(trimmed.to_string());
        self.reset_nav();
    }

    /// Navigate backward (older) in history.
    ///
    /// - If currently on the draft (`nav_index == None`), saves `current_input` as `draft`
    ///   and moves to the most recent entry.
    /// - If already navigating (`nav_index == Some(idx)`), moves to `idx - 1` until 0.
    /// - At the oldest boundary (`idx == 0`), stays at `entries[0]`.
    /// - If history is empty, returns `None` and does not panic.
    pub fn navigate_up(&mut self, current_input: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        match self.nav_index {
            None => {
                self.draft = current_input.to_string();
                let latest_idx = self.entries.len() - 1;
                self.nav_index = Some(latest_idx);
                Some(&self.entries[latest_idx])
            }
            Some(idx) => {
                if idx > 0 {
                    self.nav_index = Some(idx - 1);
                    Some(&self.entries[idx - 1])
                } else {
                    // Oldest boundary: stay at 0
                    Some(&self.entries[0])
                }
            }
        }
    }

    /// Navigate forward (newer) in history.
    ///
    /// - If not navigating (`nav_index == None`), stays at draft and returns `None`.
    /// - If navigating (`nav_index == Some(idx)`):
    ///   - If `idx + 1 < entries.len()`, moves to `idx + 1`.
    ///   - If `idx + 1 == entries.len()`, moves past the newest entry back to the saved `draft`,
    ///     resets `nav_index` to `None`, and returns `Some(&self.draft)`.
    /// - If history is empty, returns `None`.
    pub fn navigate_down(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        match self.nav_index {
            None => None, // Already at draft line
            Some(idx) => {
                if idx + 1 < self.entries.len() {
                    self.nav_index = Some(idx + 1);
                    Some(&self.entries[idx + 1])
                } else {
                    // Moving forward past newest entry restores the original draft
                    self.nav_index = None;
                    Some(&self.draft)
                }
            }
        }
    }

    /// Reset navigation position back to the live draft without erasing history entries.
    pub fn reset_nav(&mut self) {
        self.nav_index = None;
        self.draft.clear();
    }

    /// Whether the user is actively viewing a past history entry.
    pub fn is_navigating(&self) -> bool {
        self.nav_index.is_some()
    }

    /// Return the currently viewed history entry, or `None` if on draft.
    pub fn current_entry(&self) -> Option<&str> {
        self.nav_index
            .and_then(|idx| self.entries.get(idx))
            .map(|s| s.as_str())
    }

    /// Return the preserved draft string.
    pub fn draft(&self) -> &str {
        &self.draft
    }

    /// Slice of all submitted history entries from oldest to newest.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Number of entries in history.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if history contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_history() {
        let mut history = PromptHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.navigate_up("hello"), None);
        assert_eq!(history.navigate_down(), None);
        assert!(!history.is_navigating());
    }

    #[test]
    fn test_single_entry_navigation_and_boundaries() {
        let mut history = PromptHistory::new();
        history.record_submission("tell me about the project");
        assert_eq!(history.len(), 1);

        // Type a draft and press Up
        let up1 = history.navigate_up("explain arch");
        assert_eq!(up1, Some("tell me about the project"));
        assert!(history.is_navigating());
        assert_eq!(history.draft(), "explain arch");

        // Oldest boundary: pressing Up again stays at oldest
        let up2 = history.navigate_up("explain arch");
        assert_eq!(up2, Some("tell me about the project"));

        // Press Down: returns to draft
        let down1 = history.navigate_down();
        assert_eq!(down1, Some("explain arch"));
        assert!(!history.is_navigating());

        // Newest boundary: pressing Down again at draft stays there
        let down2 = history.navigate_down();
        assert_eq!(down2, None);
    }

    #[test]
    fn test_multiple_entries_traversal() {
        let mut history = PromptHistory::new();
        history.record_submission("prompt A");
        history.record_submission("prompt B");
        history.record_submission("prompt C");
        assert_eq!(history.len(), 3);

        let draft = "draft prompt";
        assert_eq!(history.navigate_up(draft), Some("prompt C"));
        assert_eq!(history.navigate_up(draft), Some("prompt B"));
        assert_eq!(history.navigate_up(draft), Some("prompt A"));
        // Boundary at oldest
        assert_eq!(history.navigate_up(draft), Some("prompt A"));

        // Navigate forward
        assert_eq!(history.navigate_down(), Some("prompt B"));
        assert_eq!(history.navigate_down(), Some("prompt C"));
        // Returning past newest restores draft
        assert_eq!(history.navigate_down(), Some("draft prompt"));
        assert!(!history.is_navigating());

        // Boundary at newest
        assert_eq!(history.navigate_down(), None);
    }

    #[test]
    fn test_empty_draft_preservation() {
        let mut history = PromptHistory::new();
        history.record_submission("first command");

        // Navigating up with empty draft
        assert_eq!(history.navigate_up(""), Some("first command"));
        assert_eq!(history.draft(), "");

        // Navigating down restores empty draft
        assert_eq!(history.navigate_down(), Some(""));
        assert!(!history.is_navigating());
    }

    #[test]
    fn test_duplicate_submission_suppression() {
        let mut history = PromptHistory::new();
        history.record_submission("cargo check");
        history.record_submission("cargo check");
        history.record_submission("  cargo check  ");
        assert_eq!(history.len(), 1);

        history.record_submission("cargo test");
        assert_eq!(history.len(), 2);

        // Different from last entry is accepted
        history.record_submission("cargo check");
        assert_eq!(history.len(), 3);
        assert_eq!(
            history.entries(),
            &["cargo check", "cargo test", "cargo check"]
        );
    }

    #[test]
    fn test_blank_submissions_ignored() {
        let mut history = PromptHistory::new();
        history.record_submission("");
        history.record_submission("   ");
        history.record_submission("\t\n");
        assert!(history.is_empty());
    }

    #[test]
    fn test_multiline_prompt_history() {
        let mut history = PromptHistory::new();
        let multiline = "def foo():\n    return 42";
        history.record_submission(multiline);
        assert_eq!(history.len(), 1);

        assert_eq!(history.navigate_up("current draft"), Some(multiline));
        assert_eq!(history.navigate_down(), Some("current draft"));
    }

    #[test]
    fn test_unicode_prompt_history() {
        let mut history = PromptHistory::new();
        let unicode_prompt = "Refactor 🦀 in src/main.rs: 日本語 & emoji ✨";
        history.record_submission(unicode_prompt);

        assert_eq!(history.navigate_up("draft"), Some(unicode_prompt));
        assert_eq!(history.navigate_down(), Some("draft"));
    }
}
