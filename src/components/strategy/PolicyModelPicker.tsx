import { useMemo, useState } from 'react'
import { Search } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/Modal'
import { Command, CommandInput, CommandList, CommandItem, CommandEmpty } from '@/components/ui/command'
import { ModelPricingBadge, pricingSourceLabel } from '@/components/shared/model-pricing-badge'
import type { DetailedModelInfo, FreeTierKind } from '@/types/tauri-commands'
import { filterPolicyModels } from './policy-model-catalog'

type ModelRef = [string, string]
export function PolicyModelLabel({ value, models, freeTiers }: { value: ModelRef; models: DetailedModelInfo[]; freeTiers: Record<string, FreeTierKind> }) {
  const model = models.find(m => m.provider_instance === value[0] && m.model_id === value[1])
  return <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 text-xs">
    <span className="truncate" title={value.join(' / ')}>{value.join(' / ')}{model?.display_name && model.display_name !== model.model_id && ` · ${model.display_name}`}{!model && ' (unavailable)'}</span>
    {model && <span title={[model.free_tier_model?.reason, pricingSourceLabel(model.pricing_source), 'Input / output USD per million tokens; free eligibility does not guarantee remaining quota.'].filter(Boolean).join('\n')}>
      <ModelPricingBadge inputPricePerMillion={model.input_price_per_million} outputPricePerMillion={model.output_price_per_million} freeTierKind={freeTiers[value[0]]} freeModel={model.free_tier_model} />
    </span>}
  </span>
}

export function PolicyModelPicker({ models, freeTiers, value, onChange, placeholder, disabled }: {
  models: DetailedModelInfo[]; freeTiers: Record<string, FreeTierKind>; value: ModelRef | null; onChange: (value: ModelRef) => void; placeholder: string; disabled?: boolean
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [provider, setProvider] = useState('')
  const [freeOnly, setFreeOnly] = useState(false)
  const [limit, setLimit] = useState(100)
  const providers = useMemo(() => [...new Set(models.map(m => m.provider_instance))].sort(), [models])
  const matches = useMemo(() => filterPolicyModels(models, query, provider, freeOnly), [models, query, provider, freeOnly])
  const choose = (model: DetailedModelInfo) => { onChange([model.provider_instance, model.model_id]); setOpen(false) }
  return <>
    <Button variant="outline" className="w-full justify-between h-auto min-h-10 text-left" disabled={disabled} aria-label={placeholder} aria-haspopup="dialog" onClick={() => { setQuery(''); setProvider(''); setFreeOnly(false); setLimit(100); setOpen(true) }}>
      {value ? <PolicyModelLabel value={value} models={models} freeTiers={freeTiers} /> : <span>{placeholder}</span>}<Search className="h-4 w-4 ml-2 shrink-0" />
    </Button>
    <Dialog open={open} onOpenChange={setOpen}><DialogContent className="sm:max-w-2xl">
      <DialogHeader><DialogTitle>{placeholder}</DialogTitle><DialogDescription>Search provider and model names. Prices are input / output USD per million tokens. Free tier eligibility does not guarantee remaining quota.</DialogDescription></DialogHeader>
      <div className="flex flex-wrap items-center gap-4 text-sm">
        <label className="flex items-center gap-2">Provider<select className="rounded border bg-background p-1" aria-label="Filter provider" value={provider} onChange={e => { setProvider(e.target.value); setLimit(100) }}><option value="">All providers</option>{providers.map(p => <option key={p} value={p}>{p}</option>)}</select></label>
        <label className="flex items-center gap-2"><input type="checkbox" checked={freeOnly} onChange={e => { setFreeOnly(e.target.checked); setLimit(100) }} />Free eligible only</label>
      </div>
      <Command shouldFilter={false} className="rounded-md border">
        <CommandInput aria-label="Search policy models" placeholder="Search models or providers…" value={query} onValueChange={q => { setQuery(q); setLimit(100) }} />
        <CommandList><CommandEmpty>No matching models.</CommandEmpty>
          {matches.slice(0, limit).map(m => <CommandItem key={JSON.stringify([m.provider_instance, m.model_id])} value={JSON.stringify([m.provider_instance, m.model_id])} onSelect={() => choose(m)} className="py-3"><PolicyModelLabel value={[m.provider_instance, m.model_id]} models={[m]} freeTiers={freeTiers} /></CommandItem>)}
        </CommandList>
      </Command>
      <div className="flex justify-between items-center text-xs text-muted-foreground"><span>{Math.min(limit, matches.length)} of {matches.length} models</span>{matches.length > limit && <Button variant="outline" size="sm" onClick={() => setLimit(n => n + 100)}>Show more</Button>}</div>
    </DialogContent></Dialog>
  </>
}
