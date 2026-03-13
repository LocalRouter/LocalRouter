<!-- @entry coding-agents-overview -->

LocalRouter can orchestrate AI coding agents (Claude Code, Gemini CLI, Codex, and more) as MCP tools through the Unified MCP Gateway. Any MCP client can spawn, interact with, and manage coding agent sessions — turning LocalRouter into a multi-agent orchestration hub.

Each client can be assigned one coding agent type. The selected agent is exposed through a unified set of MCP tools (default: `AgentStart`, `AgentSay`, `AgentStatus`, `AgentList`) that manage the full session lifecycle. The tool prefix is configurable.

Agent process management is powered by [BloopAI/vibe-kanban's executors crate](https://github.com/BloopAI/vibe-kanban), providing robust process lifecycle management including kill_on_drop, graduated signal escalation, and for Claude Code: the full SDK control protocol with session resumption via `--resume`.

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

Each client is assigned **one coding agent type** (or none). This is configured in the client's Coding Agents tab. When a client has a coding agent assigned, the unified agent tools become available in that client's MCP session.

The assigned agent determines which binary is spawned when the start tool is called, and what capabilities are available (model selection, permission modes, etc.).

<!-- @entry coding-agents-mcp-tools -->

The selected coding agent is exposed through 4 unified MCP tools. The tool prefix is configurable (default: `Agent`). When the prefix ends with an alphanumeric character, suffixes are PascalCase (`AgentStart`). When it ends with a non-alphanumeric character like `_`, suffixes stay lowercase (`agent_start`).

<!-- @entry coding-agents-tool-start -->

**Start** (default: `AgentStart`) — Start a new coding session with an initial prompt.

Parameters:
- `prompt` (required) — The initial task or prompt
- `workingDirectory` — Working directory for the session
- `model` — Model override (optional, agent default applies)
- `permissionMode` — `auto`, `supervised`, or `plan` (default: agent's configured mode)

Returns a `sessionId` and `status: "active"`. The session runs asynchronously — the tool returns immediately.

This is the only tool that triggers a gateway permission check (Allow/Ask/Off).

<!-- @entry coding-agents-tool-say -->

**Say** (default: `AgentSay`) — Send a message to an existing session. Can also interrupt and/or resume.

Parameters:
- `sessionId` (required) — The session ID
- `message` — The message to send. If session is done/error, resumes with context preserved via `--resume`.
- `interrupt` — If true, interrupts current work before sending the message. If true with no message, just stops the agent.
- `permissionMode` — Switch permission mode

Behavior:
- **Message only, session active:** sends the message to the agent (queued)
- **Message only, session ended:** auto-resumes the session with context preserved
- **Interrupt only:** gracefully interrupts the agent
- **Interrupt + message:** interrupts, then resumes with the new message
- **Neither message nor interrupt:** returns an error

<!-- @entry coding-agents-tool-status -->

**Status** (default: `AgentStatus`) — Check session status and retrieve recent output.

Parameters:
- `sessionId` (required) — The session ID
- `outputLines` — Recent output lines to return (default: 50)
- `wait` — If true, blocks until the session needs attention (done, awaiting_input, error, interrupted) instead of returning immediately
- `timeoutSeconds` — Max seconds to wait when wait=true (default: 300, max: 600)

Returns status (`active`, `awaiting_input`, `done`, `error`, `interrupted`), recent output lines, cost estimate, and turn count.

Use `wait: true` to block until the agent needs attention, instead of polling in a loop.

<!-- @entry coding-agents-tool-list -->

**List** (default: `AgentList`) — List all sessions for this client.

Parameters:
- `limit` — Max sessions to return (default: 50)

Returns a list of sessions with their ID, working directory, display text, timestamp, and status.

<!-- @entry coding-agents-approvals -->

When a coding agent needs approval (tool use, plan review) or wants to ask a clarification question, LocalRouter routes it through one of three configurable approval modes:

<!-- @entry coding-agents-approval-modes -->

**Approval modes** (configured globally in Coding Agents settings):

- **Elicitation** (default) — Forward approval requests to the MCP client via MCP's elicitation protocol. The client receives the question inline and can respond immediately. Falls back to Ask if the client doesn't support elicitation.

- **Ask** — Show an approval popup in LocalRouter's UI. The popup includes dynamic fields based on the request: allow/deny for tool approvals, or form fields for questions.

- **Allow** — Auto-approve all tool usage and answer all questions automatically. The agent runs fully autonomously. A warning is shown in the configuration UI when this mode is selected.

<!-- @entry coding-agents-session-lifecycle -->

Sessions follow a state machine:

- **Active** — Agent is processing. Transitions to `done` on completion, `error` on failure, or `interrupted` on cancel.
- **Done** — Agent finished successfully. Can be resumed via the say tool (context preserved with `--resume`).
- **Error** — Agent encountered an error. Can be resumed via the say tool.
- **Interrupted** — Agent was interrupted. Can be resumed via the say tool.

Any terminal state (`done`, `error`, `interrupted`) transitions back to `active` when a message is sent — the session auto-resumes with context preserved (for agents that support it, like Claude Code).

<!-- @entry coding-agents-permissions -->

Coding agent access is controlled per-client:

- **Permission state** — `allow`, `ask`, or `off` (configured in the client's Coding Agents tab)
- **Agent type** — Which coding agent this client uses (one per client)

Permission behavior:
- **Allow** — Client can start sessions freely, no gateway prompt
- **Ask** — Gateway prompts the user on session creation (start tool), then free interaction
- **Off** — Client has no access to coding agents
