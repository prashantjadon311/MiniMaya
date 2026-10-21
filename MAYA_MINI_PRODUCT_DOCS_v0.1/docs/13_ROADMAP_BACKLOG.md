# 13 — Milestones, Work Packages & Dependencies

**Status:** proposed implementation schedule, not authorization. Avoid date/time promises until owner reviews laptop/dependencies and approves M0. Vertical slices are prioritized; no parallel agent edits against the same repository state.

## M0 — Architecture confirmation & local tool proof
| WP | Deliverable | Source requirement | Exit criterion |
|---|---|---|---|
| M0-WP01 | Approve doc baseline, open ADR decisions, choose local security approach | All | owner accepts architecture and priorities |
| M0-WP02 | Read-only feasibility check for Rust/SQLite/loopback API under sample load | A05, H07 | recorded cold/idle RSS+CPU numbers on target laptop |
| M0-WP03 | Inspect installed candidate Codex CLI version, authorization and safe subprocess mode | B02, E05 | interface and sandbox limitations documented; no project edit |

## M1 — Foundation & minimal usable shell
| WP | Deliverable | IDs | Acceptance |
|---|---|---|---|
| M1-WP01 | Rust single-process service, static 4-tab UI, loopback protections | A02, A05, H04 | UI accessible only locally, foreign-origin mutation denied |
| M1-WP02 | SQLite migration, settings, safe config persistence, OS keyring | A01, H01, H03 | restart persistence and secret safety verified |
| M1-WP03 | Task CRUD, subtasks, notes and work journal | D01, D02, D07 | tasks survive restart, no phantom progress |
| M1-WP04 | Basic local resource view and audit events | H06, H07 | service/external RAM separately displayed |
| M1-WP05 | Identity + Models + Projects sections of Workspace (initial controls) | A01, B03, B04, E01 | fields persist and validate; four-tab rule preserved |

## M2 — Personal assistant + model orchestration
| WP | Deliverable | IDs | Acceptance |
|---|---|---|---|
| M2-WP01 | Task intent parser, chat context, task linking | A03, A04, D01 | utterance maps to editable task draft |
| M2-WP02 | Event-based reminders, schedules, overdue and planner | D03, D04, D09 | correct due time, no busy polling |
| M2-WP03 | Progress check-ins, evidence source, local notifications | D05, D06 | cannot present manual % as verified |
| M2-WP04 | Daily/weekly reports, searchable memory and exports | D08, H01, H02, H08 | accurate reports, redacted export |
| M2-WP05 | API providers/model roles, overrides, explicit fallback, usage | B01, B03–B07 | offline failures safe, no disabled fallback |
| M2-WP06 | Optional sequential multi-AI chain with step limits | B08 | bounded steps, provenance maintained |

## M3 — Supervised project-to-code workflow
| WP | Deliverable | IDs | Acceptance |
|---|---|---|---|
| M3-WP01 | Project registration, approved file scanning, Git status | E01, E02, F03 | symlink/outside-root denial |
| M3-WP02 | Read-only planning/review, immutable plan artifacts | E03 | owner sees scope and test plan before edits |
| M3-WP03 | Action/approval engine, safe command runner | E04, H04, F04 | bypass and expired approval tests blocked |
| M3-WP04 | Coding CLI interface, streaming, cancellation, safe child process | B02, E05, E08 | verified binary used; limited subprocess and logs |
| M3-WP05 | Checkpoints, diff/test verification, failed-run recovery | E06, E07, H05 | partial failure never silently repeated |
| M3-WP06 | Complete run detail, final report and task links | E09, D09, H06 | files/tests/results truthful and accessible |
| M3-WP07 | PC app launcher, system status, clipboard | F01, F02, F05 | explicit user request; least privilege |

## M4 — Voice & browser optional integrations / release hardening
| WP | Deliverable | IDs | Acceptance |
|---|---|---|---|
| M4-WP01 | Push-to-talk, STT, TTS, device/voice settings | G01–G05 | microphone denied → text still works |
| M4-WP02 | Chromium CDP lifecycle + AI site shortcuts | C01, C02, F06 | separate permitted session, clean shutdown |
| M4-WP03 | Conditional website handoff and page reading | C03–C05 | graceful manual export fallback, no DOM scrape dependency |
| M4-WP04 | Full permission/privacy/retention/recovery + UAT regression | H02–H08 | negative suite, RSS tests, owner release checklist |

## Dependency chain
`M0 → M1-WP01/02 → M1-WP03/04 → M2 → M3-WP01/02/03 → M3-WP04/05/06 → M4`.
M3 coding work can begin once M1 service+security+DB and M2 model-role basics are stable; independent M2 reminder/report work may proceed separately **only** on nonconflicting branches with owner coordination.

## Minimum demonstrable vertical slice
**First working demo:** start local service → open Workspace → save name → create task → restart → task survives and resource indicator is visible. Do not build a whole AI orchestration framework before this works.
**First supervised coding demo:** register a tiny sample Git repo → read-only plan → approval → CLI edit → run test → show actual diff and result → safe cancellation test.

## Scope caution
The source lists 55 features, but milestone IDs might be referenced more than once because a cross-cutting feature spans implementation stages. `16_TRACEABILITY.md` provides the authoritative feature-to-primary-milestone assignment.
