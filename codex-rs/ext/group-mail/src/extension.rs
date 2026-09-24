use crate::store::GroupMailStore;
use crate::store::Membership;
use crate::tools::GroupMailTool;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_core::context::GroupMailDelivery;
use codex_core::context::GroupMailInstructions;
use codex_extension_api::ContextContributor;
use codex_extension_api::ContextualUserFragment;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::PromptFragment;
use codex_extension_api::ThreadIdleCause;
use codex_extension_api::ThreadIdleInput;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadReadyInput;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolContributor;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolFinishInput;
use codex_extension_api::ToolLifecycleContributor;
use codex_extension_api::ToolLifecycleFuture;
use codex_extension_api::TurnContextContributionInput;
use codex_extension_api::TurnLifecycleContributor;
use codex_extension_api::TurnStartInput;
use codex_protocol::AgentPath;
use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::Op;
use codex_protocol::turn_input::TurnStartOptions;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub(super) struct Binding {
    pub(super) thread_id: ThreadId,
    pub(super) owner: String,
    pub(super) store: GroupMailStore,
    membership: StdMutex<Option<Membership>>,
    owned: AtomicBool,
    claim_pending: AtomicBool,
    dispatch: Arc<Mutex<()>>,
    watcher: StdMutex<Option<JoinHandle<()>>>,
}

struct GroupMailExtension {
    manager: Weak<ThreadManager>,
}

pub fn install(registry: &mut ExtensionRegistryBuilder<Config>, manager: Weak<ThreadManager>) {
    let extension = Arc::new(GroupMailExtension { manager });
    registry.thread_lifecycle_contributor(extension.clone());
    registry.turn_lifecycle_contributor(extension.clone());
    registry.prompt_contributor(extension.clone());
    registry.tool_lifecycle_contributor(extension.clone());
    registry.tool_contributor(extension);
}

impl ThreadLifecycleContributor<Config> for GroupMailExtension {
    fn on_thread_start<'a>(
        &'a self,
        input: ThreadStartInput<'a, Config>,
    ) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if input.config.ephemeral || !input.persistent_thread_state_available {
                return;
            }
            let (Ok(thread_id), Ok(session_id)) = (
                ThreadId::from_string(input.thread_store.level_id()),
                SessionId::from_string(input.session_store.level_id()),
            ) else {
                return;
            };
            if thread_id != ThreadId::from(session_id) {
                return;
            }
            let store = match GroupMailStore::open(input.config.sqlite_config().home()).await {
                Ok(store) => store,
                Err(error) => {
                    tracing::warn!(%thread_id, %error, "failed to open group mail");
                    return;
                }
            };
            let membership = match store.membership(thread_id).await {
                Ok(membership) => membership,
                Err(error) => {
                    tracing::warn!(%thread_id, %error, "failed to load group membership");
                    return;
                }
            };
            input.thread_store.insert(Binding {
                thread_id,
                owner: Uuid::now_v7().to_string(),
                store,
                membership: StdMutex::new(membership),
                owned: AtomicBool::new(false),
                claim_pending: AtomicBool::new(true),
                dispatch: Arc::new(Mutex::new(())),
                watcher: StdMutex::new(None),
            });
        })
    }

    fn on_thread_ready<'a>(
        &'a self,
        input: ThreadReadyInput<'a, Config>,
    ) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            let Some(binding) = input.thread_store.get::<Binding>() else {
                return;
            };
            if binding.current_membership().is_some() {
                match binding
                    .store
                    .mark_online(binding.thread_id, &binding.owner)
                    .await
                {
                    Ok(()) => {
                        binding.owned.store(true, Ordering::Release);
                        binding.claim_pending.store(false, Ordering::Release);
                    }
                    Err(error) => tracing::warn!(%error, "failed to mark group peer online"),
                }
            }
            let weak = Arc::downgrade(&binding);
            let manager = self.manager.clone();
            let task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(500));
                loop {
                    interval.tick().await;
                    let Some(binding) = weak.upgrade() else {
                        return;
                    };
                    if let Err(error) = binding.refresh().await {
                        tracing::warn!(%error, "failed to refresh group mail");
                        continue;
                    }
                    if let Err(error) = dispatch(&binding, &manager, DispatchMode::HighOnly).await {
                        tracing::warn!(%error, "failed to deliver group mail");
                    }
                    if let Some(manager) = manager.upgrade()
                        && let Ok(thread) = manager.get_thread(binding.thread_id).await
                        && matches!(
                            thread.agent_status().await,
                            AgentStatus::PendingInit
                                | AgentStatus::Completed(_)
                                | AgentStatus::Errored(_)
                        )
                        && !binding
                            .store
                            .pending(binding.thread_id, &binding.owner)
                            .await
                            .unwrap_or_default()
                            .is_empty()
                    {
                        thread
                            .emit_thread_idle_lifecycle_if_idle(ThreadIdleCause::Completed)
                            .await;
                    }
                }
            });
            *binding
                .watcher
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
        })
    }

    fn on_thread_idle<'a>(&'a self, input: ThreadIdleInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Some(binding) = input.thread_store.get::<Binding>()
                && let Err(error) =
                    dispatch(&binding, &self.manager, DispatchMode::IdleOrHigh).await
            {
                tracing::warn!(%error, "failed to deliver idle group mail");
            }
        })
    }

    fn on_thread_stop<'a>(&'a self, input: ThreadStopInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            let Some(binding) = input.thread_store.get::<Binding>() else {
                return;
            };
            let task = binding
                .watcher
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(task) = task {
                task.abort();
                let _ = task.await;
            }
            if let Err(error) = binding
                .store
                .mark_offline(binding.thread_id, &binding.owner)
                .await
            {
                tracing::warn!(%error, "failed to mark group peer offline");
            }
        })
    }
}

impl TurnLifecycleContributor for GroupMailExtension {
    fn on_turn_start<'a>(&'a self, input: TurnStartInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Some(binding) = input.thread_store.get::<Binding>()
                && let Err(error) = binding.refresh().await
            {
                tracing::warn!(%error, "failed to refresh group membership for turn");
            }
        })
    }
}

impl ContextContributor for GroupMailExtension {
    fn contribute_turn_context<'a>(
        &'a self,
        input: TurnContextContributionInput<'a>,
    ) -> ExtensionFuture<'a, Vec<PromptFragment>> {
        Box::pin(async move {
            let Some(binding) = input.thread_store.get::<Binding>() else {
                return Vec::new();
            };
            let Some(member) = binding.current_membership() else {
                return Vec::new();
            };
            let fragment = GroupMailInstructions::new(member.name, member.group);
            vec![PromptFragment::developer_capability(
                fragment.render(),
                fragment.content_kind(),
            )]
        })
    }
}

impl ToolContributor for GroupMailExtension {
    fn tools(
        &self,
        _session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn for<'call> ToolExecutor<ToolCall<'call>>>> {
        let Some(binding) = thread_store.get::<Binding>() else {
            return Vec::new();
        };
        if binding.current_membership().is_none() {
            return Vec::new();
        }
        ["send_to", "broadcast"]
            .into_iter()
            .map(|name| {
                Arc::new(GroupMailTool::new(name, binding.clone()))
                    as Arc<dyn for<'call> ToolExecutor<ToolCall<'call>>>
            })
            .collect()
    }
}

impl ToolLifecycleContributor for GroupMailExtension {
    fn on_tool_finish<'a>(&'a self, input: ToolFinishInput<'a>) -> ToolLifecycleFuture<'a> {
        Box::pin(async move {
            if let Some(binding) = input.thread_store.get::<Binding>()
                && let Err(error) = dispatch(&binding, &self.manager, DispatchMode::HighOnly).await
            {
                tracing::warn!(%error, "failed to deliver high-priority group mail");
            }
        })
    }
}

impl Binding {
    fn current_membership(&self) -> Option<Membership> {
        self.membership
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn refresh(&self) -> anyhow::Result<()> {
        let current = self.store.membership(self.thread_id).await?;
        if current != self.current_membership() {
            self.owned.store(false, Ordering::Release);
            self.claim_pending.store(true, Ordering::Release);
        }
        if current.is_some() {
            if self.claim_pending.load(Ordering::Acquire) {
                self.store.mark_online(self.thread_id, &self.owner).await?;
                self.owned.store(true, Ordering::Release);
                self.claim_pending.store(false, Ordering::Release);
            } else if self.owned.load(Ordering::Acquire)
                && !self.store.heartbeat(self.thread_id, &self.owner).await?
            {
                self.owned.store(false, Ordering::Release);
            }
        } else {
            self.owned.store(false, Ordering::Release);
            self.claim_pending.store(true, Ordering::Release);
        }
        *self
            .membership
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = current;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum DispatchMode {
    HighOnly,
    IdleOrHigh,
}

async fn dispatch(
    binding: &Binding,
    manager: &Weak<ThreadManager>,
    mode: DispatchMode,
) -> anyhow::Result<()> {
    let _guard = Arc::clone(&binding.dispatch).lock_owned().await;
    let Some(member) = binding.current_membership() else {
        return Ok(());
    };
    if !binding.owned.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(manager) = manager.upgrade() else {
        return Ok(());
    };
    let Ok(thread) = manager.get_thread(binding.thread_id).await else {
        return Ok(());
    };
    let status = thread.agent_status().await;
    if matches!(
        status,
        AgentStatus::Interrupted | AgentStatus::Shutdown | AgentStatus::NotFound
    ) {
        return Ok(());
    }
    let pending = binding
        .store
        .pending(binding.thread_id, &binding.owner)
        .await?;
    let through = match mode {
        DispatchMode::HighOnly => pending.iter().rposition(|mail| mail.high),
        DispatchMode::IdleOrHigh if matches!(status, AgentStatus::Running) => {
            pending.iter().rposition(|mail| mail.high)
        }
        DispatchMode::IdleOrHigh => pending.len().checked_sub(1),
    };
    let Some(through) = through else {
        return Ok(());
    };
    let delivery = &pending[..=through];
    let content = GroupMailDelivery::new(
        member.name,
        delivery
            .iter()
            .map(|mail| (mail.sender.clone(), mail.body.clone()))
            .collect(),
    )
    .render();
    let author = AgentPath::root()
        .join("group_mail")
        .map_err(anyhow::Error::msg)?;
    let communication = InterAgentCommunication::new(
        author,
        AgentPath::root(),
        Vec::new(),
        content,
        /*trigger_turn*/ true,
    );
    thread
        .submit(Op::InterAgentCommunication {
            communication,
            start_options: TurnStartOptions::default(),
        })
        .await?;
    let ids = delivery.iter().map(|mail| mail.id).collect::<Vec<_>>();
    binding
        .store
        .acknowledge(binding.thread_id, &binding.owner, &ids)
        .await?;
    Ok(())
}
