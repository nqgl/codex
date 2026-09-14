//! Runs isolated Git commands and decodes native filesystem paths.
//!
//! Commands remove inherited repository selectors and per-command Git config,
//! select the repository from the supplied working directory, and disable hooks
//! and filesystem monitoring.
//!
//! Filesystem monitoring is disabled uniformly, including Git's built-in daemon,
//! rather than probing support. These command-local overrides avoid running
//! repository-configured helpers without changing persistent Git settings.
//!
//! Working-tree operations also disable configured clean, smudge, and process filters.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_git_utils::SAFE_BARE_REPOSITORY_CONFIG;
use codex_git_utils::git_config_override_env;
use codex_protocol::shell_environment::scrub_non_inheritable_env_vars;

const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

#[derive(Clone, Copy)]
pub(crate) enum GitOperation {
    Metadata,
    WorkingTree,
}

pub(crate) fn git_output(
    cwd: &Path,
    operation: GitOperation,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<Output> {
    let mut command = base_git_command(cwd);
    if matches!(operation, GitOperation::WorkingTree) {
        command.envs(git_config_override_env(
            configured_filters(cwd)?.into_iter().flat_map(|filter| {
                [
                    (format!("filter.{filter}.process"), String::new()),
                    (format!("filter.{filter}.clean"), String::new()),
                    (format!("filter.{filter}.smudge"), String::new()),
                    (format!("filter.{filter}.required"), "false".to_owned()),
                ]
            }),
        ));
    }
    let output = command.args(args).output().context("failed to start git")?;
    ensure_git_success(&output)?;
    Ok(output)
}

pub(crate) fn git_stdout<I, S>(cwd: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(cwd, GitOperation::WorkingTree, args)?;
    let stdout = String::from_utf8(output.stdout).context("git output is not valid UTF-8")?;
    Ok(stdout.trim().to_owned())
}

pub(crate) fn git_path<I, S>(cwd: &Path, args: I) -> Result<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(cwd, GitOperation::Metadata, args)?;
    let bytes = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    #[cfg(windows)]
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    git_path_from_bytes(bytes)
}

pub(crate) fn git_path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    if bytes.is_empty() {
        bail!("git returned an empty path");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(
            std::str::from_utf8(bytes).context("git path is not valid UTF-8")?,
        ))
    }
}

/// List worktrees in NUL-delimited form, with a quoted-path fallback for older Git.
pub(crate) fn worktree_porcelain(cwd: &Path) -> Result<Vec<u8>> {
    let output = base_git_command(cwd)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .context("failed to list Git worktrees")?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    if output.status.code() != Some(129) {
        ensure_git_success(&output)?;
    }
    // Git 2.34 rejects -z with a usage error. This read-only retry retains the same isolation
    // from hooks/configuration and the caller's registration/backlink checks.
    let legacy = base_git_command(cwd)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("failed to list Git worktrees without -z")?;
    ensure_git_success(&legacy)?;
    normalize_worktree_porcelain(&legacy.stdout)
}

fn normalize_worktree_porcelain(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(data.len());
    for line in data
        .strip_suffix(b"\n")
        .unwrap_or(data)
        .split(|byte| *byte == b'\n')
    {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(quoted) = line.strip_prefix(b"worktree \"") {
            let quoted = quoted
                .strip_suffix(b"\"")
                .context("unterminated quoted Git worktree path")?;
            output.extend_from_slice(b"worktree ");
            let mut bytes = quoted.iter().copied().peekable();
            while let Some(byte) = bytes.next() {
                if byte != b'\\' {
                    output.push(byte);
                    continue;
                }
                let escaped = bytes.next().context("incomplete Git path escape")?;
                let decoded = match escaped {
                    b'a' => 7,
                    b'b' => 8,
                    b't' => 9,
                    b'n' => 10,
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => 13,
                    b'\\' | b'"' => escaped,
                    b'0'..=b'7' => {
                        let mut value = u16::from(escaped - b'0');
                        for _ in 0..2 {
                            if let Some(next @ b'0'..=b'7') = bytes.peek().copied() {
                                bytes.next();
                                value = value * 8 + u16::from(next - b'0');
                            } else {
                                break;
                            }
                        }
                        u8::try_from(value).context("invalid octal Git path escape")?
                    }
                    _ => bail!("unknown Git path escape"),
                };
                if decoded == 0 {
                    bail!("Git worktree path contains a NUL byte");
                }
                output.push(decoded);
            }
        } else {
            output.extend_from_slice(line);
        }
        output.push(b'\0');
    }
    Ok(output)
}

/// Resolves the cached repository default without fetching or inheriting Git selectors.
/// Prefer a remote HEAD (origin first), then conventional main/master refs.
pub fn default_worktree_base(cwd: &Path) -> Result<String> {
    let output = git_output(
        cwd,
        GitOperation::Metadata,
        [
            "for-each-ref",
            "--format=%(refname) %(symref)",
            "refs/remotes",
            "refs/heads",
        ],
    )?;
    // Unrelated refs may contain non-UTF-8 bytes. Decode only the chosen base,
    // because CreateWorktree accepts a UTF-8 revision.
    let branches = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let separator = line.iter().position(|byte| *byte == b' ')?;
            Some((&line[..separator], &line[separator + 1..]))
        })
        .collect::<Vec<_>>();
    let base = branches
        .iter()
        .filter(|(name, target)| name.ends_with(b"/HEAD") && !target.is_empty())
        .min_by_key(|(name, _)| (!name.starts_with(b"refs/remotes/origin/"), *name))
        .map(|(_, target)| *target)
        .or_else(|| {
            [
                b"refs/remotes/origin/main".as_slice(),
                b"refs/remotes/origin/master".as_slice(),
                b"refs/heads/main".as_slice(),
                b"refs/heads/master".as_slice(),
            ]
            .into_iter()
            .find(|candidate| branches.iter().any(|(branch, _)| branch == candidate))
        })
        .context("Could not determine the project's default branch.")?;
    std::str::from_utf8(base)
        .context("The project's default branch is not valid UTF-8.")
        .map(str::to_owned)
}

fn base_git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    // Git wrappers export repository-local state to their children. Select this
    // repository from cwd, and install only our own per-command config below.
    for name in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CEILING_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
    command
        .current_dir(cwd)
        .arg("-c")
        .arg(SAFE_BARE_REPOSITORY_CONFIG)
        .arg("-c")
        .arg(format!("core.hooksPath={DISABLED_HOOKS_PATH}"))
        .arg("-c")
        .arg("core.fsmonitor=")
        .arg("-c")
        .arg("attr.tree=")
        .arg("-c")
        .arg("core.attributesFile=")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    scrub_non_inheritable_env_vars(&mut command);
    command
}

fn configured_filters(cwd: &Path) -> Result<BTreeSet<String>> {
    let output = base_git_command(cwd)
        .args([
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            "^filter\\.",
        ])
        .output()
        .context("failed to inspect Git filter configuration")?;

    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(BTreeSet::new());
        }
        ensure_git_success(&output)?;
    }

    let mut filters = BTreeSet::new();
    let names = String::from_utf8(output.stdout).context("Git filter names are not valid UTF-8")?;
    for name in names.split('\0') {
        if let Some((driver, attribute)) = name
            .strip_prefix("filter.")
            .and_then(|name| name.rsplit_once('.'))
            && !driver.is_empty()
            && matches!(attribute, "clean" | "smudge" | "process" | "required")
        {
            filters.insert(driver.to_owned());
        }
    }
    Ok(filters)
}

fn ensure_git_success(output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("git command failed: {}", stderr.trim())
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
