use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn math_viewport_height_changes_replay_without_changing_draft_or_source() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    let source = "Equation:\n\n\\[x^2\\]\n\nAfter equation.";
    app.transcript_cells = vec![Arc::new(AgentMarkdownCell::new(
        source.into(),
        Path::new("/tmp"),
    ))];
    app.chat_widget
        .handle_paste("first line\nsecond line".into());
    let draft = app.chat_widget.composer_text_with_pending();
    let original = app.transcript_cells[0].raw_lines();
    let revision = crate::math_render::revision();
    let mut tui = crate::tui::test_support::make_test_tui()?;
    let screen = Size::new(/*width*/ 80, /*height*/ 24);
    for (previous, next, expected_top) in [(4, 6, 0), (6, 4, 0), (6, 6, 18), (24, 30, 0)] {
        tui.terminal.set_viewport_area(Rect::new(
            /*x*/ 0,
            24 - previous,
            /*width*/ 80,
            previous,
        ));
        app.reflow_math_for_viewport_height(&mut tui, screen, next)?;
        assert_eq!(
            tui.terminal.viewport_area,
            Rect::new(/*x*/ 0, expected_top, /*width*/ 80, previous)
        );
        assert_eq!(
            (
                app.chat_widget.composer_text_with_pending(),
                app.transcript_cells[0].raw_lines(),
                crate::math_render::revision()
            ),
            (draft.clone(), original.clone(), revision),
        );
    }
    let rendered = app.render_transcript_lines_for_reflow(/*width*/ 80);
    assert_snapshot!(
        "math_after_composer_resize",
        rendered
            .lines
            .iter()
            .map(rendered_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(())
}

#[tokio::test]
async fn raw_mode_does_not_replay_math_on_composer_resize() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    app.transcript_cells = vec![Arc::new(AgentMarkdownCell::new(
        "\\[x\\]".into(),
        Path::new("/tmp"),
    ))];
    app.chat_widget.set_raw_output_mode(/*enabled*/ true);
    let mut tui = crate::tui::test_support::make_test_tui()?;
    let area = Rect::new(
        /*x*/ 0, /*y*/ 20, /*width*/ 80, /*height*/ 4,
    );
    tui.terminal.set_viewport_area(area);
    app.reflow_math_for_viewport_height(
        &mut tui,
        Size::new(/*width*/ 80, /*height*/ 24),
        /*height*/ 6,
    )?;
    assert_eq!(tui.terminal.viewport_area, area);
    Ok(())
}

#[tokio::test]
async fn finalized_math_requests_replay_without_a_resize() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    app.local_settings.tui.terminal_resize_reflow_max_rows = Some(20);
    app.begin_initial_history_replay_buffer();
    app.transcript_cells = vec![Arc::new(AgentMessageCell::new(
        vec![Line::from("[x^2]")],
        /*is_first_line*/ true,
    ))];
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.handle_consolidate_agent_message(
        &mut tui,
        "\\[x^2\\]".into(),
        PathBuf::from("/tmp"),
        /*inline_visualization_context*/ None,
        ConsolidationScrollbackReflow::IfResizeReflowRan,
        /*deferred_history_cell*/ None,
    )?;
    assert!(
        app.initial_history_replay_buffer
            .as_ref()
            .unwrap()
            .render_from_transcript_tail
    );
    let rendered = app.render_transcript_lines_for_reflow(/*width*/ 40);
    assert_snapshot!(
        "finalized_math_source",
        rendered
            .lines
            .iter()
            .map(rendered_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(())
}
