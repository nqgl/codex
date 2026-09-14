//! Local Git state used by the directory watcher.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::DirectoryWatchUrgency;
use super::markers::tags_in_line;

const MAX_COMMIT_SUBJECT_CHARS: usize = 240;
const MAX_DIFF_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitCommitChange {
    pub(super) previous_head: Option<String>,
    pub(super) head: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) urgency: DirectoryWatchUrgency,
    pub(super) matched_tags: Vec<String>,
}

pub(super) struct GitState {
    worktree: PathBuf,
    watched_root: PathBuf,
    pub(super) git_dir: PathBuf,
    head: Option<String>,
}

impl GitState {
    pub(super) async fn discover(root: &Path) -> Option<Self> {
        let output = git_output(
            root,
            &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
        )
        .await?;
        let mut lines = output.lines();
        let worktree = PathBuf::from(lines.next()?);
        let git_dir = PathBuf::from(lines.next()?);
        let head = git_output(&worktree, &["rev-parse", "--verify", "HEAD"]).await;
        Some(Self {
            worktree,
            watched_root: root.to_path_buf(),
            git_dir,
            head,
        })
    }

    pub(super) async fn take_change(
        &mut self,
        markers: Option<&[String]>,
    ) -> Option<GitCommitChange> {
        let head = git_output(&self.worktree, &["rev-parse", "--verify", "HEAD"]).await;
        if head == self.head {
            return None;
        }
        let previous_head = std::mem::replace(&mut self.head, head.clone());
        let (urgency, matched_tags) = match (head.as_deref(), markers) {
            (Some(head), Some(markers)) => {
                self.added_line_match(previous_head.as_deref(), head, markers)
                    .await?
            }
            (Some(_), None) | (None, None) => (DirectoryWatchUrgency::Low, Vec::new()),
            (None, Some(_)) => return None,
        };
        let summary = if head.is_some() {
            git_output(&self.worktree, &["show", "-s", "--format=%h %s", "HEAD"])
                .await
                .map(|summary| truncate_chars(&summary, MAX_COMMIT_SUBJECT_CHARS))
        } else {
            None
        };
        Some(GitCommitChange {
            previous_head,
            head,
            summary,
            urgency,
            matched_tags,
        })
    }

    async fn added_line_match(
        &self,
        previous_head: Option<&str>,
        head: &str,
        markers: &[String],
    ) -> Option<(DirectoryWatchUrgency, Vec<String>)> {
        let mut command = Command::new("git");
        command
            .args(["-C"])
            .arg(&self.worktree)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped());
        match previous_head {
            Some(previous_head) => {
                command
                    .args(["diff", "--no-color", "--no-ext-diff", "--unified=0"])
                    .arg(previous_head)
                    .arg(head);
            }
            None => {
                command
                    .args([
                        "show",
                        "--format=",
                        "--no-color",
                        "--no-ext-diff",
                        "--unified=0",
                    ])
                    .arg(head);
            }
        }
        let pathspec = self
            .watched_root
            .strip_prefix(&self.worktree)
            .ok()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        command.arg("--").arg(pathspec);

        let mut child = command.spawn().ok()?;
        let stdout = child.stdout.take()?;
        let mut output = Vec::new();
        stdout
            .take(MAX_DIFF_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .ok()?;
        if output.len() as u64 > MAX_DIFF_BYTES {
            let _ = child.kill().await;
            let _ = child.wait().await;
            tracing::warn!(
                "Git diff exceeded {MAX_DIFF_BYTES} bytes; skipping tagged commit notification"
            );
            return None;
        }
        if !child.wait().await.ok()?.success() {
            return None;
        }
        added_lines_match(&String::from_utf8_lossy(&output), markers)
    }
}

fn added_lines_match(
    diff: &str,
    markers: &[String],
) -> Option<(DirectoryWatchUrgency, Vec<String>)> {
    let mut in_hunk = false;
    let mut urgency = None;
    let mut matched_tags = BTreeSet::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            continue;
        }
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        let Some(added_line) = in_hunk.then(|| line.strip_prefix('+')).flatten() else {
            continue;
        };
        for occurrence in tags_in_line(added_line, markers) {
            urgency = urgency.max(Some(occurrence.urgency));
            matched_tags.insert(occurrence.text);
        }
    }
    urgency.map(|urgency| (urgency, matched_tags.into_iter().collect()))
}

async fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.trim();
    (!output.is_empty()).then(|| output.to_string())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
