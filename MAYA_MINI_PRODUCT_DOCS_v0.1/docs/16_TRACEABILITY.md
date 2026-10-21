# 16 — Requirements Traceability Matrix

This table is **generated from the exact frozen source feature table** in `00_APPROVED_FEATURE_SCOPE.md`: **55 distinct IDs**, no missing/extra feature IDs. Each row names **planned** ownership and a **test obligation**, not a claim that tests are implemented or passed. Work packages in `13_ROADMAP_BACKLOG.md` refine milestones.

| Source ID | Approved feature | Planned component | Primary phase | Req | Planned test obligations |
|---|---|---|---|---|---|
| A01 | Custom assistant identity | `identity_chat / server` | M1 | FR-01 | T-01,T-24 |
| A02 | Text conversation | `identity_chat / server` | M1 | FR-01 | T-01,T-24 |
| A03 | Intent classification | `identity_chat / server` | M2 | FR-01 | T-01,T-24 |
| A04 | Context handling | `identity_chat / server` | M2 | FR-01 | T-01,T-24 |
| A05 | Local service | `identity_chat / server` | M1 | FR-01 | T-01,T-24 |
| B01 | Provider connectors | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B02 | Official coding CLI bridge | `providers / coding_cli` | M3 | FR-02 | T-04,T-05,T-15 |
| B03 | Model registry | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B04 | Role-based assignment | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B05 | Manual override | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B06 | Failure handling | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B07 | Basic cost/usage journal | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| B08 | Multi-AI chain | `providers / coding_cli` | M2 | FR-02 | T-04,T-05,T-15 |
| C01 | Browser controller | `browser_bridge` | M4 | FR-03 | T-15 |
| C02 | AI website shortcuts | `browser_bridge` | M4 | FR-03 | T-15 |
| C03 | Browser-assisted chat | `browser_bridge` | M4 | FR-03 | T-15 |
| C04 | External AI response handoff | `browser_bridge` | M4 | FR-03 | T-15 |
| C05 | Web page reading | `browser_bridge` | M4 | FR-03 | T-15 |
| D01 | Natural-language task creation | `tasks / scheduler` | M1 | FR-04 | T-02,T-03,T-20,T-21 |
| D02 | Tasks/subtasks | `tasks / scheduler` | M1 | FR-04 | T-02,T-03,T-20,T-21 |
| D03 | Daily planner | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| D04 | Reminders | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| D05 | Proactive follow-up | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| D06 | Progress updates | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| D07 | Work journal | `tasks / scheduler` | M1 | FR-04 | T-02,T-03,T-20,T-21 |
| D08 | Reports | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| D09 | Reschedule & resume | `tasks / scheduler` | M2 | FR-04 | T-02,T-03,T-20,T-21 |
| E01 | Project registration | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E02 | Project inspection | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E03 | Discuss & plan | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E04 | Approval gate | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E05 | Coding execution | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E06 | Verify changes | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E07 | Checkpoint & rollback | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E08 | Run monitoring | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| E09 | Final handoff | `projects / jobs / policy` | M3 | FR-05 | T-06,T-07,T-09,T-10,T-11 |
| F01 | Desktop launcher | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| F02 | System insights | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| F03 | File finder/reader | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| F04 | Command runner | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| F05 | Clipboard | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| F06 | Minimal browser actions | `tools / projects` | M3 | FR-06 | T-08,T-12 |
| G01 | Push-to-talk | `voice` | M4 | FR-07 | T-16 |
| G02 | Speech to text | `voice` | M4 | FR-07 | T-16 |
| G03 | Spoken responses | `voice` | M4 | FR-07 | T-16 |
| G04 | Language preferences | `voice` | M4 | FR-07 | T-16 |
| G05 | Voice settings | `voice` | M4 | FR-07 | T-16 |
| H01 | Local memory | `store / policy / telemetry` | M1 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H02 | Memory controls | `store / policy / telemetry` | M2 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H03 | Credential safety | `store / policy / telemetry` | M1 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H04 | Permissions | `store / policy / telemetry` | M1/M3 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H05 | Job controls | `store / policy / telemetry` | M3 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H06 | Audit trail | `store / policy / telemetry` | M1/M3 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H07 | Resource monitor | `store / policy / telemetry` | M1 | FR-08 | T-08,T-13,T-14,T-17,T-19 |
| H08 | Local data export | `store / policy / telemetry` | M2 | FR-08 | T-08,T-13,T-14,T-17,T-19 |

## Traceability policy
- This inventory is checked against the source feature table before release.
- As implementation progresses, add a linked test file name and actual PASS/FAIL/SKIP evidence for each row; these high-level planned IDs are not a substitute for test cases.
- Category/component mapping may change through an ADR, but source feature IDs cannot disappear without owner-approved change request.
- Cross-cutting safety/resource rules are validated in all relevant modules even if their feature ID has only one primary ownership row.
