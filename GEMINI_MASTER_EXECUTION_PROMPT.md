# Gemini CLI — Maya Mini v0.1 execution contract

You are the Rust implementation lead. Deliver the complete Maya Mini v0.1 application defined by the local master blueprint. Implement; do not produce another architecture or stop after planning.

## 1. Execution authority and immutable inputs

**When I submit this prompt to Gemini CLI for execution, I approve the blueprint revision identified below and authorize its complete implementation inside `Maya/`. Creating or storing this prompt alone does not execute or authorize implementation.** This authorization covers the declared application files, Cargo dependency resolution/build downloads, disposable test databases/projects and local verification under existing CLI permissions. It does not grant access to unrelated projects, personal databases or credentials; paid/live integrations, destructive actions, OS/toolchain installation, service activation and publication retain their specific approvals. Never bypass CLI permission controls.

Project root: `/home/thakur/projects/MiniAi/`

- `BP` means `/home/thakur/projects/MiniAi/MASTER_IMPLEMENTATION_BLUEPRINT.md`.
- `DOCS` means `/home/thakur/projects/MiniAi/MAYA_MINI_PRODUCT_DOCS_v0.1/`.
- `APP` means `/home/thakur/projects/MiniAi/Maya/`.
- Only APP may receive application source, tests, build scripts and release records. Preserve existing work. BP, this prompt, DOCS and the documentation ZIP are read-only. Do not create another plan, root policy file or replacement blueprint. Normal Cargo caches remain subject to CLI permissions; development/test runtime data uses disposable isolated locations, never personal XDG state.

Verify before editing:

```text
BP SHA-256: 34fce67da17adb224274aecc75a2196b4c13b63bbae4a7983624546bdbe51fda
DOCS/docs/00_APPROVED_FEATURE_SCOPE.md SHA-256: 963ff40c0fbd96a45b34947650474e50967cfc4d234eebf44eb275495117e693
```

A mismatch requires confirming the owner's intended revision before dependent writes; never silently replace the approved hash. Record both hashes in `APP/release/progress.md` when that file is created.

Authority: explicit owner instructions → approved feature scope → BP technical contracts → compatible source documentation. This prompt directs execution and does not amend BP. Apply **BP §1 PO-01** explicitly: its single-delivery override supersedes milestone/WP-only, per-WP stop/approval, extra-plan and partial-shipping instructions in DOCS/README.md, AGENTS.md, GEMINI.md, CLAUDE.md, PLANS.md, docs/12,13,16,18 and diagram08. Preserve all security approval and final owner acceptance requirements. Do not ask for routine permission at each internal checkpoint.

Scope is exactly **55 features**: A01–A05, B01–B08, C01–C05, D01–D09, E01–E09, F01–F06, G01–G05, H01–H08. Every feature includes behavior, validation, persistence where specified, UI/API wiring and acceptance evidence. One complete product delivery; no reduced feature release or deferred voice/browser implementation.

Unless explicitly labeled as a section of this prompt, section references below mean BP.

## 2. Start and context discipline for Gemini Flash

1. Inspect current paths, applicable agent policies, APP contents and version-control state without modifying anything. Do not assume the historical empty APP or Git status still holds. Never reset, overwrite or discard owner work.
2. Read `DOCS/AGENTS.md`, `DOCS/GEMINI.md`, `DOCS/README.md`, `DOCS/PLANS.md`, `DOCS/docs/00_APPROVED_FEATURE_SCOPE.md` and `DOCS/docs/12_SDLC_WORKFLOW.md`. Read BP §§1–2, 3, 15, 17.1 and 18 in bounded chunks (target ≤150 lines or 12 KiB per read; split longer sections and finish truncated output). Identify global boundaries before writing code. For other source documents, use BP §1.1's index and the module currently being implemented; older draft schemas/routes/milestones do not replace BP contracts.
3. Follow this prompt's §3 below immediately. Read all BP sections across the delivery; record section-read status. Do not dump BP/DOCS or the repository repeatedly into context. Locate headings with `rg -n '^#{1,3} '`, then read the needed range; continue truncated output before relying on it.
4. Work on one bounded behavior/file cluster at a time. Keep active context to this prompt's constraints, progress's current state, relevant BP definitions, the actual affected files/direct consumers and focused failure output. Load exact definitions when needed; summaries never replace interfaces/security rules.
5. Keep updates short: checkpoint/file, evidence, blocker if any, next action. Persist detailed evidence on disk; avoid repeated plans, full code dumps and successful compiler logs. Do not delegate implementation to another coding agent or paid service without authorization; retain one implementation writer.

## 3. Exact execution order and file contract

**BP §15.1 is the exact ordered file-creation/completion ledger. Execute its rows 1→11, and each row left→right.** Use the actual paths from that table; the following is an index, not permission to reorder or invent files. All listed relative paths are under APP.

| Checkpoint | Work and controlling BP sections |
|---|---|
| 1 | Cargo/toolchain/lockfile/config documentation and `release/progress.md`; §15.1 row1, §§2.1/4.1. Resolve compatible exact versions once; generate Cargo.lock with Cargo. |
| 2 | `error/types/config`, exact `migrations/0001_initial.sql`, DB worker, initial common/database tests, library declarations; row2, §§3/4.2/4.4/5. |
| 3 | Secrets/auth/audit/policy/approvals/scope/artifacts/security tests; row3, §§3/5–6/7.3–7.4/9. |
| 4 | Sandbox/process supervisor/checkpoints/process and workflow tests; row4, §§7/9.3/13. |
| 5 | Provider registry/five wire protocols/test fixtures/API coding/Codex relay; row5, §§3/8/10.1–10.2. |
| 6 | Conversations/memory/tasks/scheduling/reports/schedule tests; row6, §§3/5–6/11.1/12–13. |
| 7 | File/project/desktop/browser/voice/resources/backup adapters and integration tests; row7, §§6–7/9–10/14. |
| 8 | Workflow/assistant/protocol/state/HTTP/application startup and API tests; row8, §§2.3/3/7–9/11/13. |
| 9 | All ten UI files and browser test harness; row9, §§4.3/8/9.1/11.2. |
| 10 | Full feature/baseline/performance/release tests, smoke/measurement scripts; row10, §§12–14/16. |
| 11 | Desktop/user-service packaging assets, package script and final release records; row11, §§4.1/4.4/16–18. |

For **every file**, read its exact BP §4 row and applicable §§3/8 definitions first. Implement its stated responsibilities, signatures, inputs/outputs, dependencies, errors, security checks and feature/test mapping. Revisit already-created files as dependencies become available. Keep shared types in their designated owner; use `state.rs` and dependency injection exactly as specified. Domain code must not depend on HTTP DTOs; registry/coding must not import their concrete adapter implementations. Use §2.3 plus §4's dependency contracts, not speculative layers.

Creation order permits declaration-only initial files; it does not require pretending later dependencies exist. Populate `tests/common/mod.rs` incrementally; its complete app fixture depends on checkpoint8. Export only implemented/compilable library modules. Add an intended failing behavioral test before completing its behavior when the needed test surface exists. Run early `cargo check --lib` and focused compilable tests; postpone only unavailable-dependent assertions and record them NOT_RUN with their dependency. Do not move files out of order, introduce placeholder success, gate away mandatory features, or mark a checkpoint fully verified while its deferred tests remain. Revisit every deferred assertion once dependencies exist; all-target verification is mandatory after assembly.

## 4. Implementation invariants

Use BP §§2–11 exactly: one Rust2024 package/process, Tokio, Axum, bundled SQLite/rusqlite, static HTML/CSS/vanilla ES modules, Ubuntu baseline. No React, Electron, Docker requirement, local LLM, Node/Python runtime, extra daemon or always-listening microphone.

- Preserve every §3 type/enum/signature and §8 field/route/error/auth rule. Implement **84 frontend API method+route pairs plus one private relay endpoint**, **30 tables**, and the §4 manifest: **65 production/build files +15 test files +4 release records =84 planned files**. Counts are audits, not permission to create empty files or guessed APIs. Generated build/package outputs are distinct from planned source paths.
- SQLite schema/indexes/triggers/PRAGMAs, transactional events/idempotency, revisions, migrations, safe backup/restore and retention follow §§5–6. No blocking DB/process work on Tokio workers; no DB transaction across external awaits. No plaintext secrets in DB or exports.
- Implement actual provider/CLI/CDP/audio/desktop adapters where specified, not canned production responses, fake telemetry, empty command execution or unconditional “unavailable” stubs. Test fakes/mock servers belong only to tests. Real capability probes and safe fallback are product functionality; missing setup/account/permission differs from an implementation defect.
- Follow §10's deliberate limitations: default AI-site submission and unproven subscription-auth CLI automation remain unsupported; implement manual handoff/text/API fallbacks. Never scrape protected chat responses, infer API entitlement, invent pricing/model IDs/flags, or silently choose a paid fallback. Verify version-sensitive contracts using installed help and primary official references; pin the observed adapter version and evidence. Missing optional tools never trigger automatic installation.
- Implement exactly four tabs **Chat, Tasks, Activity, Workspace** and six Workspace groups (§11.2), keyboard/text fallback, bounded rendering and safe text insertion. Every server-backed action must call its exact §8 endpoint and show real server state; local controls follow §§4.3/11.2. Local tasks/notes/reminders/planner/reports/settings/memory remain usable offline. Distinguish manual/observed/verified progress; unknown usage/cost stays unknown.
- Correctness/security precede optimization. Enforce every §2.2 bound, explicit deadlines/cancellation, lazy initialization, next-deadline scheduling, bounded queues/streams/logs and on-demand subprocesses. Two runtime workers/max four blocking workers; one edit job, two API calls/two subprocesses at most. Profile rather than guess. §14's normal-workload Maya-owned combined RSS target is **<500,000,000 bytes**; report Chromium/IDE/coding CLI/separate speech workers separately and count Maya-owned helpers honestly.

## 5. Security and runtime approval boundaries

These requirements are mandatory Rust behavior, independently tested; neither this implementation authorization, a prompt, chat “yes”, UI session nor test mode is a product runtime execution grant. Use **BP §§7–9 and §17.1** without weakening them.

- Loopback-only authenticated API, exact Host/origin/Fetch Metadata/CSRF checks, bootstrap/session expiry, restrictive CSP and authenticated fetch-SSE. Preserve §8.1's legitimate absent-Origin safe-read rule. No tokens in query strings/browser persistent storage/logs; reload reauthentication via peer-UID control socket never cancels jobs.
- Separate immutable plan and approval-request digests, explicit scoped action sets, expiry/revocation and current project/tool/model/command/policy revisions. Preserve approval≠Start, typed continuations, context-upload consent and separate destructive/restore/history/no-tests-waiver grants. Pause/cancel must work after execution grants expire. Never fabricate real owner approval records or reuse example/template grants. Tests may simulate owner decisions only in disposable fixtures through normal validated paths; they cannot weaken production enforcement.
- Host dirfd/openat2 scope checks, protected snapshots, real OS sandbox and sterile stage. Validate API edit manifests/actual CLI changes; tests/build scripts are untrusted. Never use cwd/prompts/worktree/audit-after-edit as confinement. Codex uses the bounded run-scoped relay and native+outer isolation only after required eligibility probes; no real keys/auth files in children, unsafe flags or unsandboxed fallback.
- Journal each effect before execution. Preserve displaced originals/concurrent owner edits during promotion and rollback; reconcile partial/crashed/uncertain effects without replaying writes or claiming cross-file atomicity. Report `cancelling` until owned descendants are confirmed stopped; PID alone is insufficient identity. No silent rollback, reset/stash/clean, privilege escalation, push, arbitrary shell or host deletion.
- Keyring or explicit session-only credential handling, destination validation/SSRF controls, redaction before persistence/display and prompt-injection defenses. No credential discovery/exfiltration. Voice captures only by gesture; remote speech needs approved Rust cloud transport, browser-local speech requires `localService=true`. Preserve transient-payload cleanup and deletion/summary invalidation semantics.

During development, tests use disposable fixtures and normal application approval paths. Real paid/provider calls, personal project edits, desktop/profile access and sensitive captures need explicit applicable owner consent. `MAYA_RUN_LIVE_INTEGRATION=1` selects eligible tests; setting it is not consent, credentials or permission by itself.

## 6. Continuous verification, debugging and durable recovery

For each compilable behavior: create its failing assertion, implement the smallest contract-compliant change, format, check and run the relevant focused/unit/integration/negative cases from BP §§12–13. After shared type/schema/policy/API changes, rerun affected consumers and regression tests. As targets become available, broaden to all targets; never silence warnings, delete assertions, relax security or ignore mandatory tests to obtain green output.

On failure: capture the exact command/exit and minimal redacted diagnostic; reproduce with the smallest fixture, inspect the relevant contract/call boundary, fix the cause, rerun that case then affected regression checks. Do not loop identical commands, guess unrelated changes, add dependencies or rewrite architecture to hide an error. A genuine BP contradiction/unsafe boundary change requires a concise evidenced owner question; record exact sections, impact and smallest proposed correction, stop only dependent work and continue independent authorized work. Preserve BP unchanged. Missing compiler/system prerequisite requires prerequisite-specific approval; missing credentials/hardware stays an honest verification blocker.

Maintain only the planned durable records under `APP/release/`:

- `progress.md`: approved hashes/authorization reference, section-read ledger, checkpoint/file cursor, touched files, versions, 55-feature implementation/evidence checklist, all120 test-status checklist, deferred assertions, defects/blockers, subprocess/checkpoint/reconciliation identities and **one exact next action**. Keep a short current-state block at the top; keep interim results here with redacted generated logs under `APP/target/verification/`, rather than repeating narratives. Update after each meaningful edit/test, and before context pressure/interruption. Atomic write/rename; no secrets.
- Create `validation.json`, `measurements.csv` and `acceptance.md` for the first time in checkpoint11, at their exact §15.1 row11 positions; until then keep actual results/deferred-check status in progress and generated verification logs. `validation.json`: §13 case IDs mapped to file/function/features, actual command/time/environment/exit/output evidence and PASS/FAIL/SKIP/NOT_RUN; record capability UNAVAILABLE with its reason separately from live-test success.
- `measurements.csv`: actual §14 samples only; `acceptance.md`: §16 UAT, security/packaging/update/rollback evidence, limitations and owner acceptance status. Populate them only with collected evidence; missing measurements/UAT remain explicitly unverified.

After restart/compaction, read this prompt, progress's current-state block and recorded hashes; verify actual APP files/diff, latest evidence and active process identities, then load only the needed BP contracts and resume the exact next action. Never reset progress/re-scaffold/reinstall or replay an uncertain effect. A context limit is a continuation boundary, not a partial product release; save recovery state and identify the continuation point if execution cannot continue in the current session. Completed checks do not need repeating unless changes or lost evidence invalidate them.

## 7. Final validation and delivery gate

Implement the **120 named scenarios**: BP §12's55 feature cases, §13.1 T-01–T-24, §13.2 X-01–X-41, in §4.4's exact files. Parameterized assertions are allowed; the named count is not proof of coverage. Add tests inside planned files as needed for discovered regressions, never replace the required120. Run frontend harness, keyboard/accessibility, security/adversarial, workflow/cancellation/crash, DB migration/backup/restore, offline, resource/stress and target integration checks specified in §§13–16.

Run from APP once the complete targets exist, preserve actual outcomes, fix failures and rerun affected gates:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
bash scripts/smoke.sh target/release/maya-mini
cargo test --release --test performance -- --ignored --nocapture
```

Use `cargo fmt --all` to apply formatting, then check again. Execute §16's isolated startup/restart/auth/CSRF/offline smoke assertions and eight owner UAT actions, plus §14's exact measurement protocol, sampler and cleanup checks. Demonstrate the **real configured API EditManifest coding path** with approved scoped changes and actual tests on at least one supported route, as BP §16 requires; eligible live CLI evidence is additional when claimed. Mocks alone do not establish live integration. If consent/setup is unavailable, complete all independent implementation/verification and record the blocked acceptance step; never fake it or claim release complete. Owner UAT/acceptance must be performed by the owner, not self-signed by Gemini.

Audit feature IDs, file manifest, every route/DTO/frontend interaction, SQL/migrations, security boundaries, deferred tests, evidence and §17's nine runtime conditions. Required behavior/security tests and normal-case resource gates must pass; optional integration absence is acceptable only exactly as BP permits with tested truthful fallback. Missing mandatory evidence remains blocking.

When pre-packaging checks pass, run `bash scripts/package.sh` under §16: tested executable, embedded assets/migrations, documentation, optional deployment files, checksums and sanitized evidence. This creates a local reviewable artifact only. Do not publish, push/tag, install system-wide or enable services; do not package a failing/unverified build as accepted. Final owner acceptance remains required.

Finish with a concise report linking executable/package and four release records, feature/file/API/table/test totals, actual PASS/FAIL/SKIP/NOT_RUN evidence, measured core/external resources, remaining setup/security/acceptance blockers and exact startup command. Distinguish **implemented**, **verified**, **unavailable**, **blocked** and **owner-accepted**. Claim complete Maya Mini v0.1 only when BP §16's entire release gate is satisfied.

Begin with the read-only revision/state checks, then execute the ordered single delivery. Do not stop at a plan or a successful scaffold.
