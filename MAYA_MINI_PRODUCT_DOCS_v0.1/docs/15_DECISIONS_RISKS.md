# 15 — Architectural Decision Records (ADRs), Risks, Open Questions

This register distinguishes **source-approved choices**, derived **proposals**, and still **open decisions**. Never mistake a proposal for user sign-off.

## ADR summary
| ADR | Decision | Status | Rationale / consequence |
|---|---|---|---|
| ADR-001 | Rust backend; Tokio + Axum | **Scope-approved** | user prioritizes low overhead over dev speed; actual RSS still measured |
| ADR-002 | SQLite local-first | **Scope-approved** | no separate DB service; transactional tasks/jobs |
| ADR-003 | Existing browser, static HTML/CSS/JS | **Scope-approved** | avoid Electron/render bundle and complex frontend |
| ADR-004 | External API/official coding CLI, no local LLM | **Scope-approved** | external heavy inference; clear billing distinctions |
| ADR-005 | Project plan+scope approval before coding | **Scope-approved** | user control and predictable edits |
| ADR-006 | Chromium AI bridge optional, permission-aware | **Scope-approved** | web interaction brittle; never required for coding path |
| ADR-007 | Loopback token + CSRF, separate approval hash | **Proposed technical design** | mitigates malicious page accessing local server |
| ADR-008 | Single Rust executable/modules, on-demand subprocesses | **Proposed technical design** | reduces resident processes/overhead |
| ADR-009 | SSE activity stream, REST for CRUD | **Proposed technical design** | compact browser updates; reconnection handling |
| ADR-010 | XDG config dirs, systemd --user optional | **Proposed technical design** | predictable Ubuntu integration |

## Key risks
| Risk | Likelihood / impact | Mitigation | Release gate |
|---|---|---|---|
| Unconfined coding CLI edits outside scoped project | Medium / Critical | test OS-level confinement/staging, deny run if unsafe, full diff audit | must demonstrate boundary in negative test |
| ChatGPT/Gemini/Claude web UI automation unsupported | High / Medium | site shortcuts + manual handoff; official API/CLI primary | core coding flow works without it |
| Browser or CLI memory spikes exceed user's machine capacity | High / High | separate process telemetry, on-demand launch, limit concurrency | transparent per-process resources |
| Authentication/plan limits differ across providers | Medium / Medium | detect installed version/capabilities, no implicit entitlement assumptions | documented setup and graceful error |
| Secret leakage in logs/AI prompts | Medium / Critical | keyring, scrub, prompt context filter, redaction tests | no known leakage |
| Recovery incorrectly duplicates edits | Medium / High | durable checkpoints, reconcile uncertainty, manual approve | interruption test passes |
| Notifications lost during laptop suspend | High / Medium | persist missed reminder, show on next launch | truthful UI state |
| Scope grows beyond lightweight v0.1 | High / High | approve only one WP at a time, track 55 IDs | milestone boundary review |

## Open decisions for owner and first engineer review
1. **Precise performance measurement:** define normal-load fixture and core process tree accounting; target remains `<500 MB`.
2. **Ubuntu baseline:** tested release, architecture (`x86_64` default), Wayland/X11/browser differences.
3. **Coding CLI adapter mode:** noninteractive CLI, supported SDK/app-server or other adapter after installed-version spike; permission and streaming capabilities vary.
4. **Filesystem confinement mechanism:** restricted copy/worktree, OS sandbox, agent-specific permissions; choose based on Ubuntu and actual installed CLI before promising a hard sandbox.
5. **Keyring unavailable:** fail closed for persisted secrets versus session-only credential prompt; default proposal is fail closed for persistence.
6. **Voice defaults:** initial STT/TTS providers and latency/privacy trade-off, with text always available.
7. **Reminder notification implementation:** desktop notifications across lock/suspend and missed-event behavior.
8. **Browser handoff:** exact allowed automation actions by site, whether website reading is viable; manual export fallback mandatory.
9. **CLI subscription vs API billing:** credentials, entitlements, rate limits must be verified at setup; do not assume a paid chat website subscription permits API usage.
10. **Retention window:** tasks, transcripts, audit redaction, backup duration and storage cap; propose defaults at M0.

## Approval record template
```text
ADR ID / Decision:
Proposed by:
Alternatives:
Security / RAM / CPU / development impact:
Evidence (benchmark/doc/source):
Owner approval date:
Code changes authorized? [No until associated work-package approval]
```
