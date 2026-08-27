import { useState, useEffect, useRef, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Monitor, BarChart3, ArrowUp, ArrowDown, X } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Checkbox } from "@/components/ui/checkbox"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/Input"
import { Button } from "@/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type {
  TrayGraphSettings,
  TrayStatsSettings,
  TrayStatsConfig,
  TrayStatsItem,
  TraySource,
  TrayGraphMetric,
  TrayUsageMetric,
  TrayUsagePeriod,
  TrayLayout,
  UpdateTrayStatsConfigParams,
  ClientInfo,
  ProviderInstanceInfo,
} from "@/types/tauri-commands"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
import { InfoTooltip } from "@/components/ui/info-tooltip"

/** Static tray icon preview — LocalRouter logo in a rounded frame */
function StaticIconPreview() {
  return (
    <svg width="36" height="36" viewBox="0 0 32 32" fill="none" className="shrink-0">
      {/* Rounded border frame */}
      <rect x="0.5" y="0.5" width="31" height="31" rx="5.5" stroke="currentColor" strokeOpacity={0.5} fill="none" />
      {/* LocalRouter logo scaled to fit */}
      <g transform="translate(4, 4) scale(0.24)">
        <circle cx="20" cy="20" r="12" stroke="currentColor" strokeWidth="10" fill="none" />
        <circle cx="80" cy="80" r="12" stroke="currentColor" strokeWidth="10" fill="none" />
        <path
          d="M 32 22 C 75 15, 90 40, 50 50 C 10 60, 25 85, 68 78"
          stroke="currentColor"
          strokeWidth="10"
          strokeLinecap="round"
          fill="none"
        />
      </g>
    </svg>
  )
}

/** Animated activity graph preview — scrolling sparkline bars */
function GraphIconPreview() {
  const BAR_COUNT = 20
  const [bars, setBars] = useState<number[]>(() =>
    Array.from({ length: BAR_COUNT }, () => Math.random() * 0.8 + 0.1)
  )

  const tick = useCallback(() => {
    setBars(prev => {
      const next = prev.slice(1)
      // Generate next bar influenced by the previous value for smoother movement
      const last = prev[prev.length - 1]
      const delta = (Math.random() - 0.5) * 0.35
      const newVal = Math.max(0.05, Math.min(1, last + delta))
      next.push(newVal)
      return next
    })
  }, [])

  useEffect(() => {
    const id = setInterval(tick, 180)
    return () => clearInterval(id)
  }, [tick])

  const padding = 3
  const barAreaWidth = 36 - padding * 2
  const barAreaHeight = 32 - padding * 2
  const barWidth = barAreaWidth / BAR_COUNT

  return (
    <svg width="36" height="36" viewBox="0 0 36 36" fill="none" className="shrink-0">
      {/* Rounded border frame */}
      <rect x="0.5" y="0.5" width="35" height="35" rx="5.5" stroke="currentColor" strokeOpacity={0.5} fill="none" />
      {/* Animated bars */}
      {bars.map((h, i) => {
        const barH = h * barAreaHeight
        return (
          <rect
            key={i}
            x={padding + i * barWidth}
            y={padding + barAreaHeight - barH}
            width={Math.max(barWidth - 0.5, 0.5)}
            height={barH}
            fill="currentColor"
            opacity={0.7}
          />
        )
      })}
    </svg>
  )
}

/** Key that identifies a TraySource in React lists / lookups */
function sourceKey(s: TraySource): string {
  switch (s.kind) {
    case "all":
      return "all"
    case "client":
      return `client:${s.id}`
    case "provider":
      return `provider:${s.instance}`
    case "model":
      return `model:${s.id}`
  }
}

function sameSource(a: TraySource, b: TraySource): boolean {
  return sourceKey(a) === sourceKey(b)
}

/** Mirror of lr_config::normalize_tray_label */
function normalizeLabel(seed: string): string {
  return seed.replace(/[^A-Za-z0-9]/g, "").toUpperCase().slice(0, 4)
}

/** Static multi-panel preview of the tray icon for the current config */
function TrayStatsPreview({
  config,
  labels,
}: {
  config: TrayStatsConfig
  labels: string[]
}) {
  const PANE = 32
  const LABEL_W = 6
  const BAR_W = 4
  const paneW =
    (config.show_labels ? LABEL_W : 0) +
    (config.show_graph || !config.show_usage_bar ? PANE : 0) +
    (config.show_usage_bar ? BAR_W : 0)
  const count = Math.max(1, Math.min(labels.length, 6))
  const width = Math.max(paneW * count, PANE)
  const scale = 1.5
  const bars = useMemo(
    () =>
      Array.from({ length: count }, (_, p) =>
        Array.from({ length: 26 }, (_, i) => 0.15 + 0.8 * Math.abs(Math.sin((i + 1) * (p + 1.7) * 0.9))),
      ),
    [count],
  )

  return (
    <svg
      width={width * scale}
      height={PANE * scale}
      viewBox={`0 0 ${width} ${PANE}`}
      className="shrink-0"
      shapeRendering="crispEdges"
    >
      {Array.from({ length: count }, (_, p) => {
        let x = p * paneW
        const els: React.ReactNode[] = []
        if (config.show_labels) {
          const label = (labels[p] ?? "").padEnd(4)
          els.push(
            <text
              key="label"
              x={x + 2.5}
              y={0}
              fontSize={7}
              fontFamily="ui-monospace, monospace"
              fill="currentColor"
              textAnchor="middle"
            >
              {label
                .trim()
                .split("")
                .map((ch, i) => (
                  <tspan key={i} x={x + 2.5} y={6.5 + i * 8}>
                    {ch}
                  </tspan>
                ))}
            </text>,
          )
          x += LABEL_W
        }
        if (config.show_graph || !config.show_usage_bar) {
          els.push(
            <rect
              key="frame"
              x={x + 0.5}
              y={0.5}
              width={PANE - 1}
              height={PANE - 1}
              rx={5.5}
              stroke="currentColor"
              strokeOpacity={0.6}
              fill="none"
            />,
          )
          bars[p].forEach((h, i) => {
            const bh = Math.max(1, Math.round(h * 26))
            els.push(
              <rect
                key={`b${i}`}
                x={x + 3 + i}
                y={3 + 26 - bh}
                width={1}
                height={bh}
                fill="currentColor"
                opacity={0.8}
              />,
            )
          })
          x += PANE
        }
        if (config.show_usage_bar) {
          const fill = Math.max(0.1, 1 - p * 0.35)
          els.push(
            <rect key="track" x={x + 1} y={0} width={2} height={PANE} fill="currentColor" opacity={0.25} />,
            <rect
              key="fill"
              x={x + 1}
              y={PANE - Math.round(fill * PANE)}
              width={2}
              height={Math.round(fill * PANE)}
              fill="currentColor"
            />,
          )
        }
        return <g key={p}>{els}</g>
      })}
    </svg>
  )
}

function TrayStatsCard({ graphEnabled }: { graphEnabled: boolean }) {
  const [settings, setSettings] = useState<TrayStatsSettings | null>(null)
  const [clients, setClients] = useState<ClientInfo[]>([])
  const [providers, setProviders] = useState<ProviderInstanceInfo[]>([])
  const { models } = useIncrementalModels({ refreshOnMount: false })
  const loaded = useRef(false)
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    ;(async () => {
      try {
        const [s, c, p] = await Promise.all([
          invoke<TrayStatsSettings>("get_tray_stats_settings"),
          invoke<ClientInfo[]>("list_clients").catch(() => [] as ClientInfo[]),
          invoke<ProviderInstanceInfo[]>("list_provider_instances").catch(() => [] as ProviderInstanceInfo[]),
        ])
        setSettings(s)
        setClients(c)
        setProviders(p)
        setTimeout(() => {
          loaded.current = true
        }, 0)
      } catch (error) {
        console.error("Failed to load tray stats settings:", error)
      }
    })()
  }, [])

  const config = settings?.config
  const platform = settings?.platform

  const updateConfig = (patch: Partial<TrayStatsConfig> | ((c: TrayStatsConfig) => TrayStatsConfig)) => {
    setSettings((prev) => {
      if (!prev) return prev
      const next = typeof patch === "function" ? patch(prev.config) : { ...prev.config, ...patch }
      return { ...prev, config: next }
    })
  }

  // Debounced auto-save (label typing would otherwise save every keystroke)
  useEffect(() => {
    if (!loaded.current || !config) return
    if (saveTimer.current) clearTimeout(saveTimer.current)
    saveTimer.current = setTimeout(async () => {
      try {
        await invoke("update_tray_stats_config", { config } satisfies UpdateTrayStatsConfigParams)
      } catch (error: any) {
        console.error("Failed to save tray stats settings:", error)
        toast.error(`Failed to save: ${error.message || error}`)
      }
    }, 400)
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current)
    }
  }, [config])

  if (!config || !platform) return null

  const clientName = (id: string) => clients.find((c) => c.client_id === id)?.name
  const sourceName = (s: TraySource): string => {
    switch (s.kind) {
      case "all":
        return "All requests"
      case "client":
        return clientName(s.id) ?? `${s.id.slice(0, 8)}… (removed)`
      case "provider":
        return s.instance
      case "model":
        return s.id
    }
  }
  const sourceExists = (s: TraySource): boolean =>
    s.kind !== "client" || clients.some((c) => c.client_id === s.id)
  const defaultLabel = (s: TraySource): string => {
    switch (s.kind) {
      case "all":
        return "ALL"
      case "client":
        return normalizeLabel(clientName(s.id) ?? s.id)
      case "provider":
        return normalizeLabel(s.instance)
      case "model":
        return normalizeLabel(s.id.split("/").pop() ?? s.id)
    }
  }
  const effectiveLabel = (item: TrayStatsItem) =>
    (item.label && normalizeLabel(item.label)) || defaultLabel(item.source) || "?"

  const has = (s: TraySource) => config.items.some((i) => sameSource(i.source, s))
  const addable: { group: string; source: TraySource; name: string }[] = [
    ...clients
      .filter((c) => !has({ kind: "client", id: c.client_id }))
      .map((c) => ({ group: "Clients", source: { kind: "client", id: c.client_id } as TraySource, name: c.name })),
    ...providers
      .filter((p) => !has({ kind: "provider", instance: p.instance_name }))
      .map((p) => ({
        group: "Providers",
        source: { kind: "provider", instance: p.instance_name } as TraySource,
        name: p.instance_name,
      })),
    ...models
      .map((m) => `${m.provider}/${m.id}`)
      .filter((id, i, arr) => arr.indexOf(id) === i && !has({ kind: "model", id }))
      .map((id) => ({ group: "Models", source: { kind: "model", id } as TraySource, name: id })),
  ]
  const groups = ["Clients", "Providers", "Models"].filter((g) => addable.some((a) => a.group === g))

  const move = (index: number, dir: -1 | 1) =>
    updateConfig((c) => {
      const items = [...c.items]
      const j = index + dir
      if (j < 0 || j >= items.length) return c
      ;[items[index], items[j]] = [items[j], items[index]]
      return { ...c, items }
    })

  const extendedHere = (config.layout === "auto" ? platform.default_layout : config.layout) === "extended"
  const enabledLabels = config.items
    .filter((i) => i.enabled && sourceExists(i.source))
    .map(effectiveLabel)

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm flex items-center gap-2">
          <BarChart3 className="h-4 w-4" />
          Tray Stats
        </CardTitle>
        <CardDescription>
          Which clients and LLMs the tray icon and tray menu report on, and how
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        {!extendedHere && (
          <p className="text-xs text-muted-foreground rounded-md border border-muted p-2">
            This desktop can only show a single square tray icon, so the icon keeps the global graph.
            Per-item usage is still listed in the tray menu.
          </p>
        )}

        {extendedHere && graphEnabled && (
          <div className="flex items-center gap-3 text-foreground">
            <TrayStatsPreview config={config} labels={enabledLabels} />
            <p className="text-xs text-muted-foreground">Preview (labels + panels in display order)</p>
          </div>
        )}

        {/* Items */}
        <div className="space-y-2">
          <Label className="flex items-center gap-1">
            Items
            <InfoTooltip content="Each enabled item becomes one panel in the tray icon (up to 6) and one line in the tray menu. Labels are up to 4 letters, stacked vertically beside the panel." />
          </Label>
          <div className="rounded-md border divide-y">
            {config.items.map((item, index) => {
              const key = sourceKey(item.source)
              const exists = sourceExists(item.source)
              return (
                <div key={key} className={`flex items-center gap-2 px-2 py-1.5 ${exists ? "" : "opacity-60"}`}>
                  <Checkbox
                    checked={item.enabled}
                    onCheckedChange={(v) =>
                      updateConfig((c) => ({
                        ...c,
                        items: c.items.map((i) => (sameSource(i.source, item.source) ? { ...i, enabled: v === true } : i)),
                      }))
                    }
                  />
                  <Input
                    value={item.label ?? ""}
                    placeholder={defaultLabel(item.source) || "?"}
                    maxLength={4}
                    className="h-7 w-16 font-mono uppercase text-xs"
                    onChange={(e) =>
                      updateConfig((c) => ({
                        ...c,
                        items: c.items.map((i) =>
                          sameSource(i.source, item.source)
                            ? { ...i, label: normalizeLabel(e.target.value) || null }
                            : i,
                        ),
                      }))
                    }
                  />
                  <span className="flex-1 truncate text-sm">
                    <span className="text-muted-foreground text-xs mr-1.5">
                      {item.source.kind === "all"
                        ? ""
                        : item.source.kind === "client"
                          ? "client"
                          : item.source.kind}
                    </span>
                    {sourceName(item.source)}
                  </span>
                  <Button variant="ghost" size="icon" className="h-7 w-7" disabled={index === 0} onClick={() => move(index, -1)}>
                    <ArrowUp className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    disabled={index === config.items.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    <ArrowDown className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    disabled={item.source.kind === "all"}
                    onClick={() =>
                      updateConfig((c) => ({ ...c, items: c.items.filter((i) => !sameSource(i.source, item.source)) }))
                    }
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                </div>
              )
            })}
          </div>
          <div className="flex items-center gap-3">
            <Select
              value=""
              onValueChange={(key) => {
                const entry = addable.find((a) => sourceKey(a.source) === key)
                if (entry) updateConfig((c) => ({ ...c, items: [...c.items, { source: entry.source, enabled: true, label: null }] }))
              }}
            >
              <SelectTrigger className="w-64">
                <SelectValue placeholder="Add client, provider or model…" />
              </SelectTrigger>
              <SelectContent>
                {groups.map((g) => (
                  <SelectGroup key={g}>
                    <SelectLabel>{g}</SelectLabel>
                    {addable
                      .filter((a) => a.group === g)
                      .map((a) => (
                        <SelectItem key={sourceKey(a.source)} value={sourceKey(a.source)}>
                          {a.name}
                        </SelectItem>
                      ))}
                  </SelectGroup>
                ))}
                {groups.length === 0 && (
                  <div className="px-2 py-1.5 text-xs text-muted-foreground">Nothing left to add</div>
                )}
              </SelectContent>
            </Select>
            <div className="flex items-center gap-2">
              <Switch
                id="tray-auto-add"
                checked={config.auto_add_clients}
                onCheckedChange={(v) => updateConfig({ auto_add_clients: v })}
              />
              <Label htmlFor="tray-auto-add" className="text-xs cursor-pointer">
                Automatically add new clients
              </Label>
            </div>
          </div>
        </div>

        {/* Show */}
        <div className="space-y-2">
          <Label>Show</Label>
          <div className="flex flex-wrap gap-x-5 gap-y-2">
            {(
              [
                ["show_labels", "Labels", "Stacked 4-letter label beside each panel"],
                ["show_graph", "Graph", "Sparkline panel per item"],
                ["show_usage_bar", "Usage bar", "Thin bar showing each item's usage relative to the largest item"],
                ["show_text", "Numbers beside icon", "Usage numbers drawn as text right of the tray icon (macOS / GNOME)"],
              ] as const
            )
              .filter(([k]) => !(k === "show_text" && platform.os === "windows"))
              .map(([k, label, tip]) => (
                <div key={k} className="flex items-center gap-2">
                  <Checkbox
                    id={`tray-${k}`}
                    checked={config[k]}
                    disabled={!extendedHere}
                    onCheckedChange={(v) => updateConfig({ [k]: v === true } as Partial<TrayStatsConfig>)}
                  />
                  <Label htmlFor={`tray-${k}`} className="text-xs cursor-pointer flex items-center gap-1">
                    {label}
                    <InfoTooltip content={tip} />
                  </Label>
                </div>
              ))}
          </div>
        </div>

        {/* Metrics */}
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <div className="space-y-1.5">
            <Label className="text-xs">Graph metric</Label>
            <Select value={config.graph_metric} onValueChange={(v) => updateConfig({ graph_metric: v as TrayGraphMetric })}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="tokens">Tokens</SelectItem>
                <SelectItem value="requests">Requests</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label className="text-xs">Usage period</Label>
            <Select value={config.usage_period} onValueChange={(v) => updateConfig({ usage_period: v as TrayUsagePeriod })}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="hour">Last 1 hour</SelectItem>
                <SelectItem value="day">Last 24 hours</SelectItem>
                <SelectItem value="week">Last 7 days</SelectItem>
                <SelectItem value="month">Last 30 days</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label className="text-xs">Usage metric</Label>
            <Select value={config.usage_metric} onValueChange={(v) => updateConfig({ usage_metric: v as TrayUsageMetric })}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="tokens">Tokens</SelectItem>
                <SelectItem value="cost">Cost</SelectItem>
                <SelectItem value="requests">Requests</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          The usage period and metric drive the usage bar, the numbers beside the icon and the tray menu lines. The
          graph itself always shows the recent window set by the refresh rate above.
        </p>

        {platform.os === "linux" && (
          <div className="space-y-1.5">
            <Label className="text-xs flex items-center gap-1">
              Layout
              <InfoTooltip content="Wide multi-panel icons and text beside the icon work on GNOME (AppIndicator extension). KDE and most other panels squash wide icons into a square — use Compact there." />
            </Label>
            <Select value={config.layout} onValueChange={(v) => updateConfig({ layout: v as TrayLayout })}>
              <SelectTrigger className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">Auto (detected: {platform.default_layout})</SelectItem>
                <SelectItem value="extended">Extended (wide icon + text)</SelectItem>
                <SelectItem value="compact">Compact (single square icon)</SelectItem>
              </SelectContent>
            </Select>
          </div>
        )}
      </CardContent>
    </Card>
  )
}

export function AppearanceTab() {
  const [settings, setSettings] = useState<TrayGraphSettings>({
    enabled: false,
    refresh_rate_secs: 10,
  })
  const loaded = useRef(false)

  useEffect(() => {
    loadSettings()
  }, [])

  // Auto-save whenever settings change (skip initial load)
  useEffect(() => {
    if (!loaded.current) return
    saveSettings(settings)
  }, [settings.enabled, settings.refresh_rate_secs])

  const loadSettings = async () => {
    try {
      const result = await invoke<TrayGraphSettings>("get_tray_graph_settings")
      setSettings(result)
      // Mark as loaded after state is set so the effect doesn't fire for initial load
      setTimeout(() => { loaded.current = true }, 0)
    } catch (error) {
      console.error("Failed to load tray graph settings:", error)
    }
  }

  const saveSettings = async (s: TrayGraphSettings) => {
    try {
      await invoke("update_tray_graph_settings", {
        enabled: s.enabled,
        refreshRateSecs: s.refresh_rate_secs,
      })
    } catch (error: any) {
      console.error("Failed to save appearance settings:", error)
      toast.error(`Failed to save: ${error.message || error}`)
    }
  }

  const calculateTimeWindow = (refreshRateSecs: number): string => {
    // 26 bars per panel (see GRAPH_WIDTH in src-tauri/src/ui/tray_graph.rs)
    const totalSecs = 26 * refreshRateSecs
    if (totalSecs < 60) {
      return `${totalSecs} seconds`
    }
    const mins = Math.floor(totalSecs / 60)
    const secs = totalSecs % 60
    return secs > 0 ? `${mins}m ${secs}s` : `${mins} minute${mins > 1 ? "s" : ""}`
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Monitor className="h-4 w-4" />
            Tray Icon
          </CardTitle>
          <CardDescription>
            Choose how the system tray icon displays activity
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-3">
            <Label className="flex items-center gap-1">
              Icon Mode
              <InfoTooltip content="Choose between a static tray icon or a live activity graph showing request throughput." />
            </Label>
            <RadioGroup
              value={settings.enabled ? "graph" : "static"}
              onValueChange={(value) =>
                setSettings({ ...settings, enabled: value === "graph" })
              }
            >
              <div className="flex items-start space-x-3">
                <RadioGroupItem value="static" id="mode-static" className="mt-0.5" />
                <div className="flex items-start gap-3">
                  <StaticIconPreview />
                  <div>
                    <Label htmlFor="mode-static" className="font-medium cursor-pointer">Static Icon</Label>
                    <p className="text-xs text-muted-foreground">
                      Clean icon with notification overlays for approvals, health issues, and updates
                    </p>
                  </div>
                </div>
              </div>
              <div className="flex items-start space-x-3">
                <RadioGroupItem value="graph" id="mode-graph" className="mt-0.5" />
                <div className="flex items-start gap-3">
                  <GraphIconPreview />
                  <div>
                    <Label htmlFor="mode-graph" className="font-medium cursor-pointer">Activity Graph</Label>
                    <p className="text-xs text-muted-foreground">
                      Live token usage sparkline that updates in real-time as requests flow through
                    </p>
                  </div>
                </div>
              </div>
            </RadioGroup>
          </div>

          {settings.enabled && (
            <div className="space-y-2 pl-6 border-l-2 border-muted">
              <Label className="flex items-center gap-1">
                Graph Refresh Rate
                <InfoTooltip content="How often the activity graph redraws. Faster rates show more detail but use slightly more CPU." />
              </Label>
              <Select
                value={settings.refresh_rate_secs.toString()}
                onValueChange={(value) =>
                  setSettings({ ...settings, refresh_rate_secs: parseInt(value) })
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="1">Fast (1s refresh, 26s window)</SelectItem>
                  <SelectItem value="10">Medium (10s refresh, 4m 20s window)</SelectItem>
                  <SelectItem value="60">Slow (60s refresh, 26m window)</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                Window: {calculateTimeWindow(settings.refresh_rate_secs)}
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      <TrayStatsCard graphEnabled={settings.enabled} />
    </div>
  )
}
