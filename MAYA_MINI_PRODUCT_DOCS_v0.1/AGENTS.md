# Maya Mini — Shared coding-agent policy (Codex)

Treat `docs/00_APPROVED_FEATURE_SCOPE.md` as the feature-scope authority, not as permission to implement everything at once.

## Required reading order
1. `README.md`
2. `docs/00_APPROVED_FEATURE_SCOPE.md`
3. `docs/12_SDLC_WORKFLOW.md`
4. For the assigned milestone: `docs/13_ROADMAP_BACKLOG.md`, then relevant architecture/API/data/security/test documents
5. `PLANS.md` for changes spanning multiple modules or involving security, storage, agent execution, or interfaces.

## Non-negotiable rules
- **Do only the assigned work package**, never start the next milestone autonomously.
- The source spec is authoritative for feature scope; any derived contract is a draft pending product owner approval.
- Do not implement dynamic privileged shell execution, arbitrary filesystem edits, silent deletion, unapproved project modifications, unattended agent chains or AI web scraping that violates provider terms.
- **Approval gate lives in Rust host code**, independent of instructions sent to AI. Respect scoped paths and explicit expiry of approval.
- Keep secrets in supported OS secret storage when possible; never in SQLite plaintext, logs, prompts, test fixtures or Git.
- For code changes: include automated tests, negative tests, resource-sensitive design, and docs update. Preserve offline task functionality.
- Prefer bounded Tokio tasks and one Rust server process. Never add Electron, React, a local inference server, Docker requirement or always-on polling without explicit approved decision.
- Show concrete evidence (test output, diff, measured memory) instead of claiming success.
- Do not guess CLI flags/API models/pricing. Verify against installed-version help or official documentation before writing integration code.

## Task workflow
1. Identify approved milestone/work-package ID; if none, perform **read-only inspection** and propose the first task, then stop.
2. Record assumptions, risks, touched files, exact acceptance cases, commands, test plan and stop conditions.
3. If the work is substantial, create/update a small ExecPlan following `PLANS.md`; obtain owner approval before implementation.
4. Implement smallest testable increment; run format, lint, focused tests, broad regression as appropriate.
5. Deliver diff summary, test evidence, RSS/CPU measurement when relevant, unresolved issues and ready-to-review status.
6. Update requirement-to-test traceability when a feature actually lands.

## Build commands
Only after Rust workspace exists, prefer `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Do not mark these as run when workspace does not yet exist.
