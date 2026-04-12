//! [`GitSource`] — clone a git repository into a temporary directory.

use lore_core::LoreError;

use super::{PreparedSource, Source};

/// A documentation source that clones a remote git repository.
///
/// The repository is cloned into a [`tempfile::TempDir`] which is removed
/// when the returned [`PreparedSource`] is dropped.
pub struct GitSource {
    /// Remote URL to clone (e.g. `https://github.com/org/repo`).
    pub url: String,
    /// Branch or tag to check out. Defaults to the remote's `HEAD`.
    pub branch: Option<String>,
}

impl GitSource {
    /// Create a [`GitSource`] for the given repository URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), branch: None }
    }

    /// Set the branch or tag to check out after cloning.
    #[must_use]
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }
}

impl Source for GitSource {
    async fn prepare(&self) -> Result<PreparedSource, LoreError> {
        let url = self.url.clone();
        let branch = self.branch.clone();

        // git2 I/O is synchronous; run it off the async reactor.
        tokio::task::spawn_blocking(move || clone_repo(&url, branch.as_deref()))
            .await
            .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn clone_repo(url: &str, branch: Option<&str>) -> Result<PreparedSource, LoreError> {
    let temp = tempfile::TempDir::new().map_err(LoreError::Io)?;

    let repo = git2::Repository::clone(url, temp.path())
        .map_err(|e| LoreError::Registry(format!("git clone failed: {e}")))?;

    if let Some(branch_name) = branch {
        checkout_branch(&repo, branch_name)?;
    }

    let sha = head_sha(&repo);
    Ok(PreparedSource::from_temp(temp, sha))
}

/// Check out the named branch or tag.
///
/// Always ends in detached HEAD pointing at the resolved commit.  This works
/// for branches, tags, and bare commit SHAs without needing to know the ref
/// type up-front.
fn checkout_branch(repo: &git2::Repository, branch_name: &str) -> Result<(), LoreError> {
    // Try remote-tracking first (the common case after a fresh clone), then
    // fall back to any ref that matches (tags, short SHAs, …).
    let obj = repo
        .revparse_single(&format!("refs/remotes/origin/{branch_name}"))
        .or_else(|_| repo.revparse_single(branch_name))
        .map_err(|e| {
            LoreError::Registry(format!("branch or ref '{branch_name}' not found: {e}"))
        })?;

    repo.checkout_tree(&obj, None)
        .map_err(|e| LoreError::Registry(format!("checkout failed: {e}")))?;

    let commit = obj
        .peel_to_commit()
        .map_err(|e| LoreError::Registry(format!("ref '{branch_name}' is not a commit: {e}")))?;
    repo.set_head_detached(commit.id())
        .map_err(|e| LoreError::Registry(format!("set HEAD failed: {e}")))?;

    Ok(())
}

/// Return the short SHA of HEAD, or `None` if the repo has no commits.
fn head_sha(repo: &git2::Repository) -> Option<String> {
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    Some(commit.id().to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_source_builder() {
        let src = GitSource::new("https://github.com/example/repo").with_branch("main");
        assert_eq!(src.url, "https://github.com/example/repo");
        assert_eq!(src.branch.as_deref(), Some("main"));
    }
}
