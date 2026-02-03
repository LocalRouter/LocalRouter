import * as React from "react"
import { cn } from "@/lib/utils"

export interface TemplateItem<T = unknown> {
  id: string
  name: string
  description: string
  icon?: React.ReactNode
  data?: T
}

export interface TemplateCategory<T = unknown> {
  id: string
  title: string
  description?: string
  items: TemplateItem<T>[]
}

interface TemplateGridProps<T> {
  categories: TemplateCategory<T>[]
  onSelect: (item: TemplateItem<T>) => void
  customEntry?: {
    label: string
    description: string
    icon?: React.ReactNode
    onClick: () => void
  }
  columns?: 2 | 3
  className?: string
}

export function TemplateGrid<T>({
  categories,
  onSelect,
  customEntry,
  columns = 3,
  className,
}: TemplateGridProps<T>) {
  const gridCols = columns === 2 ? "grid-cols-2" : "grid-cols-2 sm:grid-cols-3"

  return (
    <div className={cn("space-y-6", className)}>
      {customEntry && (
        <button
          onClick={customEntry.onClick}
          className="w-full flex items-center gap-3 p-3 rounded-lg border-2 border-dashed border-muted hover:border-primary hover:bg-accent transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
        >
          {customEntry.icon && (
            <span className="text-2xl shrink-0">{customEntry.icon}</span>
          )}
          <div className="text-left">
            <p className="font-medium text-sm">{customEntry.label}</p>
            <p className="text-xs text-muted-foreground">{customEntry.description}</p>
          </div>
        </button>
      )}

      {categories.map((category) => {
        if (category.items.length === 0) return null
        return (
          <div key={category.id} className="space-y-3">
            <div>
              <h3 className="text-sm font-semibold">{category.title}</h3>
              {category.description && (
                <p className="text-xs text-muted-foreground">{category.description}</p>
              )}
            </div>
            <div className={cn("grid gap-3", gridCols)}>
              {category.items.map((item) => (
                <TemplateButton
                  key={item.id}
                  item={item}
                  onSelect={() => onSelect(item)}
                />
              ))}
            </div>
          </div>
        )
      })}
    </div>
  )
}

interface TemplateButtonProps<T> {
  item: TemplateItem<T>
  onSelect: () => void
}

function TemplateButton<T>({ item, onSelect }: TemplateButtonProps<T>) {
  return (
    <button
      onClick={onSelect}
      className={cn(
        "flex flex-col items-center gap-2 p-4 rounded-lg border-2 border-muted",
        "hover:border-primary hover:bg-accent transition-colors",
        "focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
      )}
    >
      {item.icon && (
        <div className="flex items-center justify-center h-10 w-10">
          {item.icon}
        </div>
      )}
      <div className="text-center">
        <p className="font-medium text-sm">{item.name}</p>
        <p className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
          {item.description}
        </p>
      </div>
    </button>
  )
}
