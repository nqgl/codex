//! Keep the old terminal frame visible across a transcript clear and its queued replacement.
use crossterm::execute;
use crossterm::terminal::BeginSynchronizedUpdate;
use crossterm::terminal::EndSynchronizedUpdate;
use std::io::Write;
use std::io::{self};

#[derive(Default)]
pub(super) struct HistoryReplaySync {
    active: bool,
}

impl HistoryReplaySync {
    pub(super) fn begin(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if !self.active {
            execute!(writer, BeginSynchronizedUpdate)?;
            self.active = true;
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if self.active {
            execute!(writer, EndSynchronizedUpdate)?;
            self.active = false;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "history_replay_sync_tests.rs"]
mod tests;
