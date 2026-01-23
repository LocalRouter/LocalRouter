import * as React from "react"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Button } from "@/components/ui/Button"
import {
  MoreHorizontal,
  Edit,
  Trash2,
  Copy,
  Power,
  PowerOff,
  ExternalLink,
  RefreshCw,
} from "lucide-react"
import { cn } from "@/lib/utils"

interface Action {
  id: string
  label: string
  icon?: React.ComponentType<{ className?: string }>
  onClick: () => void
  variant?: "default" | "destructive"
  disabled?: boolean
}

interface EntityActionsProps {
  actions: Action[]
  label?: string
  className?: string
}

export function EntityActions({
  actions,
  label = "Actions",
  className,
}: EntityActionsProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn("h-8 w-8", className)}
          onClick={(e) => e.stopPropagation()}
        >
          <MoreHorizontal className="h-4 w-4" />
          <span className="sr-only">{label}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-[160px]">
        <DropdownMenuLabel>{label}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {actions.map((action) => {
          const Icon = action.icon
          return (
            <DropdownMenuItem
              key={action.id}
              onSelect={(e) => {
                e.preventDefault()
                action.onClick()
              }}
              disabled={action.disabled}
              className={cn(
                action.variant === "destructive" &&
                  "text-destructive focus:text-destructive"
              )}
            >
              {Icon && <Icon className="mr-2 h-4 w-4" />}
              {action.label}
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// Pre-built action configs for common operations
export const commonActions = {
  edit: (onClick: () => void): Action => ({
    id: "edit",
    label: "Edit",
    icon: Edit,
    onClick,
  }),
  delete: (onClick: () => void): Action => ({
    id: "delete",
    label: "Delete",
    icon: Trash2,
    onClick,
    variant: "destructive",
  }),
  duplicate: (onClick: () => void): Action => ({
    id: "duplicate",
    label: "Duplicate",
    icon: Copy,
    onClick,
  }),
  enable: (onClick: () => void): Action => ({
    id: "enable",
    label: "Enable",
    icon: Power,
    onClick,
  }),
  disable: (onClick: () => void): Action => ({
    id: "disable",
    label: "Disable",
    icon: PowerOff,
    onClick,
  }),
  openExternal: (onClick: () => void): Action => ({
    id: "openExternal",
    label: "Open",
    icon: ExternalLink,
    onClick,
  }),
  refresh: (onClick: () => void): Action => ({
    id: "refresh",
    label: "Refresh",
    icon: RefreshCw,
    onClick,
  }),
}

// Toggle enable/disable action
export function createToggleAction(
  enabled: boolean,
  onToggle: () => void
): Action {
  return enabled
    ? commonActions.disable(onToggle)
    : commonActions.enable(onToggle)
}
