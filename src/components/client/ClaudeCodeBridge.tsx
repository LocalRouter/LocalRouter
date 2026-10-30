/**
 * Claude Code over the STDIO bridge.
 *
 * Why the bridge is the default for Claude Code: Claude Code receives pushed
 * events (Claude Code Channels) only from a stdio MCP server that uses the
 * handshake from before MCP revision 2026-07-28. LocalRouter over HTTP can
 * still serve tools, but cannot wake Claude when an event arrives. The STDIO
 * bridge runs `localrouter --mcp-bridge` as Claude's subprocess and relays
 * everything, including pushed events. HTTP stays available, labelled tools
 * only, and an existing HTTP setup is never rewritten without the explicit
 * "Switch to STDIO bridge" action. Mirrors
 * src-tauri/src/launcher/integrations/claude_code.rs.
 */
import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertTriangle, ArrowRightLeft, Loader2, Radio } from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { listenSafe } from "@/hooks/useTauriListener"
import { CopyableCode, CopyableCodeBlock } from "./CopyableCode"
import { CLAUDE_CHANNELS_LAUNCH_COMMAND, claudeCodeHttpConfig, claudeCodeStdioConfig } from "./claude-code-bridge-config"
import type {
  ClaudeCodeMcpSetup,
  GetClaudeCodeMcpSetupParams,
  LaunchResult,
  SwitchClaudeCodeToStdioBridgeParams,
} from "@/types/tauri-commands"

export { CLAUDE_CHANNELS_LAUNCH_COMMAND, claudeCodeHttpConfig, claudeCodeStdioConfig }

/** Why the bridge, and the caveats of the channels flag. */
export function ClaudeChannelsExplanation({ className = "" }: { className?: string }) {
  return (
    <div className={`text-xs text-muted-foreground space-y-1.5 ${className}`}>
      <p>
        Claude Code receives pushed events (Claude Code Channels) only from a stdio MCP server that uses the
        handshake from before MCP revision 2026-07-28. LocalRouter over HTTP can still serve tools, but cannot
        wake Claude when an event arrives. The STDIO bridge runs <code className="bg-muted px-1 rounded">localrouter --mcp-bridge</code> as
        Claude&rsquo;s subprocess and relays everything, including pushed events.
      </p>
      <ul className="list-disc pl-4 space-y-0.5">
        <li>The channels flag works in interactive sessions only, not with <code className="bg-muted px-1 rounded">claude -p</code>.</li>
        <li>An organisation&rsquo;s <code className="bg-muted px-1 rounded">channelsEnabled</code> policy can block it.</li>
        <li>After adding a push-capable MCP server to LocalRouter, restart <code className="bg-muted px-1 rounded">claude</code>.</li>
      </ul>
    </div>
  )
}

/** Manual setup for Claude Code: the bridge by default, HTTP as tools only. */
export function ClaudeCodeManualMcpSetup({
  binaryPath,
  clientId,
  secret,
  baseUrl,
}: {
  binaryPath: string
  clientId: string
  secret: string
  baseUrl: string
}) {
  const [showHttp, setShowHttp] = useState(false)
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        Add this to <code className="bg-muted px-1 rounded">~/.claude.json</code>:
      </p>
      <CopyableCodeBlock value={claudeCodeStdioConfig(binaryPath, clientId, secret)} />
      <div className="space-y-1">
        <p className="text-xs font-medium flex items-center gap-1.5">
          <Radio className="h-3.5 w-3.5" />
          Launch Claude Code with channels to receive pushed events
        </p>
        <CopyableCode value={CLAUDE_CHANNELS_LAUNCH_COMMAND} />
      </div>
      <ClaudeChannelsExplanation />
      <button
        type="button"
        className="text-xs text-primary hover:underline"
        onClick={() => setShowHttp((v) => !v)}
      >
        {showHttp ? "Hide" : "Show"} HTTP setup (tools only, no pushed events)
      </button>
      {showHttp && <CopyableCodeBlock value={claudeCodeHttpConfig(baseUrl, secret)} />}
    </div>
  )
}

/**
 * For an existing Claude Code client whose ~/.claude.json still uses HTTP:
 * explain what it misses and offer the explicit switch.
 */
export function ClaudeCodeHttpNotice({
  clientId,
  onSwitched,
}: {
  clientId: string
  onSwitched?: (result: LaunchResult) => void
}) {
  const [setup, setSetup] = useState<ClaudeCodeMcpSetup | null>(null)
  const [switching, setSwitching] = useState(false)

  const load = useCallback(async () => {
    try {
      setSetup(
        await invoke<ClaudeCodeMcpSetup>("get_claude_code_mcp_setup", {
          clientId,
        } satisfies GetClaudeCodeMcpSetupParams),
      )
    } catch (error) {
      console.error("Failed to read Claude Code MCP setup:", error)
    }
  }, [clientId])

  useEffect(() => {
    load()
    const l = listenSafe("clients-changed", load)
    return () => l.cleanup()
  }, [load])

  if (!setup || setup.transport !== "http") return null

  const handleSwitch = async () => {
    setSwitching(true)
    try {
      const result = await invoke<LaunchResult>("switch_claude_code_to_stdio_bridge", {
        clientId,
      } satisfies SwitchClaudeCodeToStdioBridgeParams)
      toast.success("Switched to the STDIO bridge")
      onSwitched?.(result)
      await load()
    } catch (error) {
      toast.error(`Failed to switch: ${error}`)
    } finally {
      setSwitching(false)
    }
  }

  return (
    <div className="rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 space-y-2">
      <div className="flex items-start gap-2">
        <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0 text-amber-600" />
        <div className="space-y-1">
          <p className="text-sm font-medium">Claude Code connects over HTTP: tools only</p>
          <p className="text-xs text-muted-foreground">
            <code className="bg-muted px-1 rounded">{setup.config_path}</code> uses LocalRouter&rsquo;s HTTP endpoint,
            which cannot deliver pushed events. Switch to the STDIO bridge so events from your MCP servers can wake
            Claude.
          </p>
        </div>
      </div>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button size="sm" variant="outline" disabled={switching}>
            {switching ? <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" /> : <ArrowRightLeft className="h-3.5 w-3.5 mr-1.5" />}
            Switch to STDIO bridge
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Switch Claude Code to the STDIO bridge?</AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="space-y-2">
                <p>
                  LocalRouter&rsquo;s entry in <code>{setup.config_path}</code> is replaced with a stdio server that
                  runs <code>{setup.binary_path ?? "localrouter"} --mcp-bridge</code>. A backup of the file is kept.
                </p>
                <p>Restart Claude Code afterwards with:</p>
                <code className="block bg-muted p-2 rounded text-xs">{setup.launch_command}</code>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleSwitch}>Switch</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
