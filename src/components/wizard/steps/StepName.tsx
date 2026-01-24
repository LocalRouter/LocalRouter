/**
 * Step 1: Name Your Client
 *
 * Simple name input for the new client.
 */

import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"

interface StepNameProps {
  name: string
  onChange: (name: string) => void
}

export function StepName({ name, onChange }: StepNameProps) {
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <p className="text-sm text-muted-foreground">
          Choose a name that helps you identify this client later.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="client-name">Client Name</Label>
        <Input
          id="client-name"
          placeholder="e.g., OpenCode, Development, All MCPs"
          value={name}
          onChange={(e) => onChange(e.target.value)}
          autoFocus
        />
        <p className="text-xs text-muted-foreground">
          Examples: &quot;OpenCode&quot;, &quot;IDE Integration&quot;, &quot;Production App&quot;
        </p>
      </div>
    </div>
  )
}
