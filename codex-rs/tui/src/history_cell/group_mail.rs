use super::PrefixedWrappedHistoryCell;
use ratatui::style::Stylize;

pub(crate) fn new_group_mail_message_cell(text: String) -> PrefixedWrappedHistoryCell {
    PrefixedWrappedHistoryCell::new(text, "✉ ".cyan(), "  ")
}

#[cfg(test)]
#[path = "group_mail_tests.rs"]
mod tests;
