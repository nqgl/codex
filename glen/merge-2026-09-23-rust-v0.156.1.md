# Rust 0.156.1 release-tag merge — September 23, 2026

This integration merges upstream tag `rust-v0.156.1` (`b412ff32c4`) into the
published custom harness. It uses the release tag, not the newer upstream
`main` head, and retains the fork's existing changes.

The tag has two commits beyond its common ancestor with the fork. Its GPT-6
Sol and Luna catalog entries already arrived through upstream commit
`49e95cc73f` in the previous merge. The two model entries are identical in
both histories. The release branch assigns GPT-5.6 Sol priority 6; the newer
catalog assigns priority 4. This merge keeps priority 4 and the existing
daemon-disabled focus-test fixture.

## Compatibility audit

The combined implementation does not change. The release tag bumps the Cargo
workspace version to `0.156.1`, but this fork retains `0.0.0`. A packaged
`0.156.1` build enabled the daemon updater in the CLI regression test. That
updater downloads the official installer, which could replace the custom
harness. Keeping the fork's build label preserves its prior update behavior.

No model-visible prompt, tool description, app-server API, state migration,
dictation, monitor, math, delegation, recall, or Reserve-fallback implementation
changes. Existing model-catalog and prompt snapshots remain aligned with the
combined source. Neither Cargo.lock nor the Bazel lockfile needs a change.

A focused test exposed an older assertion that quoted upstream default-mode
wording instead of this fork's concise equivalent. The test now checks the
fork's wording; the prompt itself is unchanged.

The release-version trial exposed the updater behavior. The release version and
its test-only assertion change were removed before finalizing this merge.

Merging the tag does not roll back upstream commits already in the fork. The
result is a custom harness with both the release-tag ancestry and the newer
integrations recorded in the September 22 merge.

## Validation

- Cargo metadata resolves with the retained custom version and lockfile.
- The Bazel dependency lock check passes with non-fatal upstream version
  advisories for `platforms` and `rules_cc`.
- The release-version trial ran 80 focused model-catalog, migration, daemon,
  and TUI checks: all passed after correcting one older custom-prompt assertion.
  Its broader CLI run passed 470 checks and exposed the updater change in the
  packaged-bootstrap check. The release-version trial was not retained.
- With the custom build label restored, all 53 selected model-manager and
  packaged-bootstrap checks pass. The complete Rust workspace suite is not run.
- Scoped lint and repository formatting pass. The CLI and Code Mode host build
  successfully. The CLI still reports the custom `0.0.0` label, and Cargo
  metadata validates against the unchanged lockfile. Tests are not rerun after
  the final lint and format pass.
