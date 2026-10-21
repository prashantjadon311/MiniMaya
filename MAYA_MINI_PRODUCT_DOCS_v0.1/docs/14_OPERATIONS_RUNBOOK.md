# 14 — Local Development, Deployment and Recovery Runbook

**Status:** conceptual runbook for product engineering; *do not run installation commands before chosen implementation plan and user permission*. Exact versions, paths and flags must be verified on target Ubuntu.

## Environment prerequisites
- Ubuntu desktop (minimum supported version to decide in ADRs).
- Stable Rust toolchain and build essentials appropriate for chosen SQLite linking/build strategy.
- Existing browser for `http://127.0.0.1:<port>`; browser not bundled by Maya.
- OS notification service and optional Secret Service/keyring for credential persistence.
- Optional: authenticated official coding CLI, Chromium executable with separate profile, STT/TTS providers, VS Code for viewing changes.
- Git recommended for project checkpoints and recovery.

## Development loop (once scaffolded)
1. Confirm `cargo --version`, `rustc --version`, SQLite libraries if chosen, target Ubuntu and available RAM.
2. Run Rust backend locally on loopback and open existing browser tab.
3. Validate migrations and settings with temporary test database before real user storage.
4. Make one scoped change, format/lint/test, measure owned RSS when process behavior changes.
5. Build release binary, test launching from an isolated config directory, verify no runtime Node/Python requirement.

## Packaging and startup (proposal)
- Ship a signed/versioned or checksum-verified Linux executable + static UI embedded/resources + migration assets; no Electron.
- Initial start manual or optional `systemd --user` service **only if owner opts in**. Use user privileges, no sudo/daemon service by default.
- Config/app state under XDG paths (`~/.config/maya-mini/`, `~/.local/share/maya-mini/`, `~/.cache/maya-mini/`) as confirmed in design.
- Service binds loopback and generates auth/session secret. No firewall holes, public endpoints or cloud telemetry by default.
- Notifications requiring login session may not fire if user is logged out/suspended; document real system behavior.

## Operational controls
- Status: readiness, enabled adapter capabilities, last reminder, job queue, RSS/CPU, child processes.
- Safe shutdown: stop accepting new write jobs, mark active jobs cancellation/recovery state, checkpoint SQLite WAL; preserve audit.
- Log rotation and cap; redact secrets and private file contents by default.
- Backups: SQLite backup API/safe checkpoint, versioned data copy, checksum, secrets exported separately only through OS-supported method if permitted.
- Restore drill: restore to isolated directory, migrate, load tasks and report data, check no auto-resumed external edits.

## Failure runbooks
| Symptom | First diagnostic | Safe response |
|---|---|---|
| UI opens but service unavailable | loopback binding, process status, local log | restart only Maya service; no system-wide kill |
| 500 MB goal exceeded | resident/core vs child RSS breakdown | stop/limit expensive job, capture sample, optimize bounded buffers |
| AI model/CLI auth fails | enabled role, installed binary, provider status | show action required; preserve offline tasks |
| Partially edited project | stored checkpoint, `git diff`, child exit event | mark reconciliation and request owner plan; do not auto-retry write |
| Reminder missed | timezone, notification permission, suspend/lock state | show missed state, offer reschedule |
| SQLite lock/migration failure | DB version, WAL state, backup status | avoid resetting data; diagnostic + safe rollback |
| Browser bridge fails | Chromium/version/profile/permissions | fall back to open-site/manual paste or supported API/CLI |
| Voice unavailable | device permission and provider status | fall back to text UI |

## Updates
Before upgrading: backup DB, record installed version, test migration on copy, allow cancel/rollback, verify permission defaults unchanged, rerun core smoke and security tests.

## Data ownership
All local data belongs to the owner. Explain export/delete limitations, secrets storage, and any external AI provider transfer before first use.
