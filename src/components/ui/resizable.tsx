import * as React from "react"
import { GripVertical } from "lucide-react"
import {
  Panel,
  Group,
  Separator,
} from "react-resizable-panels"

import { cn } from "@/lib/utils"

// Map legacy 'direction' prop to new 'orientation' prop
type ResizablePanelGroupProps = Omit<React.ComponentProps<typeof Group>, 'orientation'> & {
  direction?: "horizontal" | "vertical"
}

const ResizablePanelGroup = ({
  className,
  direction,
  ...props
}: ResizablePanelGroupProps) => (
  <Group
    orientation={direction}
    className={cn(
      "flex h-full w-full data-[panel-group-direction=vertical]:flex-col",
      className
    )}
    {...props}
  />
)

const ResizablePanel = Panel

const ResizableHandle = ({
  withHandle,
  className,
  ...props
}: React.ComponentProps<typeof Separator> & {
  withHandle?: boolean
}) => (
  <Separator
    className={cn(
      // Base
      "relative flex items-center justify-center bg-border",
      "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1",
      // Horizontal split (vertical divider bar)
      "w-px cursor-col-resize",
      "after:absolute after:inset-y-0 after:left-1/2 after:w-3 after:-translate-x-1/2",
      // Vertical split (horizontal divider bar)
      "data-[panel-group-direction=vertical]:h-px data-[panel-group-direction=vertical]:w-full data-[panel-group-direction=vertical]:cursor-row-resize",
      "data-[panel-group-direction=vertical]:after:left-0 data-[panel-group-direction=vertical]:after:h-3 data-[panel-group-direction=vertical]:after:w-full",
      "data-[panel-group-direction=vertical]:after:-translate-y-1/2 data-[panel-group-direction=vertical]:after:translate-x-0",
      // Rotate grip icon for vertical direction
      "[&[data-panel-group-direction=vertical]>div]:rotate-90",
      className
    )}
    {...props}
  >
    {withHandle && (
      <div className="z-10 flex h-4 w-3 items-center justify-center rounded-sm border bg-border">
        <GripVertical className="h-2.5 w-2.5" />
      </div>
    )}
  </Separator>
)

export { ResizablePanelGroup, ResizablePanel, ResizableHandle }
