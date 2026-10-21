# 02 — System Requirements & Acceptance Criteria (SRS)

**Precedence:** `00_APPROVED_FEATURE_SCOPE.md` controls. This document operationalizes its requirements for testing; it must not introduce a new feature as mandatory.

## Verifiable functional contracts
| Req | Source IDs | System shall… | Acceptance scenario |
|---|---|---|---|
| FR-01 | A01–A05 | Offer configurable identity, text chat, intent, bounded context and local-only service | Save name/language; restart; classify five core intents; external interface cannot bind |
| FR-02 | B01–B08 | Register AI services/model roles, explicit fallback, bounded chain and provider usage records | Set chat/planning/coding role; unavailable provider produces visible error; disabled fallback never used |
| FR-03 | C01–C05 | Open permitted Chromium site/AI shortcuts, conditionally perform supported web interaction and permitted response handoff | Open signed-in profile; manual export fallback on unsupported page; no protected-content scrape dependency |
| FR-04 | D01–D09 | Persist tasks/subtasks/scheduling/check-ins/evidence/reports | Create task, reminder, progress; restart; overdue/daily/weekly report reflects persisted facts |
| FR-05 | E01–E09 | Register scoped project, read/review/plan, approve, run agent, verify, log, resume and hand off | With approval edit sample repo and run tests; without approval no edit; report diff/results |
| FR-06 | F01–F06 | Launch apps, inspect system, read scoped files, run governed commands and explicit clipboard actions | Read allowed file; reject traversal/foreign folder/sudo; open VS Code if installed |
| FR-07 | G01–G05 | Offer opt-in push-to-talk STT, selected TTS/voice/language/device and text fallback | Mic denied → text works; voice unavailable → readable response; voice choice persists |
| FR-08 | H01–H08 | Store local history, protect credentials and scope, expose recovery/auditing/resource/export | Secrets masked; delete/export excludes secrets; cancel/restart no duplicate file side effects |

## Core behavior and negative requirements
- **Explicit authorization:** every implementation plan transitions to execution only with stored project-scoped approval; approval records include owner, project, immutable plan revision/hash, allowed folders, action class, time and expiry.
- **Project confinement:** canonicalized allowed root, symlinks and path revalidation; edits outside approved tree denied. Host policy must not assume an LLM will obey prompt restrictions.
- **Critical operations:** sudo, broad deletion, package install, remote git push and similar operations cannot auto-execute through AI suggestions. Separate action-specific confirmation required; never default to 'allow all'.
- **Timeout/fallback:** AI/provider requests must have explicit timeouts, cancellation propagation, bounded retries with backoff and fallback *only* to owner-enabled candidates.
- **Progress:** distinguish manually entered estimate, detected activity and verified test/commit evidence; never invent percentages.
- **Resume:** a process killed halfway must not replay a non-idempotent file edit automatically; require reconciliation, a step-specific checkpoint or fresh approval.
- **State transparency:** each run has durable ID, phase state, step state, timestamps, bounded logs, errors and artifacts.
- **Offline fallback:** local identity/settings/tasks/notes/reminders/reports available without provider connections.
- **Notifications:** desktop notification attempted only when available and enabled; failed notification recorded and UI remains source of truth.
- **Model identity:** provider connection, displayed model identifier, billing mode (API vs subscription CLI) and actual supported capabilities are separate fields.

## Nonfunctional requirements
| NFR | Requirement | Validation |
|---|---|---|
| NFR-01 Memory | Combined Maya-owned service RSS normal-use target `<500 MB`; report external processes separately | `11_PERFORMANCE_BUDGET.md` scenarios |
| NFR-02 CPU | Idle event-driven, no tight polling | CPU sampling after idle baseline |
| NFR-03 Safety | Actions restricted by explicit owner policy and scope | Security adversarial suite |
| NFR-04 Data | Local SQLite and secrets via supported secure store | Restart/recovery and secret inspection |
| NFR-05 Service | Listen loopback only; defend against localhost CSRF/foreign origins | Integration security test |
| NFR-06 Portability | Ubuntu x86_64 initially; no requirement for Docker/Electron | Clean-install on target Ubuntu |
| NFR-07 Reliability | DB migrations recoverable, jobs durable, timestamps timezone-safe | Crash/failure injection |
| NFR-08 Accessibility | Keyboard navigable, visible approval state, text alternative to voice | Manual keyboard/contrast check |
| NFR-09 Maintainability | Independent adapter traits; deterministic local core; test fakes | Offline integration tests |

## Requirements verification rules
- Every source feature must have a row in `16_TRACEABILITY.md` and at least one test obligation before closure.
- A feature is **not done** because its UI exists; it needs behavior, negative test, documentation and proof.
- No benchmark, service integration or CLI permission can be claimed as verified without log/command/date/environment evidence.
