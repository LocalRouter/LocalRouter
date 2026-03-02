<!-- @entry coding-agents-overview -->

LocalRouter can orchestrate AI coding agents (Claude Code, Gemini CLI, Codex, and more) as MCP tools through the Unified MCP Gateway. Any MCP client can spawn, interact with, and manage coding agent sessions — turning LocalRouter into a multi-agent orchestration hub.

Each installed coding agent gets its own set of 6 MCP tools that manage the full session lifecycle: starting sessions, sending messages, checking status, responding to questions, interrupting, and listing sessions.

Coding agents are **implicitly enabled** when their binary is found on the system PATH. No manual configuration is required — install a supported agent and it's automatically available.

Sessions are strictly tied to the creating client. No cross-client session visibility or sharing.

<!-- @entry coding-agents-supported -->

LocalRouter supports the following coding agents:

| Agent | Binary | Tool Prefix |
|-------|--------|-------------|
| Claude Code | `claude` | `claude_code` |
| Gemini CLI | `gemini` | `gemini_cli` |
| Codex | `codex` | `codex` |
| Amp | `amp` | `amp` |
| Aider | `aider` | `aider` |
| Opencode | `opencode` | `opencode` |
| Cursor | `cursor` | `cursor` |
| Qwen Code | `qwen-code` | `qwen_code` |
| GitHub Copilot | `gh` | `copilot` |
| Droid | `droid` | `droid` |

An agent appears in the Coding Agents view as "installed" when its binary is found on the system PATH.

<!-- @entry coding-agents-mcp-tools -->

Each enabled agent gets 6 MCP tools exposed through the gateway. The tools follow a consistent naming pattern: `{agent_prefix}_{action}`.

For example, Claude Code gets: `claude_code_start`, `claude_code_say`, `claude_code_status`, `claude_code_respond`, `claude_code_interrupt`, `claude_code_list`.

<!-- @entry coding-agents-tool-start -->

**`{agent}_start`** — Start a new coding session with an initial prompt.

Parameters:
- `prompt` (required) — The initial task or prompt
- `workingDirectory` — Working directory for the session
- `model` — Model override (optional, agent default applies)
- `permissionMode` — `auto`, `supervised`, or `plan` (default: agent's configured mode)

Returns a `sessionId` and `status: "active"`. The session runs asynchronously — the tool returns immediately.

This is the only tool that triggers a gateway permission check (Allow/Ask/Off).

<!-- @entry coding-agents-tool-say -->

**`{agent}_say`** — Send a message to an existing session. Automatically resumes if the session has ended.

Parameters:
- `sessionId` (required) — The session ID
- `message` (required) — The message to send
- `permissionMode` — Switch permission mode (interrupts + resumes if active)

Behavior:
- **Session active, no mode change:** sends the message to the agent
- **Session active, mode change:** interrupts and resumes with the new mode
- **Session ended:** auto-resumes the session with the message

<!-- @entry coding-agents-tool-status -->

**`{agent}_status`** — Check session status and retrieve recent output.

Parameters:
- `sessionId` (required) — The session ID
- `outputLines` — Recent output lines to return (default: 50)

Returns status (`active`, `awaiting_input`, `done`, `error`, `interrupted`), recent output lines, and a `pendingQuestion` object if the agent is waiting for input.

The `pendingQuestion` field is key for the approval flow — it surfaces tool approvals, plan approvals, and clarification questions that the agent needs answered before proceeding.

<!-- @entry coding-agents-tool-respond -->

**`{agent}_respond`** — Respond to a pending question (tool approval, plan approval, or clarification question).

Parameters:
- `sessionId` (required) — The session ID
- `id` (required) — Question ID from `pendingQuestion.id`
- `answers` — One answer per question (e.g., `"allow"`, `"deny: too risky"`)

Answers can include a reason after a colon. For example:
- Tool approval: `"allow"` or `"deny: too dangerous"`
- Plan approval: `"approve"` or `"reject: also cover auth"`
- Questions: Custom option values

<!-- @entry coding-agents-tool-interrupt -->

**`{agent}_interrupt`** — Interrupt a running session.

Parameters:
- `sessionId` (required) — The session ID

Sends a cancellation signal to the agent process. The session can be resumed later via `{agent}_say`.

<!-- @entry coding-agents-tool-list -->

**`{agent}_list`** — List all sessions for this client and agent.

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

For clients without elicitation support, approvals are queued as `pendingQuestion` objects. The client discovers them by polling `{agent}_status` and responds via `{agent}_respond`.

Flow: Agent requests approval → Gateway queues as `pendingQuestion` → Client polls `{agent}_status` → Client sees `pendingQuestion` → Client calls `{agent}_respond` → Agent proceeds.

The `pendingQuestion` object includes:
- `id` — Unique question identifier
- `type` — `tool_approval`, `plan_approval`, or `question`
- `questions` — Array of `{ question, options }` items

<!-- @entry coding-agents-session-lifecycle -->

Sessions follow a state machine:

- **Active** — Agent is processing. Transitions to `awaiting_input` when approval needed, `done` on completion, `error` on failure, or `interrupted` on cancel.
- **Awaiting Input** — Agent is blocked waiting for a response. Transitions back to `active` when `{agent}_respond` is called.
- **Done** — Agent finished successfully. Can be resumed via `{agent}_say` (auto-resume).
- **Error** — Agent encountered an error. Can be resumed via `{agent}_say` (auto-resume).
- **Interrupted** — Agent was interrupted. Can be resumed via `{agent}_say` (auto-resume).

Any terminal state (`done`, `error`, `interrupted`) transitions back to `active` when `{agent}_say` is called — the session auto-resumes automatically.

<!-- @entry coding-agents-permissions -->

Coding agent access is controlled per-client through a hierarchical permission system:

- **Global permission** — Default for all agents (`allow`, `ask`, `off`)
- **Per-agent override** — Override for specific agents

Permission resolution: per-agent override → global default.

- **Allow** — Client can start sessions freely, no gateway prompt
- **Ask** — Gateway prompts the user on session creation (`{agent}_start`), then free interaction
- **Off** — Client has no access to this agent

Permissions are configured in the client's Coding Agents tab.
