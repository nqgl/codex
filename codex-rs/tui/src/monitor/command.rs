//! Parsing for the user-facing `/monitor` command.

use std::path::Path;
use std::path::PathBuf;

use super::MonitorCommand;
use super::MonitorRequest;
use super::MonitorTrust;

pub(crate) const MONITOR_USAGE: &str = "Monitor commands:
  /monitor add <name> [--cwd <directory>] [--trust] -- <command>
  /monitor list
  /monitor pause <name>
  /monitor resume <name>
  /monitor remove <name>

Options for add:
  --cwd <directory>  Run the command from this directory (defaults to the session cwd)
  --trust            Deliver output without the untrusted-input warning
  --                 End monitor options and begin the command

Example:
  /monitor add project-watch --cwd ~/project --trust -- ./watch-for-changes";

pub(crate) fn parse_monitor_command(args: &str, cwd: &Path) -> Result<MonitorCommand, String> {
    let (prefix, command) = split_command(args);
    let tokens = shlex::split(prefix).ok_or_else(|| {
        format!("Could not parse /monitor arguments. Check quotes.\n{MONITOR_USAGE}")
    })?;
    match tokens.as_slice() {
        [] => return Ok(MonitorCommand::List),
        [action]
            if action.eq_ignore_ascii_case("list") || action.eq_ignore_ascii_case("status") =>
        {
            return Ok(MonitorCommand::List);
        }
        [action, name] if action.eq_ignore_ascii_case("pause") => {
            return Ok(MonitorCommand::Pause(normalize_name(name)?));
        }
        [action, name] if action.eq_ignore_ascii_case("resume") => {
            return Ok(MonitorCommand::Resume(normalize_name(name)?));
        }
        [action, name] if action.eq_ignore_ascii_case("remove") => {
            return Ok(MonitorCommand::Remove(normalize_name(name)?));
        }
        [action, ..] if !action.eq_ignore_ascii_case("add") => {
            return Err(format!(
                "Unknown /monitor action: {action}\n{MONITOR_USAGE}"
            ));
        }
        _ => {}
    }

    let Some(command) = command.filter(|command| !command.trim().is_empty()) else {
        return Err(format!(
            "/monitor add requires `--` followed by a command.\n{MONITOR_USAGE}"
        ));
    };
    let Some(name) = tokens.get(1) else {
        return Err(MONITOR_USAGE.to_string());
    };
    let name = normalize_name(name)?;
    let mut monitor_cwd = cwd.to_path_buf();
    let mut trust = MonitorTrust::Untrusted;
    let mut index = 2;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--cwd" => {
                let Some(path) = tokens.get(index + 1) else {
                    return Err(format!("--cwd requires a directory.\n{MONITOR_USAGE}"));
                };
                monitor_cwd = resolve_path(path, cwd);
                index += 2;
            }
            "--trust" => {
                trust = MonitorTrust::Trusted;
                index += 1;
            }
            option if option.starts_with("--") => {
                return Err(format!(
                    "Unknown /monitor option: {option}\n{MONITOR_USAGE}"
                ));
            }
            token => {
                return Err(format!(
                    "Unexpected /monitor argument: {token}\n{MONITOR_USAGE}"
                ));
            }
        }
    }

    Ok(MonitorCommand::Add(MonitorRequest {
        name,
        command: command.trim().to_string(),
        cwd: monitor_cwd,
        trust,
    }))
}

fn split_command(args: &str) -> (&str, Option<&str>) {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in args.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(open_quote) = quote {
            if ch == open_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if !args[index..].starts_with("--") {
            continue;
        }
        let before_is_boundary = args[..index]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let after = &args[index + 2..];
        let after_is_boundary = after.chars().next().is_none_or(char::is_whitespace);
        if before_is_boundary && after_is_boundary {
            return (&args[..index], Some(after.trim_start()));
        }
    }
    (args, None)
}

fn normalize_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty()
        || name.chars().count() > 64
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "Monitor names may contain 1-64 letters, numbers, dots, dashes, or underscores.\n{MONITOR_USAGE}"
        ));
    }
    Ok(name)
}

fn resolve_path(path: impl AsRef<Path>, cwd: &Path) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
