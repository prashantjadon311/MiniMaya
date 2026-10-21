# 07 — Local HTTP API (Proposed Contract)

Base: `http://127.0.0.1:8765/api/v1` **provisional port**; chosen port is configurable at startup, only loopback binding allowed. The API is local-use; it is **not** a remote/public service.

## Transport and access
- JSON request and response; bounded body size; SSE for run updates (no service worker required).
- Per-session unpredictable token + same-origin/Host validation and CSRF protection for mutations; `Access-Control-Allow-Origin: *` forbidden.
- Sensitive operations need **separate time-limited approval object** besides a valid UI session; browser-origin credential alone does not equal authorization to edit files.
- Errors are machine-readable, never exposing secrets or raw provider credentials.

## Shared envelopes
```json
{
  "data": { "id": "task-id", "status": "todo" },
  "meta": { "request_id": "req-01", "version": "v1" }
}
```
```json
{
  "error": { "code": "APPROVAL_REQUIRED", "message": "Owner approval required", "retryable": false },
  "meta": { "request_id": "req-02", "version": "v1" }
}
```

## Endpoints
| Method | Path | Effect |
|---|---|---|
| GET | `/health` | readiness and local capabilities; no credentials |
| GET/PATCH | `/settings` | nonsecret identity/voice/assistant/security settings (validated) |
| GET/POST | `/providers` | list/create enabled provider metadata; credentials use secret write path |
| PATCH/DELETE | `/providers/{id}` | edit/delete with role-reference safety checks |
| POST | `/providers/{id}/secret` | store secret via backend keyring; return masked status only |
| GET/PUT | `/model-roles` | assign chat/planning/coding/fallback role & policy |
| GET/POST | `/projects` | list/register canonical allowed project root |
| PATCH/DELETE | `/projects/{id}` | update scoped path or archive; invalidate old approvals |
| GET/POST | `/tasks` | list/filter, create |
| GET/PATCH | `/tasks/{id}` | details and status/progress/subtasks updates |
| POST | `/tasks/{id}/checkins` | record owner progress/blocker |
| GET/POST | `/reminders` | list/create reminders |
| PATCH/DELETE | `/reminders/{id}` | schedule change/disable |
| GET | `/reports/daily?date=YYYY-MM-DD` | accurate day report |
| GET | `/reports/weekly?week=YYYY-Www` | accurate week report |
| GET/POST | `/conversations` | list/create chat sessions |
| GET/POST | `/conversations/{id}/messages` | cursor pagination / new text or task request |
| GET/POST | `/jobs` | list/start read-only/plan-only approved workflow request |
| GET | `/jobs/{id}` | durable state + steps + artifacts |
| GET | `/jobs/{id}/events` | SSE bounded stream + reconnection cursor |
| POST | `/jobs/{id}/approval-requests` | request time-limited approval for plan revision/scope |
| POST | `/approvals/{id}/decision` | owner allow/deny with explicit acknowledgment |
| POST | `/jobs/{id}/actions` | pause, cancel, safe resume, bounded retry |
| GET | `/system/resources` | Maya-owned RSS/CPU and external tool process summaries |
| POST | `/memory/export` | owner-initiated redacted export |
| POST | `/memory/delete-request` | confirmation-gated delete of selected local history |

## Example task create
```json
{
  "title": "Review Maya Chromium adapter",
  "description": "Review and propose a scoped plan",
  "project_id": "prj-01",
  "priority": "high",
  "due_at": "2026-10-21T18:00:00+05:30",
  "estimated_minutes": 120
}
```

## Example workflow request
```json
{
  "project_id": "prj-01",
  "task_id": "task-01",
  "goal": "Review architecture and prepare an implementation plan",
  "mode": "plan_only",
  "role": "planning",
  "max_agent_steps": 3
}
```
**An API request with `mode=plan_only` must never write project files.** Executing code requires a separate approved plan revision and scope verified by Rust before process launch.

## Example approval decision
```json
{
  "decision": "approve",
  "plan_sha256": "sha256-of-immutable-plan-artifact",
  "allowed_paths": ["src", "tests"],
  "action_classes": ["project_file_edit", "nonprivileged_test"],
  "expires_in_minutes": 30,
  "owner_ack": "I approve the shown plan and paths"
}
```
No claimed security from front-end text: the server verifies current plan hash, project root, path containment, approval expiry, authorization session and action class.

## Client behavior
- Workspace edits are PATCHed through host validation; never send/return raw stored API key.
- SSE reconnect with event cursor; if unavailable, fetch `/jobs/{id}` snapshot (bounded), not constant high-frequency polling.
- Local origin protections are tested against a malicious webpage attempting to call local API.
- For broad compatibility, list endpoint pagination (`limit`, `cursor`), and never return unbounded logs or chat history.

**Contract status:** endpoints and JSON shape are engineering proposals, pending M0 approval and implementation contract tests.
