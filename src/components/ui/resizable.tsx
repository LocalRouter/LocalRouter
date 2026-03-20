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
      "flex h-full w-full",
      direction === "vertical" && "flex-col",
      className
    )}
    {...props}
  />
)

const ResizablePanel = Panel

const ResizableHandle = ({
  withHandle,
  orientation = "horizontal",
  className,
  ...props
}: React.ComponentProps<typeof Separator> & {
  withHandle?: boolean
  orientation?: "horizontal" | "vertical"
}) => (
  <Separator
    className={cn(
      // Base
      "relative flex items-center justify-center bg-border",
      "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1",
      orientation === "horizontal"
        ? [
            // Horizontal split (vertical divider bar)
            "w-px cursor-col-resize",
            "after:absolute after:inset-y-0 after:left-1/2 after:w-3 after:-translate-x-1/2",
          ]
        : [
            // Vertical split (horizontal divider bar)
            "h-px w-full cursor-row-resize",
            "after:absolute after:inset-x-0 after:top-1/2 after:h-3 after:-translate-y-1/2",
          ],
      className
    )}
    {...props}
  >
    {withHandle && (
      <div className={cn(
        "z-10 flex items-center justify-center rounded-sm border bg-border",
        orientation === "horizontal" ? "h-4 w-3" : "h-3 w-4 rotate-90"
      )}>
        <GripVertical className="h-2.5 w-2.5" />
      </div>
    )}
  </Separator>
)

export { ResizablePanelGroup, ResizablePanel, ResizableHandle }
