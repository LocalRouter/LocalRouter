import type { DetailedModelInfo } from '@/types/tauri-commands'

export function filterPolicyModels(models: DetailedModelInfo[], query: string, provider: string, freeOnly: boolean): DetailedModelInfo[] {
  const words = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean)
  return models.filter(m => (!provider || m.provider_instance === provider)
    && (!freeOnly || m.free_tier_model?.is_free === true)
    && words.every(word => `${m.provider_instance} ${m.provider_type} ${m.model_id} ${m.display_name ?? ''}`.toLocaleLowerCase().includes(word)))
    .sort((a, b) => a.provider_instance.localeCompare(b.provider_instance) || a.model_id.localeCompare(b.model_id))
}

export function knownPrice(price: number | null | undefined): price is number {
  return price != null && Number.isFinite(price) && price >= 0
}
