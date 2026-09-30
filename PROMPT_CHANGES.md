# Custom prompt changes

This ledger tracks changes to model-facing instructions. The edits emphasize
evidence, clear mechanisms, appropriate scope, and concise guidance while
preserving behavioral constraints.

## Select prompt wording

Use `codex -c 'prompt_mode="upstream"'` to select upstream wording, or
`codex -c 'prompt_mode="custom"'` to retain this fork's wording. `custom` is
the default. You can also set `prompt_mode` in `config.toml` or a named profile;
`default` is an alias for `upstream`.

The CLI forwards the effective mode in thread start, resume, and fork requests.
An updated daemon resolves prompts per thread, so its launch-time mode is only a
default. Different clients can use different modes on the same daemon. Existing
loaded threads retain their configuration; reconnecting does not rewrite them.

Upstream mode uses the selected model's catalog base instructions. It ignores
local base, developer, and compaction prompt overrides, `model_instructions_file`,
`model_catalog_json`, and configured multi-agent prompt overrides. It selects
administrator-managed prompt settings and required catalogs before local choices.
Project instructions, skills, and managed security policies remain active. It uses
stock wording for the edited delegation, collaboration, review, Guardian denial,
goal continuation, memory read, and memory write surfaces. The bundled stock
alternatives are copied from `rust-v0.159.1`; refresh them during upstream merges.

Both modes retain project instructions, skills, safety policies, and the minimal
guidance needed by custom-only features such as group mail, Balanced delegation,
Code Mode bundles, and wake-on-message delivery. Explicit role/task instructions
also remain active. Neither mode changes tool behavior.
The unused orchestrator and legacy collab templates have no runtime effect.

Start a fresh session for a clean comparison. Resuming a session retains its
earlier prompt messages; the switch does not rewrite conversation history.

P0 size review: stock goal continuation, Plan mode, review, memory read, and
stage-one memory writing templates exceed 1,000 tokens. They are fixed,
verbatim upstream alternatives; dynamic input handling is unchanged.
The v1 memory-consolidation template is about 10,770 tokens before interpolation
(`o200k_base`, tiktoken 0.12). The user explicitly approved this background job
as an exception to the repository's 10K-per-item cap on September 30, 2026.
Upstream mode uses it without truncation; custom mode retains the shorter version.

**How to use on upstream update:** for each row,
`git diff <baseline>..<new-upstream> -- <location>` shows whether
upstream touched a surface we changed; conflicts on this branch mean
the same thing. New prompt files in upstream diffs are candidates for
first-time treatment. Latest audited upstream baseline: `722dae7afd` (September 22, 2026).
The original rewrite baseline was `35eaf3ffb0bf2001486c68c47a3d946b34d16634`.
See [the September 22 integration record](glen/merge-2026-09-22.md) for compatibility
decisions and validation.
The [Rust 0.156.1 release-tag merge](glen/merge-2026-09-23-rust-v0.156.1.md)
adds no prompt changes; the September 22 upstream baseline remains the latest
audited `main` baseline.

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
| multi-agent mode policy | codex-rs/core/src/context/multi_agent_mode_instructions.rs; codex-rs/prompts/src/model_messages/multi_agent.rs | rewrite, moved upstream | explicit mode remains strict; balanced mode permits one clearly worthwhile independent delegate without making delegation a ritual; proactive mode keeps lean delegation and centralized synthesis while incorporating upstream user-override semantics |
| multi-agent model routing | codex-rs/prompts/src/multi_agent_instructions.rs | reword, moved upstream | parent/Sol remains the ordinary default, especially for cache-friendly full-history forks; lower-capability or lower-effort overrides are reserved for clearly mechanical bounded work with settled semantics and normally use no/selective history |
| wait_for_environment | codex-rs/core/src/tools/handlers/wait_for_environment.rs | 1 line | do-not-wait → no-need-to-wait |
| guardian denial text | codex-rs/prompts/src/model_messages/guardian.rs | reword, moved upstream | rejection follow-up as mechanism; timeout and review-failure text untouched |
| permissions: never | codex-rs/prompts/templates/permissions/approval_policy/never.md | reword | rejection mechanics |
| compact prompt | codex-rs/prompts/templates/compact/prompt.md | deliberately unchanged | Retains upstream compaction behavior |
| persistent mode | codex-rs/prompts/templates/persistent_mode.md | new upstream surface, unchanged pending review | Retains post-final follow-up authorization and stopping rules; wording and repetition remain candidates for review |
| memories write templates | codex-rs/memories/write/templates/memories/{stage_one_system,consolidation}.md | rewrite 1449→506 | Retains collaborative reasoning, decisions, rationale, and conceptual models; emphasizes deletion of superseded material; removes repeated rules; resolves conflicting preference guidance; adds Understanding sections while preserving schemas and contracts |
| memories read path | codex-rs/ext/memories/templates/memories/read_path.md | 1 passage | update-gate stated as mechanism; quick-pass/citation contracts untouched |
| memories v2 templates | codex-rs/memories/write/templates/memories/*_v2.md; codex-rs/ext/memories/templates/memories/read_path_v2.md | new upstream surface, unchanged pending review | opt-in version/dual-write path; custom v1 rewrites do not apply automatically |
| guardian charter | codex-rs/prompts/templates/guardian/{policy_template.md,policy.md} | unchanged after upstream move | Retains the detailed approval-review policy |
| guardian synchronous reviewer | codex-rs/ext/guardian-v2/src/sync_reviewer/policy_template.md | new upstream surface, unchanged | internal risk reviewer; preserve exact policy mechanics |
| shell/exec tool specs | codex-rs/core/src/tools/handlers/shell_spec.rs | unchanged | Retains platform-specific execution guidance |
| skipped as dormant | realtime/, tui ide_context, windows sandbox, older-model prompts, sample skills, imagegen | unchanged | revisit if enabled |

## September 22 resolver audit

The shared `codex-prompts` resolver owns bundled role, delegation, Guardian, and
permission defaults. Custom default wording moved with those surfaces. Explicit
configuration still wins over model-catalog defaults; configured delegation mode
stays independent of effort. Catalog role and tool overrides remain intentional
overrides, not another copy of the bundled wording.

Code Mode catalog descriptions retain runtime-owned bundle and bounded-parallel
call guidance. Empty descriptions still suppress it. The added supplement is
under 1,000 tokens; it introduces no variable-sized context.

P0 manual size-review finding: upstream adds configurable Guardian extra policy
and policy-template text without an individual-message size cap. These inputs
can exceed 1,000 tokens and, with sufficiently large configuration, 10,000 tokens.
The reviewer has a complete-request budget, which is not an individual-policy
cap. The merge retains upstream semantics rather than silently truncating a
security policy. No local extra policy or template override was enabled.
