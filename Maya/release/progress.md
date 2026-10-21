# Maya Mini v0.1 — Implementation Progress & Verification Ledger

## Codex Takeover Audit — 2026-10-21 (authoritative current block)

- **Recovery snapshot:** `release/snapshots/pre-codex-takeover-20261021T000000+0530.tar.gz`; 58 non-build entries; SHA-256 `dbc9aa4d8466a81b2d2aa890fa525816580b57f468deb4fcb6c622830dbc8ac4`.
- **Repository state:** Git is present on `main`; verified checkpoints `829546b`, `5a6897e`, and `c18c985` follow owner-created initial commit `1ab1c6b86e3ad26411d55cccbf726d3840ebb0b8`. Remote `origin` points to the public MiniMaya repository. Nothing has been pushed. Never reset to the remote snapshot. The pre-takeover archive remains an independent recovery point. Uncommitted work must be checked with `git status` before every checkpoint.
- **Blueprint/scope revisions checked:** blueprint `34fce67da17adb224274aecc75a2196b4c13b63bbae4a7983624546bdbe51fda`; scope `963ff40c0fbd96a45b34947650474e50967cfc4d234eebf44eb275495117e693`.
- **Implementation state:** checkpoints 1–7 and the production files in checkpoint 8 compile. Checkpoints 8–9 remain **PARTIAL** pending complete workflow/browser acceptance. Checkpoint 10 now has frontend, HTTP, performance, and release smoke coverage plus all three planned scripts. `tests/feature_acceptance.rs` exists with A01–D09 (27/55 named feature cases) passing; E01–H08 and `baseline.rs` remain. Checkpoint 11 has desktop/service packaging inputs; final evidence and gated package execution remain pending.
- **Present production integration:** `src/workflow.rs`, `src/assistant.rs`, `src/protocol.rs`, `src/state.rs`, `src/http/{events,routes,mod}.rs`, `src/app.rs`, `src/main.rs`, all ten planned `web/` assets, and `tests/frontend.{html,js}` exist. The four-tab shell and six Workspace groups render. Workspace now round-trips identity, providers/models/roles, write-only credentials, complete voice fields, projects/tools/commands, reminder/report/security/browser/resource settings, and backups against real handlers. Chat consumes live authenticated job-state events and reloads job-linked responses. Real Chromium DOM acceptance remains open. All three release scripts, both packaging files, `tests/performance.rs`, and `tests/release.rs` exist. `tests/feature_acceptance.rs`, `tests/baseline.rs`, and final generated evidence remain absent.
- **HTTP/API coverage:** 84/84 documented frontend API method+route pairs are registered and the route-set contract test passes. Registration is not release acceptance: some handlers still return capability/approval placeholders, strict duplicate-key/rate-limit/SSE-heartbeat requirements remain open, and the frontend does not yet exercise all contracts.
- **Database:** the v1 migration defines 30 tables. Confirmed project, voice, artifact-directory, and backup/restore persistence defects have been fixed and covered by the existing integration suite; complete schema/retention/recovery acceptance remains open.
- **Feature status:** 0/55 approved features are release-verified. The persisted coding path now implements discovery, scan, context consent, AI plan generation, immutable digest-bound plan approval, checkpoint/stage execution, registered tests, deletion/no-test approval gates, promotion, and report states. External chat now returns `202` with a persisted `chat` job before provider I/O, uses cancellable `generate`/`record_response` steps, binds exact provider/model revisions, and links the resulting message to the job. Safe never-started chat jobs resume after restart; interrupted provider effects reconcile rather than replay. Standalone registered commands bind their project/command scope and run only in a disposable stage. Export has a persisted `export_data` operation and selection-bound dedicated endpoint. Remaining release blockers include end-to-end coding fixtures, real frontend/backend browser acceptance, recovery/retention coverage, and the 120-case release mapping.
- **Fresh commands:** current-tree `cargo fmt --all -- --check` and `cargo check --all-targets --locked` PASS. Escalated `cargo test --locked --test feature_acceptance -- --test-threads=4` PASS for all 27 currently implemented cases A01–D09. Earlier full-suite `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` (44 passed/2 ignored), release performance tests, release build, smoke, and frontend checks passed before this feature-contract increment and must be rerun at the next complete checkpoint. Final long-duration release evidence remains pending.
- **Preliminary measured runtime (not the final §14 fixture):** release startup-to-health 124 ms; 10 one-second idle samples mean/peak core RSS 13,410,304 B and mean CPU 0.000000% of one core; 10 half-second unthrottled health-load samples mean RSS 13,499,597 B, peak RSS 13,508,608 B, mean CPU 6.650944%. Raw rows are in ignored generated `release/measurements.csv`. Host metadata and the required 10-minute/seeded normal/API/coding/recovery fixtures are still required before release claims.
- **Repaired failures:** project timestamps and voice schema queries now match the migration; artifact storage creates its private directory; backup uses the read queue and restore uses a validated staged atomic database swap instead of backing up through an open write transaction; stale file/browser fixtures were corrected to match schema and honest capability fallback.
- **Security verification:** localhost binding, Host/Origin/Fetch-Metadata checks, bearer/CSRF authentication, strict duplicate-key rejection, 120/minute session mutation limiting, four-stream admission, SSE heartbeat, idempotency persistence/startup uncertainty, job/plan/project/digest/scope-bound approvals, scoped file/process services, audit logging, private control socket, and focused negative tests exist. Coding manifests enforce approved actions/paths, exact replace/delete preimages, staged regular-file/no-symlink rules, file/content bounds, and approved baseline binding. The direct test-permit constructor is absent from release builds. Provider create and patch require valid kind/protocol data and HTTPS, except explicitly acknowledged loopback HTTP; endpoint changes invalidate stored credentials. Full workflow approval continuity, stream lag/replay evidence, prompt-injection E2E, and release sandbox evidence remain open.
- **External integrations:** no live paid-provider, Codex, CDP, voice, keyring, desktop, or sandbox success is claimed. Their availability must be detected at runtime; unsupported/needs-setup results require the blueprint fallbacks.
- **Classifier repair:** A03/D01 rule parsing now produces constrained drafts for all five intent classes, keeps ambiguous reminders unsaved until an exact offset-bearing time is supplied, and emits approval-requiring typed drafts for clipboard and URL actions. Prompt-injection text remains an inert question. The focused unit test, formatting, and Clippy with warnings denied pass; URL case is preserved.
- **Contract/security repairs from feature acceptance:** task/reminder input now uses offset-bearing RFC3339 at the wire boundary while responses remain UTC milliseconds; nullable patches distinguish omission from explicit clear; flattened list queries decode their documented top-level fields. Host validation now covers public health/static routes as well as protected APIs. Browser prompt submission is honestly unavailable without a compiled supported adapter, and unsafe URL/action shapes are rejected before approval creation. Duplicate schedule delivery returns the existing occurrence, and owner task PATCH cannot claim observed/verified progress.
- **Current exact action:** implement and run E01–H08 in `tests/feature_acceptance.rs`, create `tests/baseline.rs`, close coding workflow/recovery and real browser UI acceptance gaps, then run the long-duration release fixtures and produce final evidence.
- **Historical note:** all sections below this block are Gemini-era claims retained for provenance. They are superseded wherever they conflict with this audit.

## Current State Block
- **Active Checkpoint:** Checkpoint 6 complete -> Moving to Checkpoint 7
- **Current Cursor:** Row 6 complete (`src/conversation.rs`, `src/memory.rs`, `src/tasks.rs`, `src/schedule.rs`, `src/reports.rs`, `tests/scheduling.rs`), starting Row 7 (`src/fileops.rs`, `src/project.rs`, `src/desktop.rs`, `src/browser.rs`, `src/voice.rs`, `src/telemetry.rs`, `src/backup.rs`, `tests/integrations.rs`)
- **Compilation Status:** `cargo check --lib` passed; `cargo fmt --check` passed; `cargo clippy --tests` clean; `cargo test` passed 27/27 tests (`database.rs`: 5, `security.rs`: 7, `processes.rs`: 3, `workflow.rs`: 2, `providers.rs`: 5, `scheduling.rs`: 5)
- **Next Exact Action:** Create Checkpoint 7 files in left-to-right order: `src/fileops.rs`, `src/project.rs`, `src/desktop.rs`, `src/browser.rs`, `src/voice.rs`, `src/telemetry.rs`, `src/backup.rs`, `tests/integrations.rs`.

---

## Authorization & Immutable Hashes
- **Blueprint (BP) SHA-256:** `34fce67da17adb224274aecc75a2196b4c13b63bbae4a7983624546bdbe51fda`
- **Scope Doc SHA-256:** `963ff40c0fbd96a45b34947650474e50967cfc4d234eebf44eb275495117e693`
- **Execution Authority:** Gemini Master Execution Prompt / BP §1 PO-01 single-delivery authorization.

---

## Section Read Ledger
- [x] BP §1 Authority, process override and audited evidence
- [x] BP §2 Fixed engineering decisions and nonfunctional contract
- [x] BP §3 Canonical types, Rust interfaces and semantic rules
- [x] BP §4 Complete application file manifest
- [x] BP §5 Complete SQLite schema and persistence contract
- [x] BP §6 Retention, deletion, export, backup and restore
- [x] BP §7 Workflow, approvals, execution and crash recovery
- [x] BP §8 Exhaustive HTTP and frontend contract
- [x] BP §9 Host-enforced security design
- [x] BP §10 External integration contracts and capability fallbacks
- [x] BP §11 Personal assistant algorithms and minimal UI behavior
- [x] BP §12 Complete approved-feature inventory, ownership and acceptance
- [x] BP §13 Complete finite test specification
- [x] BP §14 Performance verification and resource accounting
- [x] BP §15 Gemini's single-delivery implementation contract
- [x] BP §16 Build, startup, smoke, packaging and final acceptance
- [x] BP §17 Resolved decisions, runtime blockers and remaining risk
- [x] BP §18 Validation record and final handoff

---

## Checkpoint Progress Ledger (BP §15.1)
- [x] **Checkpoint 1:** Cargo.toml, rust-toolchain.toml, .gitignore, Cargo.lock, README.md, CHANGELOG.md, release/progress.md
- [x] **Checkpoint 2:** src/error.rs, src/types.rs, src/config.rs, migrations/0001_initial.sql, src/db.rs, tests/common/mod.rs, tests/database.rs, src/lib.rs
- [x] **Checkpoint 3:** src/secrets.rs, src/auth.rs, src/audit.rs, src/policy.rs, src/approval.rs, src/scope.rs, src/artifacts.rs, tests/security.rs
- [x] **Checkpoint 4:** src/sandbox.rs, src/process.rs, src/checkpoint.rs, tests/processes.rs, tests/workflow.rs
- [x] **Checkpoint 5:** src/provider.rs, src/providers/mod.rs, src/providers/openai.rs, src/providers/gemini.rs, src/providers/anthropic.rs, tests/providers.rs, src/coding.rs, src/codex.rs
- [x] **Checkpoint 6:** src/conversation.rs, src/memory.rs, src/tasks.rs, src/schedule.rs, src/reports.rs, tests/scheduling.rs
- [x] **Checkpoint 7:** files implemented; service tests pass; release feature acceptance remains pending
- [ ] **Checkpoint 8 (PARTIAL):** all planned files exist and compile; route-set tests pass; full persisted coding execution, strict HTTP controls, and runtime acceptance remain
- [ ] **Checkpoint 9 (PARTIAL):** all ten production web assets exist with four tabs/six groups; editable configuration flows and `tests/frontend.{html,js}` remain
- [ ] **Checkpoint 10:** tests/feature_acceptance.rs, tests/baseline.rs, tests/performance.rs, tests/release.rs, scripts/smoke.sh, scripts/measure.sh
- [ ] **Checkpoint 11:** packaging/maya-mini.desktop, packaging/maya-mini.service, scripts/package.sh, release/validation.json, release/measurements.csv, release/acceptance.md

---

## 55 Approved Features Implementation & Verification Status
- [ ] A01: Identity customization
- [ ] A02: Chat stream & message history
- [ ] A03: Intent classifier & action drafts
- [ ] A04: Conversation context & model override
- [ ] A05: Single-process daemon & UI server
- [x] B01: Model provider registry & adapters
- [x] B02: CLI tool execution adapter
- [x] B03: Custom model registration
- [x] B04: Model role assignments
- [x] B05: Per-project/conversation model override
- [x] B06: Resilience & retry with backoff
- [x] B07: Token & cost tracking journal
- [x] B08: Sequential model chaining
- [ ] C01: CDP browser controller
- [ ] C02: Web AI quick-access shortcuts
- [ ] C03: Assisted AI web chat
- [ ] C04: Manual response handoff
- [ ] C05: Web research & page inspection
- [ ] D01: Natural language task creation
- [ ] D02: Hierarchical task manager
- [ ] D03: Daily & upcoming planner
- [ ] D04: Desktop reminders & notifications
- [ ] D05: Proactive task check-in prompts
- [ ] D06: Task progress tracking
- [ ] D07: Structured daily work journal
- [ ] D08: End-of-day & weekly summary reports
- [ ] D09: Reschedule & postpone workflows
- [ ] E01: Project workspace registration
- [ ] E02: Project context scanner
- [ ] E03: Multi-step coding planner
- [ ] E04: Step-by-step approval gate
- [ ] E05: Isolated workspace execution
- [ ] E06: Test & verification runner
- [ ] E07: Snapshot & rollback checkpointing
- [ ] E08: Live coding session activity monitor
- [ ] E09: Task handoff & artifact generator
- [ ] F01: Desktop app launcher
- [ ] F02: Resource footprint monitor
- [ ] F03: Local file explorer & previewer
- [ ] F04: Approved workspace command runner
- [ ] F05: Clipboard manager & actions
- [ ] F06: Quick web search & URL opener
- [ ] G01: Push-to-talk voice input
- [ ] G02: Speech-to-text transcription
- [ ] G03: Text-to-speech audio output
- [ ] G04: Multilingual voice support
- [ ] G05: Audio device selector
- [ ] H01: Local-first SQLite persistence
- [ ] H02: User memory & preferences store
- [ ] H03: Secure credential storage
- [ ] H04: Action permission tiers
- [ ] H05: Crash recovery & state resumption
- [ ] H06: Action audit log & tamper evidence
- [ ] H07: Resource budget & memory governor
- [ ] H08: Data export & privacy controls

---

## 120 Planned Scenarios Status (55 Feature + 24 Baseline + 41 Extended)
- 55 Feature tests (`feature_a01` .. `feature_h08`): PENDING Checkpoint 10
- 24 Baseline tests (`baseline_01` .. `baseline_24`): PENDING Checkpoint 10
- 41 Extended tests (`extended_01` .. `extended_41`): PENDING Checkpoint 2..10

---

## Deferred Assertions & Blockers
- None currently.

## Subprocess / Reconciliation Identities
- None active.
