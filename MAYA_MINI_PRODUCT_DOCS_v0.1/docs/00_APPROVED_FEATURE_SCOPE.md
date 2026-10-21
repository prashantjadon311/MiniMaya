# MAYA MINI v0.1 — Final Feature Scope & Configuration Workspace

**Platform:** Ubuntu Linux (single-user, local-first)  
**Backend:** Rust + Tokio + Axum  
**Database:** SQLite (rusqlite)  
**UI:** HTML + CSS + Vanilla JavaScript, served locally, opened in an existing browser  
**AI execution:** Supported AI APIs and coding CLI (e.g. Codex CLI). Optional browser AI adapter for user-visible, permitted actions.  
**Goal:** A lightweight personal assistant that handles tasks and runs supervised coding workflows using external AI.  
**Scope:** v0.1 only. Not the full Maya V1–V5 roadmap.

## 1. Non-negotiable requirements

- Maya-owned resident processes target **<500 MB combined RSS** in normal use (Rust core + local service). This is a design/test target, not a guarantee.
- Report memory separately for **Chromium, VS Code, external coding CLI/agent, and any separate STT/TTS service**. A 500 MB all-inclusive cap is not promised.
- Event-driven background loops; avoid continuous polling, background LLMs, and unnecessary workers.
- Single local user, local-first task history and configuration.
- Never silently modify files outside an approved project scope, run privileged commands, or delete data.
- Browser integrations must obey service rules; use supported APIs or official CLIs for dependable automation. Do not depend on scraping protected web responses.
- Every long-running job should be inspectable, pausable, cancellable, and recoverable.

## 2. Detailed v0.1 Features

### A. Assistant Core & Identity

| ID | Feature | Required behavior |
|---|---|---|
| A01 | Custom assistant identity | Configurable name, greeting, language (Hindi/Hinglish/English), and reply length/tone. |
| A02 | Text conversation | Chat with Maya, see action status and concise results. |
| A03 | Intent classification | Distinguish general question, task, reminder, project work, and system action. |
| A04 | Context handling | Remember current task/project and retain bounded recent chat context. |
| A05 | Local service | Localhost-only Rust service with no external hosted backend. |

### B. AI Models & Providers

| ID | Feature | Required behavior |
|---|---|---|
| B01 | Provider connectors | Configurable OpenAI-compatible, Gemini, Anthropic, and other compatible API endpoints as available; no provider hard-coded as mandatory. |
| B02 | Official coding CLI bridge | Launch configured coding CLI (initial target: Codex CLI), pass approved project context, capture structured results/status. |
| B03 | Model registry | Save provider, model identifier, endpoint, enabled status, and usage role. |
| B04 | Role-based assignment | Choose default chat, planning/review, coding, and fallback provider/model/CLI. |
| B05 | Manual override | Override selected model for a conversation or one workflow. |
| B06 | Failure handling | Timeouts, retry caps, cancellation, fallback only to explicitly enabled providers. |
| B07 | Basic cost/usage journal | Record API request usage/cost when provider reports it; distinguish subscriptions from API billing. |
| B08 | Multi-AI chain | User-defined sequence such as research → review → code; no uncontrolled model fan-out. |

### C. Browser & Web AI Bridge

| ID | Feature | Required behavior |
|---|---|---|
| C01 | Browser controller | Connect to a separate approved Chromium profile/session via CDP if configured; open pages and manage relevant tabs. |
| C02 | AI website shortcuts | Open ChatGPT, Gemini, and Claude sites in signed-in browser sessions. |
| C03 | Browser-assisted chat | User-visible prompt submission/response reuse only where supported and permitted; show errors or ask for manual paste/export when not supported. |
| C04 | External AI response handoff | Save permitted response text as a task artifact and pass it to the next selected agent/provider. |
| C05 | Web page reading | Open/search/read/summarize non-restricted pages on request. |

**Reliability rule:** The automated project-to-code pipeline must also work through an official coding CLI or API, without depending on the AI website's page structure or scraping.

### D. Personal Task & Planning Assistant

| ID | Feature | Required behavior |
|---|---|---|
| D01 | Natural-language task creation | Create task with name, description, project, priority, due date, estimate. |
| D02 | Tasks/subtasks | Status: Todo / In progress / Blocked / Done / Cancelled; subtasks and checklist. |
| D03 | Daily planner | Today, upcoming, overdue, priority ordering, recommended sequence. |
| D04 | Reminders | One-time and simple recurring reminders with local desktop notification. |
| D05 | Proactive follow-up | User-configurable check-in times; ask for progress and blockers, never claim progress without evidence. |
| D06 | Progress updates | Manually supplied percentage, task events, and evidence-based Git/test signals; distinguish estimates from verified outcomes. |
| D07 | Work journal | Persist check-ins, decisions, blockers, activity timestamps, and notes. |
| D08 | Reports | Daily/weekly completion, pending, delayed, blockers, key work log. |
| D09 | Reschedule & resume | Postpone, reprioritize, resume paused workflows, link work to projects. |

### E. Developer Workflow & Coding Agent

| ID | Feature | Required behavior |
|---|---|---|
| E01 | Project registration | Add project path, name, notes, preferred coding model/CLI, allowed folders. |
| E02 | Project inspection | Read files under approved scope; summarize structure, README, language, dependencies, and Git status. |
| E03 | Discuss & plan | Send goal/context to configured AI; generate a review, approach, and ordered implementation steps. |
| E04 | Approval gate | Ask for explicit approval before executing a new implementation plan; retain user-approved scope. |
| E05 | Coding execution | Ask coding CLI/API to edit project files within scope; use VS Code as optional viewer/editor. |
| E06 | Verify changes | Capture git diff, run configured non-privileged tests/builds, report successes/failures. |
| E07 | Checkpoint & rollback | Save Git checkpoint or scoped backup before edits; offer diff review and recovery. |
| E08 | Run monitoring | Step status, elapsed time, logs, failures, retries, cancellation, and resume point. |
| E09 | Final handoff | Save summary: changes, files touched, tests, issues, next actions; update linked tasks. |

**Canonical workflow:** User request → task → project scan → selected AI review → plan → approval → coding CLI/API → tests/diff → report → follow-up.

### F. Local PC, Files & Automation

| ID | Feature | Required behavior |
|---|---|---|
| F01 | Desktop launcher | Open supported installed apps, folders, web links, and VS Code project path. |
| F02 | System insights | On-demand CPU, RAM, storage, running process summary. |
| F03 | File finder/reader | Search approved folders; preview/read text/code/Markdown and supported PDFs. |
| F04 | Command runner | Run allowlisted commands or explicitly approved commands; sandbox/limits where practical. |
| F05 | Clipboard | Read/write on explicit request and with OS permission. |
| F06 | Minimal browser actions | Open link, search, retrieve allowed page content; no banking or purchases. |

### G. Voice Interaction

| ID | Feature | Required behavior |
|---|---|---|
| G01 | Push-to-talk | Hold/click button to capture speech; no always-on mic in v0.1. |
| G02 | Speech to text | Selectable external/cloud or lightweight supported STT provider, with text fallback. |
| G03 | Spoken responses | Selectable installed/system or provider-based TTS voice; optional mute. |
| G04 | Language preferences | Hindi, Hinglish and English input/output where supported by selected provider. |
| G05 | Voice settings | Input device, output device, chosen voice, rate, preview/test. |

### H. Memory, Security & Reliability

| ID | Feature | Required behavior |
|---|---|---|
| H01 | Local memory | SQLite storage for tasks, projects, relevant preferences, decisions, and compact conversation summaries. |
| H02 | Memory controls | Review, search and delete stored items; clear chat history separately. |
| H03 | Credential safety | OS keyring/secret storage where possible; never plaintext API keys in SQLite or logs. |
| H04 | Permissions | Action tiers: read, modify in approved project, and destructive/privileged. |
| H05 | Job controls | Pause, stop, retry, recover interrupted step; prevent duplicate side effects on retry. |
| H06 | Audit trail | Who/what launched a task, AI provider used, commands, file edits, tests, and approvals. |
| H07 | Resource monitor | Display Maya-owned RAM/CPU, external tool RAM separately, warn on limit. |
| H08 | Local data export | Export settings (without secrets), tasks and reports to JSON/Markdown. |

## 3. Minimal Workspace (Configuration UI)

**Top-level navigation: exactly 4 tabs**

1. **Chat** — one chat stream, project selector, mic, model indicator, running action, quick stop button.
2. **Tasks** — Today / Upcoming / Projects, progress, reminders, check-ins, run report link.
3. **Activity** — current job steps, progress, logs, approvals, diff/test summary, pause/resume/cancel.
4. **Workspace** — minimal configuration sections shown below.

### Workspace settings sections

| Section | Editable settings | Limits |
|---|---|---|
| Identity | Assistant name, preferred language, response style, default greeting | No avatar editor/theme marketplace |
| Models | Add/edit/remove API provider, masked API key, base URL (when supported), available model IDs, model role, fallback, timeout | No model downloads or fine-tuning |
| Voice | STT source, TTS source, selected voice, microphone, speaker, speaking speed, test buttons | No always-listening/wake word in v0.1 |
| Projects & Tools | Allowed project folders, coding CLI executable, VS Code executable, Chromium profile, browser bridge toggle | No unrestricted filesystem access |
| Personal Assistant | Working hours, reminder notification toggle, default check-in frequency, daily review time, report delivery preference | No autonomous calendar/email integrations |
| Security & System | Action approval rules, history retention, memory clear/export, resource monitor, backups | Destructive actions always require confirmation |

**Minimal workspace wireframe:**

```text
┌──────────────────────────────────────────────────────────────────┐
│ MAYA Mini                                        ● Ready   64 MB  │
├────────────┬─────────────────────────────────────────────────────┤
│ Chat       │ WORKSPACE                                           │
│ Tasks      │                                                     │
│ Activity   │ Assistant  Name [Maya]  Language [Hinglish]        │
│ Workspace  │                                                     │
│            │ AI Models  Chat [Gemini ▼]  Coding [Codex CLI ▼]    │
│            │            [+ Add provider]                        │
│            │                                                     │
│            │ Voice      Voice [Default ▼]  [▶ Test]             │
│            │                                                     │
│            │ Projects   Allowed path [/home/.../projects]       │
│            │                                                     │
│            │ Reminders  Check-in [Every 2h ▼] Notifications [on] │
│            │                                                     │
│            │ Security   Execute code [Ask] Delete [Always ask]  │
│            │                                                     │
│            │                                      [Save settings]│
└────────────┴─────────────────────────────────────────────────────┘
```

**UI design:** responsive, functional, keyboard friendly; no avatar, heavy animations, charting library, React runtime, Electron, or always-running frontend service. Use an existing browser tab to avoid another bundled renderer.

## 4. Explicitly Out of Scope for v0.1

- Local on-device LLM hosting and self-training.
- Always-listening microphone, wake word, and real-time full-duplex voice.
- Self-modifying Maya code, autonomous plugin marketplace, and agent swarms.
- Multi-user accounts, cloud sync, mobile apps, smart home.
- Automatic banking, payment, purchasing, password extraction, and bulk website scraping.
- Persistent unattended multi-hour workflows with unrestricted system access.
- Built-in IDE; VS Code and coding CLI are external integrations.

## 5. Definition of Done (v0.1)

1. User changes Maya name, model assignments, and voice in Workspace; settings persist after restart.
2. A natural-language request creates a task with reminder and progress follow-up; daily report includes it accurately.
3. User approves a project's file path and a coding plan. Maya runs coding CLI/API on that project, captures changed files + test results, and reports status.
4. Failed/cancelled steps are visible; user can resume safely without duplicate edits when feasible.
5. No unapproved destructive/sudo operation occurs; secrets never appear in logs or exported files.
6. Rust core + local service is profiled for the **<500 MB combined target**, with clear separate RAM reporting for browser, VS Code, coding agent and STT/TTS processes.
7. UI stays usable with external AI services offline for tasks, notes, reminders, and reports.

## 6. Build Order (to ship early)

1. **Foundation:** Rust service + SQLite + minimal Chat/Tasks/Workspace UI; permissions & resource telemetry.
2. **Assistant:** Task parsing, reminders, check-ins, memory, reports.
3. **Coding workflow:** Project paths, AI role setup, Codex CLI integration, approval, diff/tests, logs.
4. **Extensions:** Push-to-talk + TTS; API provider chaining; permitted Chromium bridge and web handoff.

**Scope-lock rule:** Any additional feature requires explicit change request. Do not silently promote full Maya roadmap features into Maya Mini v0.1.
