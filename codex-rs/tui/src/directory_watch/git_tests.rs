use std::process::Command as StdCommand;

use pretty_assertions::assert_eq;
use tempfile::tempdir;

use super::*;

#[test]
fn added_line_tags_set_urgency_and_removed_tags_do_not_match() {
    let markers = vec!["@codex".to_string()];
    let low = "\
diff --git a/notes.md b/notes.md
@@ -1,0 +2 @@
+please check @codex
";
    let high = "\
diff --git a/notes.md b/notes.md
@@ -1,0 +2 @@
+please check @codex#from:glen#urgency:high
";
    let removed = "\
diff --git a/notes.md b/notes.md
@@ -1 +0,0 @@
-please check @codex #urgency:high
";

    assert_eq!(
        added_lines_match(low, &markers),
        Some((DirectoryWatchUrgency::Low, vec!["@codex".to_string()]))
    );
    assert_eq!(
        added_lines_match(high, &markers),
        Some((
            DirectoryWatchUrgency::High,
            vec!["@codex#from:glen#urgency:high".to_string()]
        ))
    );
    assert_eq!(added_lines_match(removed, &markers), None);
}

#[tokio::test]
async fn git_state_only_returns_commits_with_added_tags() {
    let temp = tempdir().expect("tempdir");
    git(temp.path(), &["init"]);
    git(temp.path(), &["config", "user.name", "Codex Test"]);
    git(
        temp.path(),
        &["config", "user.email", "codex@example.invalid"],
    );
    std::fs::write(temp.path().join("notes.md"), "initial\n").expect("write initial file");
    git(temp.path(), &["add", "notes.md"]);
    git(temp.path(), &["commit", "-m", "initial"]);

    let mut state = GitState::discover(temp.path()).await.expect("Git state");
    let markers = vec!["@codex".to_string()];

    std::fs::write(temp.path().join("notes.md"), "initial\nordinary change\n")
        .expect("write ordinary change");
    git(temp.path(), &["add", "notes.md"]);
    git(temp.path(), &["commit", "-m", "ordinary"]);
    assert_eq!(state.take_change(Some(&markers)).await, None);

    std::fs::write(
        temp.path().join("notes.md"),
        "initial\nordinary change\n@codex#from:glen#urgency:high\n",
    )
    .expect("write tagged change");
    git(temp.path(), &["add", "notes.md"]);
    git(temp.path(), &["commit", "-m", "tagged"]);
    let change = state
        .take_change(Some(&markers))
        .await
        .expect("tagged commit");
    assert_eq!(change.urgency, DirectoryWatchUrgency::High);
    assert_eq!(
        change.matched_tags,
        vec!["@codex#from:glen#urgency:high".to_string()]
    );
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git command failed: {args:?}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
