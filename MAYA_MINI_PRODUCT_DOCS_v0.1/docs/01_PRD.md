# 01 — Product Requirements Document (PRD)

**Product:** Maya Mini v0.1 | **Status:** derived design, owner review required | **Owner:** project owner

## Problem statement
The owner needs a **small Ubuntu personal assistant** capable of managing daily obligations and supervising project-development requests end-to-end, from discussing a scoped project with an AI service through a reviewed coding plan, implementation via an official coding agent, test execution and a truthful report. Existing AI chat tools alone do not persist task/check-in state or coordinate local project actions safely.

## Intended user and environment
- One owner on Ubuntu laptop, interacting primarily through a lightweight local browser tab.
- Hindi, Hinglish and English text; push-to-talk/voice as an optional extension.
- The laptop may run VS Code, Chromium and coding agents independently. The Maya-owned service must remain small.

## Product outcomes
1. **Delegate:** 'Maya, review this registered project, prepare a plan and implement it after my approval.'
2. **Remember:** Track goals, tasks, progress, blockers and check-in outcomes across restarts.
3. **Review:** See precise step state, approvals, diff, tests, reports, and recover safely after cancellation/failure.
4. **Configure:** Pick assistant identity, provider roles, voices, project folders, notification times and permissions from one minimal workspace.
5. **Trust:** No silent privileged/system-wide actions or unapproved project changes.

## User journeys
- **J1: Coding task:** register project → request review → inspect plan → approve allowed paths → AI agent edits within scope → verify diff/tests → summarize and update task.
- **J2: Personal task:** natural-language create → reminder fires → user reports progress → Maya records evidence/estimate → daily summary.
- **J3: Workspace setup:** choose name/language/model roles → securely store credential through supported OS facility → choose project root and permissions → save/restart → unchanged config restored.
- **J4: Voice:** press mic → STT or graceful text fallback → respond using preferred language → optionally speak response.
- **J5: Limited web action:** user asks to open one AI site in Chromium → MAYA opens site; automated submission/reading only when integration permits it; manual handoff otherwise.

## MVP constraints (binding from source)
- <500 MB combined Maya-owned resident-process RSS is a measurement target under defined conditions, **not a guarantee**; external agents/browser/IDE/voice workers measured separately.
- Tasks, notes, reminders and reports remain functional without connectivity.
- No local LLM, cloud-hosted Maya backend, wake word, built-in IDE, banking/payment, phone/smart home, persistent unrestricted autonomous execution, agent swarms, multi-user.
- Localhost-only Rust API + static Vanilla JS, no Electron/React requirement.

## Metrics (proposed, to confirm)
| Metric | Definition / acceptance |
|---|---|
| Core memory | Maya-owned service **combined RSS <500 MB** in normal idle and ordinary local task workflows, measured per performance plan |
| Idle CPU | Near zero after quiescence; quantify through idle 10-minute baseline; no busy loop |
| Reliability | Reminder persistence after restart, durable job state, no unapproved duplicate changes |
| Safety | Zero unapproved privileged/destructive operations in negative test suite |
| Transparency | Each AI coding run offers captured plan, approval record, changed files, tests and final status |
| UX | Four top-level navigation tabs, six workspace settings groups, no mandatory setup wizard with many steps |

## Scope and release posture
The source spec (`00_APPROVED_FEATURE_SCOPE.md`) enumerates **55** features in sections A–H. All remain accepted v0.1 scope; milestones implement them incrementally. A provider/model may be absent or disallowed: show honest 'unavailable' state instead of silently substituting.

## Product decisions pending owner confirmation
Performance measurement baseline, exact CLI integration mode, minimum supported Ubuntu versions, browser automation degree, initial STT/TTS provider, reminder delivery in locked/suspended sessions; see `15_DECISIONS_RISKS.md`.

## Changes to scope
Any new feature, removed feature, changed permission policy or increased resident processes requires a change request in `docs/12_SDLC_WORKFLOW.md` plus update to traceability; changing a document without approval does not modify the baseline.
