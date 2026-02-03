import * as React from "react"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

export interface TabConfig {
  value: string
  label: string
  icon?: React.ReactNode
  disabled?: boolean
  disabledReason?: string
}

interface AddResourceTabsProps {
  tabs: TabConfig[]
  value: string
  onValueChange: (value: string) => void
  className?: string
}

export function AddResourceTabs({
  tabs,
  value,
  onValueChange,
  className,
}: AddResourceTabsProps) {
  return (
    <TooltipProvider>
      <Tabs value={value} onValueChange={onValueChange} className={className}>
        <TabsList className={cn("grid w-full", `grid-cols-${tabs.length}`)}>
          {tabs.map((tab) => (
            <TabTriggerWithTooltip
              key={tab.value}
              tab={tab}
            />
          ))}
        </TabsList>
      </Tabs>
    </TooltipProvider>
  )
}

interface TabTriggerWithTooltipProps {
  tab: TabConfig
}

function TabTriggerWithTooltip({ tab }: TabTriggerWithTooltipProps) {
  const trigger = (
    <TabsTrigger
      value={tab.value}
      disabled={tab.disabled}
      onClick={(e) => {
        if (tab.disabled) {
          e.preventDefault()
        }
      }}
      className={cn(
        "gap-2",
        tab.disabled && "opacity-50 cursor-not-allowed"
      )}
    >
      {tab.icon}
      {tab.label}
    </TabsTrigger>
  )

  if (tab.disabled && tab.disabledReason) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>{trigger}</TooltipTrigger>
        <TooltipContent>
          <p>{tab.disabledReason}</p>
        </TooltipContent>
      </Tooltip>
    )
  }

  return trigger
}

export { Tabs, TabsList, TabsTrigger }
