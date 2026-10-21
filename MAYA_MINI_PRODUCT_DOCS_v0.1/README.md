# Maya Mini v0.1 — Product & Engineering Documentation

**Status:** Development baseline / documentation candidate for owner review  
**Source-of-truth for approved feature scope:** [docs/00_APPROVED_FEATURE_SCOPE.md](docs/00_APPROVED_FEATURE_SCOPE.md)  
**Target:** Ubuntu, single-user, local-first lightweight assistant and supervised AI coding orchestrator  
**Stack decision:** Rust + Tokio + Axum + rusqlite/SQLite + HTML/CSS/vanilla JS; external AI/API/CLI as needed  
**Version:** 0.1 | **Document date:** 2026-10-20

> This is an **implementation-ready documentation pack**, not finished software, verified benchmarks, or proof of legal/commercial integration availability. The feature scope is transcribed verbatim from the previously finalized spec. Detailed designs below are *proposals* to be reviewed before implementation. If a derived document conflicts with the source, the source wins until the owner signs off on a change request.

## Where to start

1. **Owner/product reviewer:** `docs/01_PRD.md` → `docs/03_WORKSPACE_UX.md` → `docs/08_FLOWCHARTS.md` → `docs/15_DECISIONS_RISKS.md`.
2. **Developer/coding agent:** `AGENTS.md` (Codex), `CLAUDE.md` (Claude Code), or `GEMINI.md` (Gemini CLI) → `docs/00_APPROVED_FEATURE_SCOPE.md` → `docs/04_ARCHITECTURE.md` → `docs/13_ROADMAP_BACKLOG.md`.
3. **QA/reviewer:** `docs/02_SRS_ACCEPTANCE.md` → `docs/10_TEST_STRATEGY.md` → `docs/16_TRACEABILITY.md` → `docs/19_RELEASE_CHECKLIST.md`.

## Document index

| File | Purpose |
|---|---|
| `docs/00_APPROVED_FEATURE_SCOPE.md` | Exact, unchanged prior v0.1 feature specification (55 feature IDs) |
| `docs/01_PRD.md` | Product vision, audience, constraints, user journeys, success metrics |
| `docs/02_SRS_ACCEPTANCE.md` | Functional/nonfunctional requirements + verifiable acceptance tests |
| `docs/03_WORKSPACE_UX.md` | Four-tab minimal UI and six configuration groups, interaction specifications |
| `docs/04_ARCHITECTURE.md` | Components, data boundaries, Rust crate/module plan, error conventions |
| `docs/05_INTEGRATION_CONTRACTS.md` | AI/CLI/browser/voice/OS adapter contracts and fallback behavior |
| `docs/06_DATA_MODEL.md` | SQLite entity relationships, persistence, migration, retention |
| `docs/07_HTTP_API.md` | Proposed local REST/SSE API contract (draft, versioned) |
| `docs/08_FLOWCHARTS.md` | Mermaid architecture, flowcharts, sequence and state diagrams |
| `docs/09_SECURITY_PRIVACY.md` | Threat model, security design, approvals, secrets, isolation |
| `docs/10_TEST_STRATEGY.md` | Unit/integration/E2E/security/performance/UAT strategy |
| `docs/11_PERFORMANCE_BUDGET.md` | Memory/CPU budgets, measurement method and optimization gates |
| `docs/12_SDLC_WORKFLOW.md` | Requirements → design → implementation → release with approval gates |
| `docs/13_ROADMAP_BACKLOG.md` | Small, dependency-ordered work packages with exit criteria |
| `docs/14_OPERATIONS_RUNBOOK.md` | Dev environments, deploy/update, backup/recovery/diagnostics |
| `docs/15_DECISIONS_RISKS.md` | ADRs, risk register, explicitly open decisions |
| `docs/16_TRACEABILITY.md` | All approved feature IDs mapped to components, validation and milestone |
| `docs/17_USER_STORIES.md` | Product-first scenarios and end-to-end acceptance walkthroughs |
| `docs/18_AGENT_HANDOFF.md` | Codex/Gemini/Claude prompts, handoff checklist, task records |
| `docs/19_RELEASE_CHECKLIST.md` | Final go/no-go evidence checklist and owner acceptance |
| `PLANS.md` | Agent planning protocol for sizeable work |
| `templates/` | Non-executable example request/config/handoff formats |

## Project invariants

- The requested **<500 MB combined resident Maya-owned service RSS** is a **measured target**, never an unqualified guarantee. Report Chromium/VS Code/coding CLI/voice processes separately.
- **No local LLM**, no React/Electron runtime, no multi-user server, no agent self-modification, no wake word in v0.1.
- Project modifications require explicit owner approval of **plan and scoped paths**. Destructive or privileged actions always require separate confirmation.
- Prefer supported APIs/official coding CLIs. Web AI bridges are **optional, capability-gated**, not a prerequisite for project-to-code automation.
- Localhost-only; project and secret boundaries are enforced by host code, not solely prompts.
- Background activity is event-based; the product must remain useful offline for local tasks, notes, reminders, reports.

## SDLC status and next action

**Complete:** proposed documentation and initial traceability. **Not complete:** owner sign-off, proof-of-concept benchmarks, implementation, verification. Review open decisions (`docs/15_DECISIONS_RISKS.md`) and approve milestone M0 before telling any agent to scaffold code. `docs/12_SDLC_WORKFLOW.md` specifies the gate.

**Implementation command policy:** No development code, dependency installation, service installation, database migrations or destructive actions are authorized solely by this documentation pack.
