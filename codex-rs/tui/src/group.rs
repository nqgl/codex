//! User-only group membership commands.

pub(crate) const GROUP_USAGE: &str = "Usage: /group [join <group> <name>|leave]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GroupCommand {
    Status,
    Join { group: String, name: String },
    Leave,
}

pub(crate) fn parse_group_command(args: &str) -> Result<GroupCommand, String> {
    let words = args.split_whitespace().collect::<Vec<_>>();
    match words.as_slice() {
        [] | ["status"] => Ok(GroupCommand::Status),
        ["join", group, name] => Ok(GroupCommand::Join {
            group: (*group).to_string(),
            name: (*name).to_string(),
        }),
        ["leave"] => Ok(GroupCommand::Leave),
        _ => Err(GROUP_USAGE.to_string()),
    }
}
