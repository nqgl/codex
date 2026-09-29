use super::new_group_mail_message_cell;
use crate::history_cell::HistoryCell;

#[test]
fn received_mail_shows_every_line() {
    let cell = new_group_mail_message_cell(
        "From alice to bob:\nFirst line\nSecond line with detail".to_string(),
    );
    let rendered = cell
        .display_lines(/*width*/ 20)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(rendered, @"
    ✉ From alice to bob:
      First line
      Second line with
      detail
    ");
}
