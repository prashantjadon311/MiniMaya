# 05 — Adapter and External Integration Contracts

The approved feature specification allows **multiple provider types**, but none is mandatory. Do not hard-code a model, available provider, subscription entitlement, CLI flag, web DOM or API price.

## Provider/CLI roles
| Capability | Contract | Essential constraints |
|---|---|---|
| Text model | `generate(request, cancellation) -> response/error` | provider+model ID, role, timeout, input size limit, provenance, usage if reported |
| Planning/review | `review(project_summary, request) -> plan_artifact` | read-only context, no direct modification from plan text |
| Coding CLI | `execute(approved_plan, scoped_project, cancellation) -> run_events + exit` | installed executable detection, permission control, explicit cwd, bounded stdout/stderr, deadline |
| Optional browser CDP | `open(url)`, `read_allowed_page`, `supported_chat_handoff` | authorized profile, permitted interactions; manual handoff when blocked |
| Voice STT | `transcribe(audio, language) -> text/error` | explicit activation, no persistent microphone stream |
| Voice TTS | `speak(text, voice, rate) -> event/error` | opt-in and cancellable; no text-only blocker |
| Linux tools | `run(approved_argv, cwd, limits)` / `open_path` | command allowlist or individually approved argv; no implicit shell |

## Example transport-neutral types
```text
ModelRequest {
  request_id, provider_id, model_id, role,
  message_blocks, project_context_refs[], timeout_ms,
  idempotency_key?, user_initiated: bool
}
ModelResult {
  request_id, provider_id, actual_model_id?, text,
  usage_input_tokens?, usage_output_tokens?, estimated_cost?,
  status, error_code?, artifact_refs[]
}
CodingRunRequest {
  job_id, approved_plan_sha256, project_root_canonical,
  allowed_paths[], allowed_tool_actions[], exec_timeout_ms,
  checkpoint_ref, approval_id
}
```

## Selection & fallback
1. A role selects its explicitly assigned provider or installed CLI adapter; a conversation may override the default.
2. If provider is down, show reason and offer only **owner-enabled** eligible fallback. Do not silently swap between subscription and paid API.
3. Sequential multi-AI chains are explicitly configured with order, max steps and per-step costs; no uncontrolled fan-out.
4. Responses are typed artifacts with provider, timestamp, model ID and source type; if user manually pasted a web response, label it `manual-handoff`.
5. Protect privacy: do not upload more of the project than user-approved context; exclude secrets, keys and ignored/private paths.

## Coding CLI integration
- **Initial target:** official Codex CLI, because source scope names it; architecture remains adapter-based for other official coding CLIs.
- Before invoking any CLI: check installed binary, version/help, interactive vs noninteractive support, authentication availability and whether approvals can be surfaced. Record detected capabilities in `Workspace > Projects & Tools`.
- Prefer supported noninteractive/structured interface for task automation; if unavailable, fail visibly or require interactive owner step.
- Use exact argument arrays and verified cwd; stdin/out protected and capped; child process tree cancellation needs verification on target Ubuntu.
- Treat CLI work as potentially side-effectful; launch with least-privilege folder access and configured sandbox, not just a prompt that says 'do not write outside'.

## AI website / Chromium integration
- Open ChatGPT/Gemini/Claude signed-in URLs as **shortcuts**; do not assume browser cookies, login state or site automation policies.
- Browser-assisted sending/reading is conditional on service rules and technical support; if unavailable use user's manual paste/export or supported API/CLI.
- Separate Chromium profile/session; CDP debug port, if used, bound to loopback, restricted, temporary, and disabled on completion.
- Never copy cookies, inspect passwords, bypass anti-bot/security measures or depend on a website DOM for the mission-critical coding flow.

## Contract testing
- Fake model errors: timeout, denied permission, 429/rate limit, malformed response, API offline, provider disabled.
- Fake CLI: no binary, exit nonzero, partial edits then interruption, huge stdout, hangs, authorization prompt, cancellation.
- Fake CDP: browser missing, tab closed, site login required, session not permitted.
- Voice: device missing, no permission, STT unavailable; chat works in all these cases.

**External source note:** Official references to verify at implementation time: [OpenAI Codex platform](https://developers.openai.com/blog/codex-as-a-platform), [OpenAI Codex development workflows](https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex), [Gemini CLI docs](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md), [Claude Code docs](https://code.claude.com/docs/en/memory). Provider documentation changes; installed-version tests control implementation.
