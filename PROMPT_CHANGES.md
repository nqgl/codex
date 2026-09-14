# Custom prompt changes

This ledger tracks changes to model-facing instructions. The edits emphasize
evidence, clear mechanisms, appropriate scope, and concise guidance while
preserving behavioral constraints.

**How to use on upstream update:** for each row,
`git diff <baseline>..<new-upstream> -- <location>` shows whether
upstream touched a surface we changed; conflicts on this branch mean
the same thing. New prompt files in upstream diffs are candidates for
first-time treatment. Latest audited upstream baseline: `5fb3b7e401` (September 14, 2026).
The original rewrite baseline was `35eaf3ffb0bf2001486c68c47a3d946b34d16634`.
See [the September 14 integration record](glen/merge-2026-09-14.md) for compatibility
decisions and validation.

| Surface | Location | Change | Notes |
|---|---|---|---|
| 5.6 base prompt (sol/terra/luna) | models.json base_instructions → deployed out-of-tree from glen/gpt-5.6_base.md via model_instructions_file | rewrite (168→~120 lines) | Condenses skill guidance; removes fixed research-time limits and blanket praise restrictions; distinguishes confidence from evidence for destructive actions; preserves intent and personality guidance |
| goals continuation | codex-rs/ext/goal/templates/goals/continuation.md | distill and migrate | Preserves evidence-based completion checks; incorporates upstream waiting, progress, objective-fidelity, and three-turn blocked-audit mechanics with less repetition |
| orchestrator agent | codex-rs/core/templates/agents/orchestrator.md | rewrite | Consolidates accumulated guidance into one coherent description |
| collab multi-agent | codex-rs/core/templates/collab/experimental_prompt.md | rewrite | Explains delegation mechanics and their rationale |
| review rubric | codex-rs/prompts/templates/review/rubric.md | distill 87→53 | Preserves the JSON schema and bug criteria; removes redundant reviewer-style guidance |
| plan mode | codex-rs/collaboration-mode-templates/templates/plan.md | wording only | Preserves mechanics, categories, and blocking-question rules |
| execute mode | removed upstream | retired | hidden Execute mode and its template were removed upstream; the prior dedupe no longer has a live prompt surface |
| default mode | codex-rs/collaboration-mode-templates/templates/default.md | concise merge | retains assumption-first behavior; incorporates upstream optional-question, no-answer, and permission-routing mechanics |
| pair programming mode | removed upstream | retired | hidden Pair Programming mode and its template were removed upstream |
| agent roles | codex-rs/core/src/agent/role.rs | reword | Clarifies explorer/worker descriptions and verification guidance |
| multi-agent tool text | codex-rs/core/src/tools/handlers/multi_agents_spec.rs | 2 lines | Clarifies send_input guidance and spawn authorization |
| multi-agent mode policy | codex-rs/core/src/context/multi_agent_mode_instructions.rs | rewrite | explicit mode remains strict; balanced mode permits one clearly worthwhile independent delegate without making delegation a ritual; proactive mode keeps lean delegation and centralized synthesis while incorporating upstream user-override semantics |
| multi-agent model routing | codex-rs/core/src/session/multi_agents.rs | reword | parent/Sol remains the ordinary default, especially for cache-friendly full-history forks; lower-capability or lower-effort overrides are reserved for clearly mechanical bounded work with settled semantics and normally use no/selective history |
| wait_for_environment | codex-rs/core/src/tools/handlers/wait_for_environment.rs | 1 line | do-not-wait → no-need-to-wait |
| guardian denial text | codex-rs/ext/guardian-reviewer/src/completion.rs | reword, moved upstream | rejection follow-up as mechanism; timeout and review-failure text untouched |
| permissions: never | codex-rs/prompts/templates/permissions/approval_policy/never.md | reword | rejection mechanics |
| compact prompt | codex-rs/prompts/templates/compact/prompt.md | deliberately unchanged | Retains upstream compaction behavior |
| persistent mode | codex-rs/core/assets/persistent_mode.md | new upstream surface, unchanged pending review | Retains post-final follow-up authorization and stopping rules; wording and repetition remain candidates for review |
| memories write templates | codex-rs/memories/write/templates/memories/{stage_one_system,consolidation}.md | rewrite 1449→506 | Retains collaborative reasoning, decisions, rationale, and conceptual models; emphasizes deletion of superseded material; removes repeated rules; resolves conflicting preference guidance; adds Understanding sections while preserving schemas and contracts |
| memories read path | codex-rs/ext/memories/templates/memories/read_path.md | 1 passage | update-gate stated as mechanism; quick-pass/citation contracts untouched |
| memories v2 templates | codex-rs/memories/write/templates/memories/*_v2.md; codex-rs/ext/memories/templates/memories/read_path_v2.md | new upstream surface, unchanged pending review | opt-in version/dual-write path; custom v1 rewrites do not apply automatically |
| guardian charter | codex-rs/core/assets/guardian/{policy_template.md,policy.md} | unchanged after upstream move | Retains the detailed approval-review policy |
| guardian synchronous reviewer | codex-rs/ext/guardian-v2/src/sync_reviewer/policy_template.md | new upstream surface, unchanged | internal risk reviewer; preserve exact policy mechanics |
| shell/exec tool specs | codex-rs/core/src/tools/handlers/shell_spec.rs | unchanged | Retains platform-specific execution guidance |
| skipped as dormant | realtime/, tui ide_context, windows sandbox, older-model prompts, sample skills, imagegen | unchanged | revisit if enabled |
