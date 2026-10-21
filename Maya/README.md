# Maya Mini v0.1

Maya Mini is a single-user, local-first personal assistant and supervised AI coding orchestrator designed for Ubuntu x86_64 desktops.

## Architecture & Principles
- **One Process:** A single Rust binary hosts the local Axum web server, SQLite database, task scheduler, security policy engine, and subprocess supervisor.
- **Embedded Web UI:** Served directly by the local loopback server using standard semantic HTML, CSS, and vanilla ES modules. No Node.js, Electron, or React runtime dependencies.
- **Offline Capable:** Identity, tasks, notes, reminders, daily/weekly planner, reports, and search function fully offline without any external AI model connection.
- **Strict Host-Side Security:** Path boundaries are enforced by Linux `dirfd` and `openat2` resolution checks (`RESOLVE_BENEATH`), not model prompts. Subprocesses run in private namespaces via bubblewrap (`bwrap`).
- **Owner Approval Gate:** All file modifications, deletions, test executions, application launches, and context uploads require explicit, cryptographic digest-bound owner approval.
- **Resource Footprint:** Designed to maintain a Maya-owned RSS target below 500 MB under normal workloads.

## Prerequisites
- **Target OS:** Ubuntu 24.04 LTS x86_64 (or compatible Linux desktop).
- **Toolchain:** Rust 1.99+ with Cargo.
- **Optional Tools (probed at runtime):**
  - `bwrap` (Bubblewrap) for sandboxed child process isolation.
  - `git` for repository metadata inspection.
  - `chromium` or `chromium-browser` with CDP remote debugging for browser assistance.
  - `pdftotext` (poppler-utils) for document inspection.
  - `wl-copy`/`wl-paste` or `xclip` for clipboard integration.
  - `espeak-ng` for offline text-to-speech.

## Building & Running

### Build
```bash
cargo build --release --locked
```

### Manual Interactive Launch
```bash
./target/release/maya-mini --open
```
This starts the local HTTP server on `127.0.0.1` (default port 8765 or OS-selected port with `--port 0`), generates a one-time bootstrap launch token in memory, and opens your default browser via `xdg-open`.

### Headless Launch
```bash
./target/release/maya-mini --headless --print-launch-link
```

## Security & Privacy Limits
- Loopback-only binding (`127.0.0.1`). No public network listener.
- Bearer session authentication and CSRF token verification required for all state-changing endpoints.
- Secret credentials (e.g. OpenAI/Anthropic/Gemini API keys) are stored in the OS Secret Service keyring or kept purely in memory for the active session. Key bytes are never written to SQLite or logs.
- Sensitive directories (`.git`, `.ssh`, `.gnupg`, `.env*`, private keys) are excluded from context scans by default.
