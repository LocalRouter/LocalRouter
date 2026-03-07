<!-- @entry coding-agents-overview -->

LocalRouter can orchestrate AI coding agents (Claude Code, Gemini CLI, Codex, and more) as MCP tools through the Unified MCP Gateway. Any MCP client can spawn, interact with, and manage coding agent sessions — turning LocalRouter into a multi-agent orchestration hub.

Each client can be assigned one coding agent type. The selected agent is exposed through a unified set of MCP tools (`coding_agent_start`, `coding_agent_say`, etc.) that manage the full session lifecycle: starting sessions, sending messages, checking status, responding to questions, interrupting, and listing sessions.

Coding agents are **implicitly enabled** when their binary is found on the system PATH. No manual configuration is required — install a supported agent and it's automatically available.

Sessions are strictly tied to the creating client. No cross-client session visibility or sharing.

<!-- @entry coding-agents-supported -->

LocalRouter supports the following coding agents:

| Agent | Binary | Description |
|-------|--------|-------------|
| Claude Code | `claude` | Anthropic's agentic coding tool |
| Gemini CLI | `gemini` | Google's AI coding assistant |
| Codex | `codex` | OpenAI's autonomous coding agent |
| Amp | `amp` | Sourcegraph's AI coding agent |
| Aider | `aider` | AI pair programming in terminal |
| Opencode | `opencode` | Open-source terminal AI assistant |
| Cursor | `cursor` | Cursor's CLI agent |
| Qwen Code | `qwen-code` | Alibaba's Qwen-based agent |
| GitHub Copilot | `gh` | GitHub Copilot's CLI extension |
| Droid | `droid` | Autonomous coding agent |

An agent appears in the Coding Agents view as "installed" when its binary is found on the system PATH.

<!-- @entry coding-agents-client-assignment -->

Each client is assigned **one coding agent type** (or none). This is configured in the client's Coding Agents tab. When a client has a coding agent assigned, the unified `coding_agent_*` tools become available in that client's MCP session.

The assigned agent determines which binary is spawned when `coding_agent_start` is called, and what capabilities are available (model selection, permission modes, etc.).

<!-- @entry coding-agents-mcp-tools -->

The selected coding agent is exposed through unified MCP tools with the `coding_agent_` prefix. The same tool names are used regardless of which agent is assigned — the gateway routes to the correct agent automatically.

<!-- @entry coding-agents-tool-start -->

**`coding_agent_start`** — Start a new coding session with an initial prompt.

Parameters:
- `prompt` (required) — The initial task or prompt
- `workingDirectory` — Working directory for the session
- `model` — Model override (optional, agent default applies)
- `permissionMode` — `auto`, `supervised`, or `plan` (default: agent's configured mode)

Returns a `sessionId` and `status: "active"`. The session runs asynchronously — the tool returns immediately.

This is the only tool that triggers a gateway permission check (Allow/Ask/Off).

<!-- @entry coding-agents-tool-say -->

**`coding_agent_say`** — Send a message to an existing session. Automatically resumes if the session has ended.

Parameters:
- `sessionId` (required) — The session ID
- `message` (required) — The message to send
- `permissionMode` — Switch permission mode (interrupts + resumes if active)

Behavior:
- **Session active, no mode change:** sends the message to the agent
- **Session active, mode change:** interrupts and resumes with the new mode
- **Session ended:** auto-resumes the session with the message

<!-- @entry coding-agents-tool-status -->

**`coding_agent_status`** — Check session status and retrieve recent output.

Parameters:
- `sessionId` (required) — The session ID
- `outputLines` — Recent output lines to return (default: 50)

Returns status (`active`, `awaiting_input`, `done`, `error`, `interrupted`), recent output lines, and a `pendingQuestion` object if the agent is waiting for input.

The `pendingQuestion` field is key for the approval flow — it surfaces tool approvals, plan approvals, and clarification questions that the agent needs answered before proceeding.

<!-- @entry coding-agents-tool-respond -->

**`coding_agent_respond`** — Respond to a pending question (tool approval, plan approval, or clarification question).

Parameters:
- `sessionId` (required) — The session ID
- `id` (required) — Question ID from `pendingQuestion.id`
- `answers` — One answer per question (e.g., `"allow"`, `"deny: too risky"`)

Answers can include a reason after a colon. For example:
- Tool approval: `"allow"` or `"deny: too dangerous"`
- Plan approval: `"approve"` or `"reject: also cover auth"`
- Questions: Custom option values

<!-- @entry coding-agents-tool-interrupt -->

**`coding_agent_interrupt`** — Interrupt a running session.

Parameters:
- `sessionId` (required) — The session ID

Sends a cancellation signal to the agent process. The session can be resumed later via `coding_agent_say`.

<!-- @entry coding-agents-tool-list -->

**`coding_agent_list`** — List all sessions for this client.

Parameters:
- `limit` — Max sessions to return (default: 50)

Returns a list of sessions with their ID, working directory, display text, timestamp, and status.

<!-- @entry coding-agents-approvals -->

When a coding agent needs approval (tool use, plan review) or wants to ask a clarification question, LocalRouter routes it through the gateway. There are two routing paths depending on client capabilities.

<!-- @entry coding-agents-elicitation -->

If the MCP client supports **elicitation**, approval requests are forwarded directly to the client. The client receives the question inline and can respond immediately without polling.

Flow: Agent requests approval → Gateway forwards to client → Client responds → Agent proceeds.

This is the preferred path as it provides real-time interaction.

<!-- @entry coding-agents-polling -->

For clients without elicitation support, approvals are queued as `pendingQuestion` objects. The client discovers them by polling `coding_agent_status` and responds via `coding_agent_respond`.

Flow: Agent requests approval → Gateway queues as `pendingQuestion` → Client polls `coding_agent_status` → Client sees `pendingQuestion` → Client calls `coding_agent_respond` → Agent proceeds.

The `pendingQuestion` object includes:
- `id` — Unique question identifier
- `type` — `tool_approval`, `plan_approval`, or `question`
- `questions` — Array of `{ question, options }` items

<!-- @entry coding-agents-session-lifecycle -->

Sessions follow a state machine:

- **Active** — Agent is processing. Transitions to `awaiting_input` when approval needed, `done` on completion, `error` on failure, or `interrupted` on cancel.
- **Awaiting Input** — Agent is blocked waiting for a response. Transitions back to `active` when `coding_agent_respond` is called.
- **Done** — Agent finished successfully. Can be resumed via `coding_agent_say` (auto-resume).
- **Error** — Agent encountered an error. Can be resumed via `coding_agent_say` (auto-resume).
- **Interrupted** — Agent was interrupted. Can be resumed via `coding_agent_say` (auto-resume).

Any terminal state (`done`, `error`, `interrupted`) transitions back to `active` when `coding_agent_say` is called — the session auto-resumes automatically.

<!-- @entry coding-agents-permissions -->

Coding agent access is controlled per-client:

- **Permission state** — `allow`, `ask`, or `off` (configured in the client's Coding Agents tab)
- **Agent type** — Which coding agent this client uses (one per client)

Permission behavior:
- **Allow** — Client can start sessions freely, no gateway prompt
- **Ask** — Gateway prompts the user on session creation (`coding_agent_start`), then free interaction
- **Off** — Client has no access to coding agents
