import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Users, ExternalLink } from "lucide-react"
import type { ClientFeatureStatus, GetFeatureClientsStatusParams } from "@/types/tauri-commands"

interface FeatureClientsCardProps {
  feature: GetFeatureClientsStatusParams['feature']
  /** Which client tab to navigate to (default: "models") */
  clientTab?: string
  onNavigateToClient?: (view: string, subTab?: string | null) => void
}

export function FeatureClientsCard({ feature, clientTab = "models", onNavigateToClient }: FeatureClientsCardProps) {
  const [statuses, setStatuses] = useState<ClientFeatureStatus[]>([])
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    try {
      const result = await invoke<ClientFeatureStatus[]>("get_feature_clients_status", {
        feature,
      } satisfies GetFeatureClientsStatusParams)
      setStatuses(result)
    } catch (err) {
      console.error("Failed to load feature client statuses:", err)
    } finally {
      setLoading(false)
    }
  }, [feature])

  useEffect(() => {
    load()

    const listeners = [
      listenSafe("clients-changed", load),
      listenSafe("config-changed", load),
    ]

    return () => {
      listeners.forEach(l => l.cleanup())
    }
  }, [load])

  const activeClients = statuses.filter((s) => s.active)
  const inactiveClients = statuses.filter((s) => !s.active)

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm font-medium flex items-center gap-2">
          <Users className="h-4 w-4" />
          Clients
          {!loading && (
            <span className="text-muted-foreground font-normal">
              {activeClients.length} active
              {inactiveClients.length > 0 && `, ${inactiveClients.length} inactive`}
            </span>
          )}
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-0">
        {loading ? (
          <p className="text-sm text-muted-foreground">Loading...</p>
        ) : statuses.length === 0 ? (
          <p className="text-sm text-muted-foreground">No clients configured</p>
        ) : (
          <div className="space-y-1.5">
            {statuses.map((s) => (
              <div
                key={s.client_id}
                className="flex items-center justify-between py-1 px-2 rounded-md hover:bg-muted/50 group"
              >
                <div className="flex items-center gap-2 min-w-0">
                  {onNavigateToClient ? (
                    <button
                      onClick={() => onNavigateToClient("clients", `${s.client_id}|${clientTab}`)}
                      className="text-sm font-medium truncate hover:underline text-left"
                    >
                      {s.client_name}
                    </button>
                  ) : (
                    <span className="text-sm font-medium truncate">{s.client_name}</span>
                  )}
                  {s.source === "override" && (
                    <Badge variant="outline" className="text-[10px] px-1 py-0 shrink-0">
                      Override
                    </Badge>
                  )}
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <Badge
                    variant={s.active ? "success" : "secondary"}
                    className="text-[10px] px-1.5 py-0"
                  >
                    {s.active ? "Active" : "Inactive"}
                  </Badge>
                  {onNavigateToClient && (
                    <button
                      onClick={() => onNavigateToClient("clients", `${s.client_id}|${clientTab}`)}
                      className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-foreground transition-opacity"
                      title="Go to client settings"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
