
import { useState } from "react"
import { ColumnDef } from "@tanstack/react-table"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { DataTable, DataTableColumnHeader } from "@/components/shared/data-table"
import { Badge } from "@/components/ui/Badge"
import {
  EntityActions,
  commonActions,
  createToggleAction,
} from "@/components/shared/entity-actions"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  allowed_llm_providers: string[]
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
  mcp_deferred_loading: boolean
  created_at: string
  last_used: string | null
}

interface ClientListProps {
  clients: Client[]
  loading: boolean
  onSelect: (clientId: string) => void
  onRefresh: () => void
}

export function ClientList({
  clients,
  loading,
  onSelect,
  onRefresh,
}: ClientListProps) {
  const [deleteTarget, setDeleteTarget] = useState<Client | null>(null)

  const handleToggleEnabled = async (client: Client) => {
    try {
      await invoke("toggle_client_enabled", {
        clientId: client.client_id,
        enabled: !client.enabled,
      })
      toast.success(`Client ${client.enabled ? "disabled" : "enabled"}`)
      onRefresh()
    } catch (error) {
      console.error("Failed to toggle client:", error)
      toast.error("Failed to update client")
    }
  }

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return
    try {
      await invoke("delete_client", { clientId: deleteTarget.client_id })
      toast.success("Client deleted")
      onRefresh()
    } catch (error) {
      console.error("Failed to delete client:", error)
      toast.error("Failed to delete client")
    } finally {
      setDeleteTarget(null)
    }
  }

  const columns: ColumnDef<Client>[] = [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Name" />
      ),
      cell: ({ row }) => (
        <div className="font-medium">{row.getValue("name")}</div>
      ),
    },
    {
      accessorKey: "client_id",
      header: "Client ID",
      cell: ({ row }) => (
        <code className="text-xs text-muted-foreground">
          {(row.getValue("client_id") as string).slice(0, 12)}...
        </code>
      ),
    },
    {
      accessorKey: "enabled",
      header: "Status",
      cell: ({ row }) => (
        <Badge variant={row.getValue("enabled") ? "success" : "secondary"}>
          {row.getValue("enabled") ? "Enabled" : "Disabled"}
        </Badge>
      ),
    },
    {
      accessorKey: "allowed_llm_providers",
      header: "Providers",
      cell: ({ row }) => {
        const providers = row.getValue("allowed_llm_providers") as string[]
        return (
          <span className="text-sm text-muted-foreground">
            {providers.length === 0 ? "All" : providers.length}
          </span>
        )
      },
    },
    {
      accessorKey: "mcp_access_mode",
      header: "MCP",
      cell: ({ row }) => {
        const mode = row.getValue("mcp_access_mode") as string
        const servers = row.original.mcp_servers as string[]
        return (
          <span className="text-sm text-muted-foreground">
            {mode === 'all' ? "All" : mode === 'specific' ? servers.length : "None"}
          </span>
        )
      },
    },
    {
      accessorKey: "last_used",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Last Used" />
      ),
      cell: ({ row }) => {
        const lastUsed = row.getValue("last_used") as string | null
        if (!lastUsed) return <span className="text-muted-foreground">Never</span>
        const date = new Date(lastUsed)
        return (
          <span className="text-sm text-muted-foreground">
            {date.toLocaleDateString()}
          </span>
        )
      },
    },
    {
      id: "actions",
      cell: ({ row }) => {
        const client = row.original
        return (
          <EntityActions
            actions={[
              commonActions.edit(() => onSelect(client.client_id)),
              createToggleAction(client.enabled, () =>
                handleToggleEnabled(client)
              ),
              commonActions.delete(() => setDeleteTarget(client)),
            ]}
          />
        )
      },
    },
  ]

  return (
    <>
      <DataTable
        columns={columns}
        data={clients}
        searchKey="name"
        searchPlaceholder="Search clients..."
        loading={loading}
        onRowClick={(client) => onSelect(client.client_id)}
        emptyMessage="No clients found. Create one to get started."
      />

      <AlertDialog open={!!deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Client?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete "{deleteTarget?.name}" and revoke its API key.
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteConfirm}
              className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
