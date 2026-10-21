# Changelog — Maya Mini

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - Unreleased

### Added
- Single-process Rust backend with Axum HTTP/SSE server and SQLite persistence.
- Embedded 4-tab responsive web interface (Chat, Tasks, Activity, Workspace).
- SQLite-backed tasks, schedules, journals, planner, reports, conversations, memory, audit, usage, backup metadata, and workflow state.
- Provider registry and supported OpenAI, Gemini, and Anthropic wire adapters with bounded retries and usage records.
- Approval-gated coding workflow with project scanning, immutable plans, isolated stages, host-validated manifests, registered tests, promotion checks, and recovery states.
- Host-side origin/session/CSRF controls, scoped approvals, file containment, bounded subprocess supervision, secret redaction, and capability detection.
- Approval-bound export, delete, command, browser, desktop, backup restore, and cloud voice operation paths.
- Smoke, resource sampling, desktop entry, optional user service, and gated packaging harnesses.

### Known Limitations
- Release acceptance is still in progress. The 55 feature cases and 120-scenario evidence ledger are not yet all executed, so v0.1 must not be represented as fully released.
- Automated coding through external CLI requires an installed supported Codex binary and valid environment; fallback API edit manifest is supported directly.
- CDP and third-party website behavior depends on a separately configured Chromium debugging target and the fixed supported-site adapters; unavailable targets report capability failure.
- Speech features depend on browser Web Speech API support or optional local `espeak-ng` / cloud provider endpoints.
- Paid provider, real Codex, desktop notification, microphone, and browser success paths are not claimed until explicitly configured and tested; local fallbacks remain available where documented.
