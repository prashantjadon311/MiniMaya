# 08 — Flowcharts, Sequences & State Machines (Mermaid)

The diagrams are **editable text** and render in Mermaid-enabled Markdown viewers (for example GitHub). They visualize *proposed* design, not working software. All workflow writes route through host-side security gate.

## 1. Architecture overview
```mermaid
flowchart TD
  U[Owner] --> UI[Existing Browser / Local Workspace]
  UI -->|127.0.0.1| HTTP[Axum HTTP and SSE]
  HTTP --> R[Rust Orchestrator]
  R --> T[Task + Reminder Service]
  R --> M[Models and CLI Adapters]
  R --> W[Scoped Workspace Tools]
  R --> X[Optional Browser / Voice]
  R --> P[Policy and Approval Guard]
  P -. authorizes .-> W
  P -. authorizes .-> M
  T --> DB[(SQLite)]
  R --> DB
  P --> DB
  M --> EXT[External AI or Coding Agent]
```

## 2. User request to coding delivery
```mermaid
flowchart TD
  A([Owner requests project work]) --> B{Registered project?}
  B -- No --> B1[Request scoped path registration] --> B
  B -- Yes --> C[Read-only project inspection]
  C --> D[AI review and plan artifact]
  D --> E[Show plan, paths, risks and tests]
  E --> F{Explicit owner approval?}
  F -- No --> G[Save plan, wait or cancel]
  F -- Yes --> H[Verify plan hash, paths, expiry, action policy]
  H --> I{Valid authorization?}
  I -- No --> E
  I -- Yes --> J[Save checkpoint and launch official coding CLI]
  J --> K[Capture bounded events and edits]
  K --> L[Inspect actual diff and run approved tests]
  L --> M{Tests pass?}
  M -- No --> N[Report failure and request next action]
  M -- Yes --> O[Save artifact and final report]
  O --> P([Update linked task])
```

## 3. Coding workflow interaction / sequence
```mermaid
sequenceDiagram
  participant U as Owner
  participant W as Workspace UI
  participant M as Maya Rust Core
  participant P as Policy Guard
  participant C as Coding CLI
  participant FS as Approved Project
  U->>W: Discuss project and implement
  W->>M: Create workflow request
  M->>FS: Scoped read-only inspection
  M->>W: Plan + provenance + proposed paths
  U->>W: Approve plan revision and paths
  W->>M: Signed-in local approval decision
  M->>P: Validate hash, folder, actions, expiry
  alt Approved
    P-->>M: Allowed scope
    M->>C: Run with restricted cwd/context
    C->>FS: Attempt file modifications
    FS-->>M: Actual diff/commit state
    M->>FS: Run allowlisted verification
    M-->>W: Status, diff, tests, report
  else Rejected or expired
    P-->>M: Deny
    M-->>W: Blocked, no edits
  end
```
**Implementation warning:** Simply setting a CLI cwd does not guarantee filesystem confinement. Host authorization + OS sandbox/mechanism must be considered separately, see `09_SECURITY_PRIVACY.md`.

## 4. Personal task lifecycle
```mermaid
flowchart TD
  A[Request in Chat or Tasks] --> B[Parse title, due time, priority]
  B --> C{Time ambiguous?}
  C -- Yes --> D[Ask owner to clarify timezone/date] --> B
  C -- No --> E[Persist task and reminder]
  E --> F[Schedule next wake-up event]
  F --> G{Deadline or check-in due?}
  G -- No --> F
  G -- Yes --> H[Send permitted local notification]
  H --> I[Ask progress and blocker]
  I --> J{Owner answered?}
  J -- No --> K[Mark check-in unanswered]
  J -- Yes --> L[Store manual progress + evidence tag]
  K --> M[Generate truthful daily/weekly report]
  L --> M
  M --> N{Task done?}
  N -- No --> F
  N -- Yes --> O([Archive/review])
```

## 5. Persisted job state diagram
```mermaid
stateDiagram-v2
  [*] --> Created
  Created --> Inspecting
  Inspecting --> Planned
  Planned --> AwaitingApproval
  AwaitingApproval --> Approved: owner approves immutable plan
  AwaitingApproval --> Cancelled: deny/cancel
  Approved --> Executing: policy recheck
  Executing --> Verifying: tool exits / changed files recorded
  Verifying --> Reporting
  Reporting --> Completed
  Inspecting --> Failed
  Executing --> Failed
  Verifying --> Failed
  Executing --> Cancelling: cancellation requested
  Cancelling --> Cancelled: child stopped and reconciled
  Approved --> Paused: before next step
  Paused --> Approved: resume approved step
  Executing --> NeedsReconciliation: crash or uncertain side effect
  NeedsReconciliation --> AwaitingApproval: replan/reapprove
  NeedsReconciliation --> Verifying: safe state confirmed
  Failed --> AwaitingApproval: revised plan / new authorization
  Completed --> [*]
  Cancelled --> [*]
```
Note: `NeedsReconciliation` is a proposed internal recovery status derived from source requirement H05, not an additional end-user capability.

## 6. Provider/model routing and website fallback
```mermaid
flowchart LR
  Q[Request] --> R[Resolve role and model preference]
  R --> A{Provider enabled and available?}
  A -- Yes --> B{Supported API or official CLI?}
  B -- Yes --> C[Execute supported integration]
  B -- No --> D{Permitted browser interaction?}
  D -- Yes --> E[Optional browser handoff]
  D -- No --> F[Manual copy or export requested]
  A -- No --> G{Explicit enabled fallback?}
  G -- Yes --> R
  G -- No --> H[Show actionable error]
  C --> I[Store result with provenance]
  E --> I
  F --> I
```

## 7. Security decision flow
```mermaid
flowchart TD
  I[Requested action] --> C[Classify read / scoped-write / critical]
  C --> P[Canonicalize and validate path/command]
  P --> D{Denied or out of scope?}
  D -- Yes --> X([Reject and audit])
  D -- No --> A{Owner approval required?}
  A -- No --> E[Execute safe read]
  A -- Yes --> U[Show exact action, scope and expiry]
  U --> V{Owner explicitly approves?}
  V -- No --> X
  V -- Yes --> W[Revalidate plan hash and scope at execution]
  W --> Z{Still valid?}
  Z -- No --> X
  Z -- Yes --> K[Execute bounded action and audit]
```

## 8. SDLC flow
```mermaid
flowchart LR
  Req[Approved Scope] --> Spec[Design + Acceptance]
  Spec --> Review{Owner sign-off}
  Review -- No --> Spec
  Review -- Yes --> Plan[Milestone Work Plan]
  Plan --> Implement[Small implementation increment]
  Implement --> Tests[Unit/Integration/Security]
  Tests --> Pass{All gates pass?}
  Pass -- No --> Implement
  Pass -- Yes --> Performance[RSS/CPU + End-to-end UAT]
  Performance --> Release{Owner accepts?}
  Release -- No --> Plan
  Release -- Yes --> Ship[Package / Record Release]
```
