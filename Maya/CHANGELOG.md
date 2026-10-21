# Changelog — Maya Mini

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-10-20

### Added
- Complete implementation of all 55 approved v0.1 features across categories:
  - Personal Assistant Core (A01–A05)
  - Model & Agent Providers (B01–B08)
  - Browser Control & Web Automation (C01–C05)
  - Tasks & Personal Assistant Features (D01–D09)
  - Supervised Coding Workflow (E01–E09)
  - File, App & System Integration (F01–F06)
  - Voice Interaction (G01–G05)
  - Privacy, System & Maintenance (H01–H08)
- Single-process Rust backend with Axum HTTP/SSE server and SQLite persistence.
- Embedded 4-tab responsive web interface (Chat, Tasks, Activity, Workspace).
- Host-level security policy with path confinement (`openat2` beneath) and process isolation via Bubblewrap.
- Offline-first capabilities for tasks, schedules, journals, planner, and reports.
- Comprehensive test suite covering 120 verification scenarios.

### Known Limitations
- Automated coding through external CLI requires an installed supported Codex binary and valid environment; fallback API edit manifest is supported directly.
- CDP browser automation supports dedicated profiles; third-party website DOM form submission defaults to unsupported unless compiled site adapters are configured.
- Speech features depend on browser Web Speech API support or optional local `espeak-ng` / cloud provider endpoints.
