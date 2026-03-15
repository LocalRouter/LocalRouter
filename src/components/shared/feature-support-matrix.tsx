import { TooltipProvider } from "@/components/ui/tooltip"
import { SupportLevelBadge } from "@/components/shared/support-level-badge"
import type {
  EndpointSupport,
  FeatureSupport,
  FeatureEndpointRow,
  FeatureModeRow,
  MatrixCell,
} from "@/types/tauri-commands"

interface ProviderFeatureTableProps {
  title: string
  items: (EndpointSupport | FeatureSupport)[]
}

export function ProviderFeatureTable({ title, items }: ProviderFeatureTableProps) {
  if (items.length === 0) return null

  return (
    <div>
      <h4 className="text-xs font-medium text-muted-foreground mb-2">{title}</h4>
      <TooltipProvider>
        <div className="border rounded-md divide-y">
          {items.map((item) => (
            <div
              key={item.name}
              className="flex items-center justify-between px-3 py-1.5 text-sm"
            >
              <span className="truncate mr-2">{item.name}</span>
              <SupportLevelBadge level={item.support} notes={item.notes} featureName={item.name} />
            </div>
          ))}
        </div>
      </TooltipProvider>
    </div>
  )
}

interface MatrixGridProps {
  title: string
  description?: string
  columnHeaders: string[]
  rows: (FeatureEndpointRow | FeatureModeRow)[]
}

function getRowName(row: FeatureEndpointRow | FeatureModeRow): string {
  return 'feature_name' in row ? row.feature_name : row.name
}

export function MatrixGrid({ title, description, columnHeaders, rows }: MatrixGridProps) {
  if (rows.length === 0) return null

  return (
    <div>
      <h4 className="text-sm font-medium mb-1">{title}</h4>
      {description && (
        <p className="text-xs text-muted-foreground mb-2">{description}</p>
      )}
      <TooltipProvider>
        <div className="border rounded-md overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left px-3 py-2 font-medium text-muted-foreground whitespace-nowrap" />
                {columnHeaders.map((header) => (
                  <th
                    key={header}
                    className="px-2 py-2 font-medium text-muted-foreground text-center whitespace-nowrap"
                  >
                    {header}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y">
              {rows.map((row) => {
                const rowName = getRowName(row)
                return (
                  <tr key={rowName} className="hover:bg-muted/30">
                    <td className="px-3 py-1.5 font-medium whitespace-nowrap">
                      {rowName}
                    </td>
                    {row.cells.map((cell: MatrixCell, i: number) => (
                      <td key={i} className="px-2 py-1.5 text-center">
                        <SupportLevelBadge
                          level={cell.support}
                          notes={cell.notes}
                          featureName={`${rowName} \u00d7 ${columnHeaders[i]}`}
                          compact
                        />
                      </td>
                    ))}
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      </TooltipProvider>
      <div className="flex items-center gap-4 mt-2 text-[10px] text-muted-foreground">
        <span className="text-green-600 dark:text-green-400">{"\u2713"} Supported</span>
        <span className="text-yellow-600 dark:text-yellow-400">P Partial</span>
        <span className="text-blue-600 dark:text-blue-400">{"\u2713*"} Via Translation</span>
        <span>{"\u2014"} N/A</span>
      </div>
    </div>
  )
}
