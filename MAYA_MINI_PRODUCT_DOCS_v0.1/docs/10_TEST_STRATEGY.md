# 10 — Test & QA Strategy

## Test layers
| Layer | Tests | Mocking and evidence |
|---|---|---|
| Rust unit | parser, scheduler timezone/recurrence, model routing, state machine, policy, path guard, retention | deterministic fake clock/filesystem/provider; test IDs |
| DB integration | migrations, foreign keys, task persistence, audit correlation, crash recovery | temporary SQLite on real filesystem |
| HTTP integration | auth/CSRF, data validation, jobs/events, pagination, unknown/disabled providers | loopback test server, token isolation |
| CLI adapter | installed binary detection, actual exit status, cancel/timeout, massive logs, denied writes | mocked CLI then optional real installed CLI with tiny sandbox repo |
| UI E2E | four tabs, workspace save/restart, task CRUD, approval/denial, logs, keyboard navigation | existing browser, no mandatory browser automation at runtime |
| Security | symlink traversal, malicious HTML/provider instruction, origin spoofing, secret leak, nonapproved edits | fail-closed assertions; must pass before milestones involving actions |
| Performance | RSS + CPU idle/workload, limit warnings, child-process separation | `11_PERFORMANCE_BUDGET.md`, reproducible time series |
| UAT | scenarios J1–J5 against named acceptance cases | owner walkthrough + screenshots/logs, no fake success |

## High-priority test cases (minimum)
| ID | Scenario | Expected |
|---|---|---|
| T-01 | Update identity/model/voice, restart | exact persisted nonsecret values, masked secrets |
| T-02 | Create task with due date, recurring reminder, check-in | fires/saves once per occurrence, notifies as configured |
| T-03 | Enter 40% manual progress, create report | report attributes `manual`, no manufactured verified completion |
| T-04 | Offline provider | tasks/reports work; model call fails clearly |
| T-05 | Disabled fallback provider | never contacted |
| T-06 | Ask for project edit without approved plan | no project file touched |
| T-07 | Approve plan, then modify plan content | approval invalidated |
| T-08 | Approve `src/`, attempt `../secrets`, symlink out | deny before operation; log event |
| T-09 | CLI edits partially and exits nonzero | failed/needs-reconciliation, actual diff captured |
| T-10 | Crash during editing, restart | never automatically duplicate edit |
| T-11 | Run nonprivileged tests | test result and exact command/exit recorded |
| T-12 | Attempt sudo/delete/push | separate critical confirmation required; prohibited classes denied |
| T-13 | Malicious website POSTs to localhost | blocked by server-side cross-origin/session protections |
| T-14 | Export memory/settings | valid export, secret values absent |
| T-15 | Browser integration denied/unavailable | manual handoff or supported API/CLI path still usable |
| T-16 | Mic access denied | text chat remains usable |
| T-17 | UI lost while job runs | reconnect shows persisted job and true state |
| T-18 | Output 100 MB from child process | bounded log truncation, stable core RAM, visible warning |
| T-19 | RSS idle, task workflow, API, CLI | no false under-500MB claims; child memory shown separately |
| T-20 | Daylight saving / India timezone scheduling | correct calendar occurrence; UTC storage |
| T-21 | Work hours check-in policy | no unauthorized duplicate/out-of-hours prompts |
| T-22 | Child cancellation not immediate | `cancelling` until process exit; no false status |
| T-23 | Change registered project root | revoke old permissions; validate new scope |
| T-24 | UI keyboard navigation | four tabs/settings/approvals accessible and labeled |

## Entry/exit criteria
- Per work package: requirements link + tests designed **before** edits; minimum focused test + negative branch + docs update.
- Milestone completion: required cases green, no critical security issues, source IDs and evidence updated, cross-module regression green.
- Release: all T-01 through T-24 evaluated and either **pass** or transparently documented as a release blocker/owner-accepted noncritical limitation **within original scope**; security blockers never waived.

## Suggested commands only after code exists
`cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; integration/UI harness commands recorded by actual implementation.

## Evidence standard
For each test: commit hash, system/tool version, executable command, time, actual output, PASS/FAIL/SKIP reason, affected requirement ID. A test never becomes 'passed' merely because an agent stated it would pass.
