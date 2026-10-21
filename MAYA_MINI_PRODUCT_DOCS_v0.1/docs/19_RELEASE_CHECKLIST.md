# 19 — Maya Mini v0.1 Release Acceptance Checklist

Mark an item **PASS** only with verifiable evidence and repository reference; **OPEN** denotes not completed. This documentation package **does not** certify that any implementation test has run.

## A. Scope and configuration
- [ ] 55 approved feature IDs have test/implementation evidence or owner-approved scope change.
- [ ] Exactly 4 tabs and 6 Workspace configuration groups verified.
- [ ] Name/language/provider roles/voice/project/safety preferences persist after restart.
- [ ] No local inference model, no heavy bundled renderer, no always-listening mic.

## B. Personal assistant and AI
- [ ] Tasks/reminders/check-ins/planner/reports work with AI offline.
- [ ] Every progress item labels manual/observed/verified evidence.
- [ ] Only configured providers/models/explicit fallbacks run, with timeouts and usage provenance.
- [ ] Conditional website bridge gracefully degrades; coding workflow does not depend on protected chat website extraction.
- [ ] STT/TTS (if selected) require explicit activation and fall back to text.

## C. Coding execution, safety and recovery
- [ ] Plan must be approved before editing, tied to project paths and immutable plan hash.
- [ ] Path traversal, symlink, changed plan, expired authorization attempts denied.
- [ ] Privileged/destructive commands cannot execute without separate explicit approval (out-of-scope actions denied).
- [ ] Real diff/test/exit status captured; failures never presented as success.
- [ ] Cancel/restart/partial edit recovery prevents duplicate side effects.
- [ ] Permission and redaction audit log recorded without exposing API keys.

## D. Security and reliability
- [ ] Loopback-only API with cross-origin CSRF/session negative test.
- [ ] OS secret storage or secure session-only fallback, zero plaintext secret artifacts.
- [ ] Database migration, backup/restore and event/restart tests pass.
- [ ] Browser CDP (if used) isolated, restricted and terminated when finished.
- [ ] Tests T-01 to T-24 in `10_TEST_STRATEGY.md` have evidence; all security blockers cleared.

## E. Resource and user experience
- [ ] Normal-case Maya-owned resident RSS measured `<500 MB` with reproducible fixtures.
- [ ] Browser/IDE/coding CLI/voice memory reported separately; no all-inclusive 500 MB claim.
- [ ] Idle CPU measurement shows no busy polling.
- [ ] Large output/event/history loads bounded; leak/stress scenarios verified.
- [ ] Keyboard controls, text fallback, truthful progress/cancel state accepted.

## F. Packaging and owner acceptance
- [ ] Reproducible Ubuntu install/update/rollback path tested.
- [ ] Setup instructions list real versions, authentication and billing distinctions.
- [ ] Owner UAT `17_USER_STORIES.md` conducted, defects tracked.
- [ ] Final release tag, changelog, checksums and remaining limitations recorded.
- [ ] Owner signs off v0.1 release.

**Release state at documentation handoff:** **NOT RELEASED / NOT IMPLEMENTED**.
