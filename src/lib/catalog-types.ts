// Catalog types for model metadata
// These correspond to the Rust types in src/ui/commands.rs

export interface CatalogMetadata {
  fetch_date: string;
  api_version: string;
  total_models: number;
}

export interface CatalogStats {
  total_models: number;
  fetch_date: string;
  providers: Record<string, number>;
  modalities: Record<string, number>;
}

export interface CatalogInfo {
  pricing_source: 'catalog' | 'provider' | 'fallback';
  catalog_date?: string;
  matched_via?: string;
}
