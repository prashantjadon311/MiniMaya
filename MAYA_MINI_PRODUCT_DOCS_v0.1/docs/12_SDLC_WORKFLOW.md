# 12 — SDLC: Controlled AI-Assisted Development

## Development model
**Iterative, test-first where feasible, small vertical slices** with human approval at scope/design gates. Codex/Gemini/Claude are interchangeable engineering contributors, not independent product owners. Git history, docs and tests are source of implementation truth.

## Phases, artifacts and review gates
| Stage | Inputs | Required output | Exit gate |
|---|---|---|---|
| **S0 Discovery / scope** | approved feature source | PRD, 55-feature inventory, constraints, non-goals | owner confirms no scope drift |
| **S1 Architecture/design** | PRD, security risks | architecture, data model, API, UX, Mermaid flows, ADRs | owner approves M0 architectural baseline |
| **S2 Work planning** | approved design | milestone cards, owner-selected next WP, test/rollback strategy | explicit work-package approval |
| **S3 Implementation** | approved WP/plan | smallest compiling increment, tests, evidence, docs updates | peer/AI review + scope review |
| **S4 Verification** | implemented branch | unit/integration/security/UI tests, memory benchmark | all mandatory gates green |
| **S5 Acceptance** | verified build, release checklist | owner UAT, defects resolved, version sign-off | owner accepts release |
| **S6 Maintenance** | reported issue | small scoped correction, regression test, changelog | verified patch |

## Approval rules
- A user asking for documents authorizes **documents only**, not code/dependency installation.
- Creating an initial implementation plan is **not approval to execute it**. Owner first reviews design baseline; then reviews chosen milestone work plan; only then product code may be written.
- New work outside approved feature IDs is a formal change request (CR) with rationale, resource impact, privacy risk, schedule and approval.
- Any planned permission/secret boundary change is a separate design review regardless of work-package size.

## Branching and commit conventions
- `main` protected by human review in normal Git workflows; branch `feat/M1-WP03-task-crud`, `fix/...`, `docs/...`.
- Commit related tests and docs with implementation; small single-purpose changes.
- Before merging: format, lint, tests, security checks, traceability update, diff review; no AI pushes to remote without separate explicit authorization.
- Record software/license dependencies and maintenance burden when adding crates.

## Definition of Ready (DoR) for any WP
- WP ID, source feature IDs, user-visible acceptance, architecture/doc links.
- Known affected files/modules and data migrations identified, dependencies available.
- Security classification and approval requirements documented.
- Unit/integration/negative/performance tests identified.
- Owner approves implementation scope and optional work plan.

## Definition of Done (DoD)
1. Behavior meets source requirement and acceptance examples.
2. Diff is scoped and readable; no unused heavy dependencies; no secret leakage.
3. Required tests **executed** with evidence; no 'not run' mislabeled PASS.
4. Errors, cancellation and offline behavior documented/tested.
5. New/modified data schema has forward migration and recovery proof.
6. High-risk operations pass policy/security testing and approval UX.
7. Relevant docs/flowcharts/API/traceability updated.
8. Resource impact measured for new resident process, adapter or long-running task.
9. Owner has reviewed and accepted the work package.

## AI agent handoff process
- Before agent work: provide approved WP ID + files/doc index + explicit scope + test instructions + desired output format.
- Agent writes `workplan` (if needed); owner approves; agent works within scope.
- After agent work: share handoff template with diff, commands run/results, open issues, approval scope, risks, next exact step. A different agent begins by inspecting git diff and docs, **not** trusting the previous agent's narrative unverified.
- AI reviews can disagree; evidence, owner decisions and source spec determine resolution. Never ask multiple agents to edit the same uncommitted files concurrently.

## Documentation change control
| Type | Example | Process |
|---|---|---|
| Clarification | explain pause semantics | update derived doc, notify reviewer |
| Interface change | rename API field | ADR + dependent docs + tests + owner approval |
| Scope addition | cloud sync, wake word | CR, PRD/source update if accepted, new traceability |
| Constraint exception | higher RAM ceiling | explicit owner decision, revised benchmark target; never silently relax |

## Exit strategy per milestone
At each milestone, freeze a deployable subset and run lightweight smoke test; do not wait for all 55 features to show a working vertical slice. The final v0.1 release requires the full accepted source scope or an explicitly signed scope revision.
