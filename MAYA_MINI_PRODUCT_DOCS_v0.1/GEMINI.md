# Gemini CLI instructions — Maya Mini

Read `AGENTS.md` first. All shared rules there apply. Read `README.md` and `docs/00_APPROVED_FEATURE_SCOPE.md` before implementation.

- Only execute the currently approved milestone/work package. Never silently expand v0.1 scope.
- Use approved implementation plans, guarded project paths, host-side approval controls, and no hidden privileged commands.
- Keep Rust core single-process and event driven. Separate Maya-owned resource accounting from external Chromium/IDE/coding agents.
- Verify installed Gemini CLI options and authentication model when integrating; do not presume plan limits or web-page automations.
- Follow `docs/18_AGENT_HANDOFF.md`; output changed-file list, tests actually run, and blockers.
