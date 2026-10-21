# 03 — UX Specification: Minimal Four-Tab Workspace

## Information architecture (fixed)
**Exactly four top-level tabs:** `Chat`, `Tasks`, `Activity`, `Workspace`.

### 1. Chat
- Header: selected project, model/provider status, overall service status, stop action.
- Body: text conversation with time/status badges, action receipts, errors, links to run details; bounded scrollback/pagination.
- Composer: text entry, send, optional push-to-talk mic; no continuous audio capture.
- Action intent preview: for any write/action, show plan summary + requested scope + approval dialogue; free text alone must not authorize action.
- Empty state: one-line example: 'Create a task' or 'Discuss this project', no animated avatar.

### 2. Tasks
- Views: Today, Upcoming, Projects (subfilters, **not** extra top-level tabs).
- Each task: title, project, priority, due time with timezone, progress source (manual/observed/verified), status, subtasks, next check-in.
- Actions: add, update, postpone, mark done, ask for progress, open linked run, view daily/weekly reports.
- Check-ins are nonintrusive notifications based on configured schedule; suppress duplicate prompts and outside working hours as configured.

### 3. Activity
- Run list with most recent first; one active run detail at a time.
- Run detail: step timeline, current actor/provider, elapsed time, queued/current/completed/blocked markers, logs, plan revision, approved paths, diff summary, test results, cancel/pause/resume.
- Pause semantics: pause **between steps** by default; an active CLI subprocess may only support cancellation, not safe suspension. Display correct capability.
- Approval is a clearly differentiated dialog with project, editable scope, file list where known, allowed actions, provider/CLI, timeout and irreversible risks.
- Failure mode: error + recovery options; never display 'done' if tests not run or failed.

### 4. Workspace
**Exactly six settings groups**, as agreed in approved source:

| Group | Controls | Input constraints / behavior |
|---|---|---|
| Identity | Name, language, concise/standard response, greeting | nonempty trimmed names, persistent, no avatar |
| AI Models | provider list, model identifier, role assignment, endpoint, key, fallback, timeout | allow explicit enabled-state, secret mask, validation/test, no plaintext readback |
| Voice | STT/TTS adapter, voice, mic/speaker, speed, test | voice listed only if provider supports, explicit mic permission, text fallback |
| Projects & Tools | allowed project roots, CLI path, editor path, Chromium profile & toggle | canonical path validation, verify executable, no arbitrary root |
| Personal Assistant | working hours, check-in frequency, reminders, daily review and reports | timezone and recurrence visible, non-working hours respected |
| Security & System | approvals, retention/export/delete, backup and RAM/CPU | critical approval cannot be turned off; show external RAM separately |

## Design rules
- Static HTML/CSS/Vanilla JS; no heavy framework, bundled renderer, animation, charts or avatar.
- Responsive single-column at narrow widths; desktop nav rail or tabs.
- Accessible labels, keyboard focus order, validation errors next to controls, sufficient contrast, reduced motion.
- Sensitive inputs masked, never put API keys in HTML source, frontend localStorage or querystring.
- UX must reflect actual server state from API; browser page reload does not cancel background job.
- Explicit offline banners for AI-only features without blocking task work.

## UX acceptance
1. Save identity/models/voice/project settings, restart local service; values persist, secrets remain masked.
2. Keyboard-only user can create task, review plan, deny approval, inspect failed run.
3. No more than four first-level routes, six Workspace groups.
4. Cancelling run changes state promptly; if a subprocess cannot be instantly interrupted, present `cancelling` not `cancelled`.
5. Network disconnected: tasks/reminders/reports still navigable and editable.
