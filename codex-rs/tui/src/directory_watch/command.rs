//! Parsing for the user-facing `/watch` command.

use std::path::Path;
use std::path::PathBuf;

use super::DirectoryWatchCommand;
use super::DirectoryWatchFilter;
use super::DirectoryWatchMode;
use super::DirectoryWatchRequest;
use super::DirectoryWatchTrust;

pub(crate) const WATCH_USAGE: &str = "Usage: /watch <directory> \
    [--tagged [<tag>] ... | --marker <tag> ...] [--files-only | --commits-only] [--trust] \
    | /watch status | /watch stop [<directory>]";

const DEFAULT_MARKERS: [&str; 2] = ["@codex", "@all"];

pub(crate) fn parse_watch_command(args: &str, cwd: &Path) -> Result<DirectoryWatchCommand, String> {
    let tokens = shlex::split(args)
        .ok_or_else(|| format!("Could not parse /watch arguments. Check quotes.\n{WATCH_USAGE}"))?;
    match tokens.as_slice() {
        [] => return Ok(DirectoryWatchCommand::Status),
        [command] if command.eq_ignore_ascii_case("status") => {
            return Ok(DirectoryWatchCommand::Status);
        }
        [command] if command.eq_ignore_ascii_case("stop") => {
            return Ok(DirectoryWatchCommand::StopAll);
        }
        [command, path] if command.eq_ignore_ascii_case("stop") => {
            return Ok(DirectoryWatchCommand::StopRoot(resolve_path(path, cwd)));
        }
        _ => {}
    }

    let mut path = None;
    let mut markers = Vec::new();
    let mut mode = DirectoryWatchMode::FilesAndCommits;
    let mut trust = DirectoryWatchTrust::Untrusted;
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--tagged" => {
                let explicit_marker = path
                    .is_some()
                    .then(|| tokens.get(index + 1))
                    .flatten()
                    .filter(|marker| !marker.starts_with("--"));
                if let Some(marker) = explicit_marker {
                    markers.push(normalize_marker(marker)?);
                    index += 2;
                } else {
                    markers.extend(DEFAULT_MARKERS.map(str::to_string));
                    index += 1;
                }
            }
            "--marker" => {
                let Some(marker) = tokens.get(index + 1) else {
                    return Err(format!("--marker requires text.\n{WATCH_USAGE}"));
                };
                markers.push(normalize_marker(marker)?);
                index += 2;
            }
            "--files-only" => {
                if mode == DirectoryWatchMode::CommitsOnly {
                    return Err(format!(
                        "--files-only and --commits-only cannot be combined.\n{WATCH_USAGE}"
                    ));
                }
                mode = DirectoryWatchMode::FilesOnly;
                index += 1;
            }
            "--commits-only" => {
                if mode == DirectoryWatchMode::FilesOnly {
                    return Err(format!(
                        "--files-only and --commits-only cannot be combined.\n{WATCH_USAGE}"
                    ));
                }
                mode = DirectoryWatchMode::CommitsOnly;
                index += 1;
            }
            "--trust" => {
                trust = DirectoryWatchTrust::Trusted;
                index += 1;
            }
            token if token.starts_with("--") => {
                return Err(format!("Unknown /watch option: {token}\n{WATCH_USAGE}"));
            }
            token => {
                if path.replace(PathBuf::from(token)).is_some() {
                    return Err(format!(
                        "/watch accepts one directory at a time.\n{WATCH_USAGE}"
                    ));
                }
                index += 1;
            }
        }
    }

    let Some(path) = path else {
        return Err(WATCH_USAGE.to_string());
    };
    markers.sort();
    markers.dedup();
    let root = resolve_path(path, cwd);
    let filter = if markers.is_empty() {
        DirectoryWatchFilter::All
    } else {
        DirectoryWatchFilter::HeaderMarkers(markers)
    };
    Ok(DirectoryWatchCommand::Start(DirectoryWatchRequest {
        root,
        filter,
        mode,
        trust,
    }))
}

fn resolve_path(path: impl AsRef<Path>, cwd: &Path) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn normalize_marker(marker: &str) -> Result<String, String> {
    let marker = marker.trim().to_lowercase();
    let marker = marker
        .split_once('#')
        .map_or(marker.as_str(), |(base, _)| base);
    if marker.is_empty() {
        return Err(format!("A watch tag cannot be empty.\n{WATCH_USAGE}"));
    }
    if marker.starts_with('@') {
        Ok(marker.to_string())
    } else {
        Ok(format!("@{marker}"))
    }
}
