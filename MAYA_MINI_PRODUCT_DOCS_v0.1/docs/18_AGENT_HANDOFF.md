# 18 — Codex / Gemini CLI / Claude Code Handoff Handbook

## Rule 0: one owner of the working tree
Only one coding agent may change the active working tree at a time; review agents work read-only or on isolated Git branches/worktrees. The user assigns milestone/ID; an AI must not assume unstated permission to implement.

## Portable one-time startup prompt (for Codex/Gemini/Claude)
```text
You are an implementation contributor for Maya Mini v0.1, not the product owner.
Before work, read README.md, AGENTS.md, docs/00_APPROVED_FEATURE_SCOPE.md,
docs/12_SDLC_WORKFLOW.md, docs/13_ROADMAP_BACKLOG.md, and relevant design docs.
Your assigned work package is: <WP-ID>. Your current operation is: READ-ONLY PLAN.
Do not write product code or install dependencies until I approve a concrete
work plan. List any open ADR and security/performance risks for this WP.
Produce: (1) exact source feature IDs; (2) files/modules affected;
(3) incremental plan; (4) security controls; (5) tests, including negative tests;
(6) RSS/CPU measurement impact; (7) unanswered questions. Then stop.
```

## Implementation prompt — only after approving an actual WP plan
```text
Approved work package: <WP-ID>.
Approved plan file and hash: <PATH/HASH>.
Approved files/modules: <LIST>. Forbidden files/paths: <LIST>.
Implement ONLY this slice. Preserve loopback security, project scope approvals,
secret protection and event-driven low-memory design. Add tests first or with
behavior. Run applicable verified commands; never claim commands were run if not.
Deliver actual changed file list, test results, known issues, resource measurements,
and updated traceability. Stop at the WP boundary; do not start another WP.
```

## Review-only prompt for a second agent
```text
READ-ONLY REVIEW of commit/diff <REF> for <WP-ID>.
Read approved feature IDs, acceptance requirements, security/privacy document and
related tests. Look for scope drift, unintended file access, weak approval checks,
secret leaks, subprocess/process-group risks, event-loop CPU usage, unbounded RAM,
untested failure states, mistaken source claims and regressions.
Report actionable findings by severity with exact file locations and reproduction.
Do not edit code or broaden scope.
```

## Cross-agent handoff template (use `templates/HANDOFF_TEMPLATE.md`)
- WP + approved feature IDs; relevant branch and commit hash.
- Current state: implemented / planned / blocked; exact next action.
- Files changed + architectural decisions + source scope check.
- Tests: commands, actual PASS/FAIL/SKIP and dates.
- Perf: method/metrics or 'not measured'.
- Security: permission/path/secret/CLI considerations + negative tests.
- Reconciliation: job/checkpoint status if any tool modified files.
- Questions needing owner sign-off.

## Reviewer responsibilities
- **Codex contributor:** implement approved slice after plan approval, maintain `AGENTS.md` convention, show actual diff/test results.
- **Gemini reviewer:** can challenge architecture/usage; follows `GEMINI.md`; do not ask it to change code in parallel.
- **Claude reviewer:** can perform independent threat modeling/UX validation; follows `CLAUDE.md`; read-only unless owner assigns separate approved WP.
These are **suggested role assignments**, not assumptions about availability, entitlements or performance of any tool.

## Minimum model context to provide
`00_APPROVED_FEATURE_SCOPE.md` + exact WP card + relevant architecture/API/data/security/test sections. Avoid providing every giant document on every small edit; use navigation and targeted references to keep context small and focused.

## Resource and service verification
Agent-specific CLI command flags, authentication, model choices and quotas must be validated against installed version and official docs during M0. For unreliable AI website bridges, implement manual handoff instead of attempting credential extraction or forbidden scraping.
