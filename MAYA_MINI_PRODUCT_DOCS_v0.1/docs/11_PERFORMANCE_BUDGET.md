# 11 — Performance & Resource Budget

**Binding target:** Maya-owned resident Rust/core/service processes <500 MB **combined RSS** in normal use. The approved source explicitly excludes Chromium, VS Code, external coding CLI and separate STT/TTS workers from this cap; they must nonetheless be reported separately.

## Proposed internal engineering budget (not measured facts)
| Resident allocation envelope | Design target |
|---|---:|
| Rust backend + HTTP stack | ≤ 80 MB |
| SQLite caches / settings / task scheduling | ≤ 40 MB |
| Bounded chat/event buffers and workflow state | ≤ 50 MB |
| Telemetry/temporary headroom | ≤ 30 MB |
| **Proposed working envelope** | **≤ 200 MB** |
| **Hard acceptance target** | **< 500 MB combined resident Maya-owned processes** |

The working envelope is an **aspirational design allocation**, not a promise: release depends on measured outcomes. Idle browser tab may add significant memory outside Maya service accounting.

## Scenarios to benchmark
1. **Cold startup**: time-to-ready, initial RSS peak and settled RSS after 30s.
2. **Idle 10m**: no job, one task schedule waiting. Sample RSS & process CPU each 5s (measurement tool external).
3. **Normal local workload**: 1000 tasks, 100 reminders, 20 chat turns, reports, UI refresh; measure p50/p95 response time and peak RSS.
4. **AI API workflow**: provider response streamed with bounded buffering and cancellation; log memory ceiling.
5. **Coding CLI**: small dummy repo, long logs, test run; distinguish Maya service RSS vs child CLI vs VS Code/browser.
6. **Crash/recovery**: resume DB and history; no memory leak across 25 consecutive runs.
7. **Voice/Chromium**: only if adapters installed; separate per-process RSS, startup latency and cleanup confirmation.

## Measurement protocol on Ubuntu
- Record date/time, CPU type, RAM, kernel, Rust build profile, executable version/commit, task fixture size and memory methodology.
- Use `ps -o pid,ppid,rss,%cpu,comm -p <pid>`, `/proc/<pid>/status` (VmRSS), `/proc/<pid>/smaps_rollup` if permitted; sample process tree but **avoid simply adding shared RSS as actual unique memory**. Report sum-RSS as requested budget proxy and optionally PSS for shared-memory accounting.
- Track child processes separately by process groups/PID ancestry and known CLI/browser handles; avoid accidentally including an unrelated user's browser process.
- Collect settled, p95, peak and measurement conditions. CPU `%` varies per-core semantics; present raw metric and averaging interval.
- Record performance regressions in tracked test fixture; CI checks major slowdowns but does not substitute for target laptop manual benchmark.

## Efficiency techniques
- Rust build in `--release`; log/HTTP event stream bounded and paginated; no full read of huge file into RAM.
- Scheduling based on next due time + async timer (no 1s busy polling).
- Cap concurrent AI calls and subprocesses; child output piped incrementally to capped files and ring buffer.
- SQLite WAL/cache limits set by profiling, no separate database process.
- Voice/browser/CLI launched only when requested; close owned session/process on completion; never kill user's unrelated IDE/browser.
- Cache only small task summaries; load large history on demand. Prefer OS notifications over always-redrawing UI.

## Release pass/fail
- **Pass:** exact test fixtures show Maya-owned combined RSS under 500 MB under defined normal scenarios; idle CPU near zero compared with baseline, no persistent busy wait, and externally spawned memory clearly separately reported.
- **Fail:** service above budget, unbounded growth, unreported external child memory, leaked processes, or claiming a 500 MB cap for Chromium+VS Code.
- Any exception needs owner review and revised requirements; no silent relaxing of the target.
