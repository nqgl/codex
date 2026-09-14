//! Settings-adjacent popup surfaces for `ChatWidget`.
//!
//! This keeps theme and experimental-feature UI out of the main
//! orchestration module without changing their event wiring.

use super::*;

impl ChatWidget {
    pub(crate) fn open_delegation_popup(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Delegation selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }
        if !self.current_model_uses_multi_agent_v2() {
            self.add_info_message(
                "Agent delegation requires the multi-agent feature.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        let current_mode = self.effective_multi_agent_mode();
        let choices = [
            (
                MultiAgentMode::ExplicitRequestOnly,
                "Explicit requests only",
                "Delegate only when you explicitly request sub-agents.",
            ),
            (
                MultiAgentMode::Balanced,
                "Balanced",
                "Use a focused sub-agent for clearly independent, worthwhile work.",
            ),
            (
                MultiAgentMode::Proactive,
                "Proactive",
                "Let Codex delegate when it judges parallel work useful.",
            ),
        ];
        let mut items: Vec<SelectionItem> = choices
            .into_iter()
            .map(|(mode, name, description)| {
                let selected_mode = mode.clone();
                SelectionItem {
                    name: name.to_string(),
                    description: Some(description.to_string()),
                    is_current: current_mode == mode,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SetMultiAgentMode(selected_mode.clone()));
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        let max_threads = self
            .config
            .multi_agent_v2
            .max_concurrent_threads_per_session;
        items.push(SelectionItem {
            name: format!("Concurrency cap: {max_threads}"),
            description: Some(
                "Choose how many agents may be resident at once, including the primary."
                    .to_string(),
            ),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenMultiAgentConcurrencyPopup);
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        let mut header = ColumnRenderable::new();
        header.push(Line::from("Agent Delegation".bold()));
        header.push(Line::from(
            "Applies to this session only; config.toml is unchanged.".dim(),
        ));
        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        });
    }

    pub(crate) fn open_multi_agent_concurrency_popup(&mut self) {
        let current = self
            .config
            .multi_agent_v2
            .max_concurrent_threads_per_session;
        let mut choices = vec![1, 2, 3, 4, 6, 8, 12, 16];
        if !choices.contains(&current) {
            choices.push(current);
            choices.sort_unstable();
        }
        let items = choices
            .into_iter()
            .map(|max_threads| {
                let subagent_count = max_threads.saturating_sub(1);
                let description = if subagent_count == 0 {
                    "Primary agent only; delegation cannot run concurrently.".to_string()
                } else if subagent_count == 1 {
                    "Primary agent plus up to 1 concurrent sub-agent.".to_string()
                } else {
                    format!("Primary agent plus up to {subagent_count} concurrent sub-agents.")
                };
                SelectionItem {
                    name: max_threads.to_string(),
                    description: Some(description),
                    is_current: current == max_threads,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SetMultiAgentMaxConcurrentThreads(max_threads));
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        let mut header = ColumnRenderable::new();
        header.push(Line::from("Agent Concurrency".bold()));
        header.push(Line::from(
            "Applies to this session only; config.toml is unchanged.".dim(),
        ));
        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        });
    }

    pub(super) fn open_theme_picker(&mut self) {
        let codex_home = codex_utils_home_dir::find_codex_home().ok();
        let params = crate::theme_picker::build_theme_picker_params(
            self.local_settings.tui.theme.as_deref(),
            codex_home.as_deref(),
            self.last_rendered_width.get(),
        );
        self.bottom_pane.show_selection_view(params);
    }

    pub(crate) fn open_experimental_popup(&mut self) {
        let Some(thread_id) = self.thread_id() else {
            self.add_info_message(
                "Experimental features are unavailable until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        };
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        self.app_event_tx.send(AppEvent::FetchExperimentalFeatures {
            thread_id,
            response_tx,
        });
        let view = ExperimentalFeaturesView::new(
            Vec::new(),
            thread_id,
            Some(response_rx),
            self.app_event_tx.clone(),
            self.bottom_pane.list_keymap(),
        );
        self.bottom_pane.show_view(Box::new(view));
    }
}
