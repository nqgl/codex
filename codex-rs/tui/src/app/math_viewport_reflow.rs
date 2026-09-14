//! Repair math placements before the inline composer changes the transcript's scroll region.
//!
//! The normal viewport-growth path scrolls only the rows above the composer. Rebuilding from
//! source instead gives Kitty fresh placeholder rows and image placements in one replay, just as
//! a terminal resize does. Cached rasters are reused; this does not invalidate the math revision
//! or change the conversation. Only frames with an actual viewport-height change take this path.

use super::App;
use crate::history_cell::HistoryRenderMode;
use crate::tui;
use color_eyre::eyre::Result;
use ratatui::layout::Size;

impl App {
    /// Called only when math images exist and the ordinary chat view owns the terminal.
    pub(super) fn reflow_math_for_viewport_height(
        &mut self,
        tui: &mut tui::Tui,
        screen_size: Size,
        height: u16,
    ) -> Result<()> {
        let previous_height = tui.terminal.viewport_area.height;
        if previous_height == 0
            || previous_height == height.min(screen_size.height)
            || self.chat_widget.history_render_mode() == HistoryRenderMode::Raw
            || self.transcript_cells.is_empty()
        {
            return Ok(());
        }
        // Replay resets the viewport's anchor before draw updates its height, avoiding a partial
        // scroll of already-displayed math. Keep the existing streaming/replay bookkeeping.
        tracing::debug!(
            previous_height,
            height,
            "replaying math after viewport height change"
        );
        self.finish_required_stream_reflow(tui)
    }
}
