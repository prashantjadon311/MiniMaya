# 17 — User Stories & Demo Walkthroughs

Each is derived from approved scope; these are acceptance scenarios, not extra product features.

## US-01 Configure assistant identity and models
**As owner**, I choose Maya's display name, Hindi/Hinglish/English language, response style, default chat/planning/coding roles, and optional voice without editing config files.  
**Given** initial startup, **when** I save settings and restart, **then** all nonsecret values persist; masked key is not retrievable, provider status shows whether the integration is usable.  
**Negative:** invalid endpoint/model ID or keyring unavailable produces actionable error without writing plaintext credentials.

## US-02 Add and track daily tasks
**As owner**, I say 'Today review project X at 6 PM and remind me', and want the task, due time and reminder recorded.  
**Then** planner lists it; on check-in Maya asks progress within working hours; when I say '40%', the report cites **manual progress**.  
**Negative:** ambiguous time prompts for clarification; no fabricated work completion.

## US-03 Delegate a safe coding change
**As owner**, I register a small Git repo, ask Maya to review architecture and implement an enhancement.  
**Given** project registration, **when** I request the work, **then** Maya scans allowed files, asks configured AI for a plan, shows paths/tests/risks, and **waits** for explicit approval before coding.  
**After approval** the official CLI works in permitted scope; Maya captures diff, test outcome, and final report linked to task.  
**Negative:** denial prevents edit; approval scope/hash mismatch halts execution.

## US-04 Resume safely after crash
**As owner**, I return after a coding subprocess crashed.  
**Then** Maya shows `NeedsReconciliation`, current diff and relevant checkpoint, not 'completed'. I may inspect, restore or reapprove a revised step after validation.  
**Negative:** no duplicate edits, silent rollback or unapproved retry.

## US-05 Use provider chain on request
**As owner**, I direct 'Use the planning model, have the reviewer critique its plan, and pass approved code work to coding agent'.  
**Then** only enabled roles/providers run, in order and with bounded steps; artifacts show who generated what.  
**Negative:** disabled/unauthorized provider and cost-unknown fallback are surfaced, not silently substituted.

## US-06 Use voice without always listening
**As owner**, I press mic, speak Hindi and optionally hear spoken reply.  
**Then** audio capture starts only while explicitly activated, preferred voice is used when supported.  
**Negative:** denied mic or missing TTS falls back to normal text UI.

## US-07 Open AI websites without breaking coding workflow
**As owner**, I ask to open ChatGPT/Gemini/Claude in Chromium to discuss a topic.  
**Then** Maya opens an authorized signed-in session; an automated handoff happens only if permitted and implemented, otherwise Maya asks me to paste/export allowed content.  
**Negative:** failure to extract website answer does not prevent an official CLI/API-driven project coding task.

## US-08 Control PC within policy
**As owner**, I ask Maya to open VS Code on my project and summarize Git status.  
**Then** it opens installed editor (if present), reads only permitted paths and shows result.  
**Negative:** request to delete directory, sudo install software or copy secrets is blocked or separately confirmed as specified in security policy.

## US-09 Review activity and resources
**As owner**, I inspect a running workflow and its impact on my laptop.  
**Then** Activity shows steps, logs, pause/cancel where safe, elapsed time, source evidence and separate Maya core RSS vs Chromium/CLI/IDE memory figures.  
**Negative:** a cancelled-but-alive child process is never falsely marked completely stopped.

## US-10 Export records and restore after restart
**As owner**, I export tasks, reports and safe settings to JSON/Markdown and verify local data after restart.  
**Then** export excludes raw secrets and preserves task history and decisions according to configured retention.  
**Negative:** sensitive tokens/cookies do not appear in output.

## Owner UAT script
1. Launch from fresh profile; choose language/name; create task and a reminder; restart and verify persistence.
2. Go offline; update task, record manual progress and generate daily report.
3. Create tiny sample Git repo; register it; ask review → inspect plan → **deny** and check no edit.
4. Repeat request; approve exact plan/paths; verify real code diff and test evidence.
5. Simulate failure/cancellation; confirm no duplicate write and recovery state is visible.
6. Test foreign-path command and browser-to-localhost cross-origin request: both denied.
7. Test voice and browser if configured; missing integration must not break basic app.
8. Record RSS/CPU measurements and external-process breakout; check release gates.
