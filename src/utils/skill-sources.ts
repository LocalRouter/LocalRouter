import type {
  PermissionState,
  SkillInfo,
  SkillSourceInfo,
  SkillsPermissions,
} from "@/types/tauri-commands"

export interface SkillSourceGroup<T extends Pick<SkillInfo, "name" | "source_path">> {
  source: SkillSourceInfo
  skills: T[]
}

/** Last path component, used when the backend has no entry for a source */
function fallbackLabel(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path
}

/**
 * Group skills under their sources, in source order, skills sorted by name.
 * Sources without any of the given skills are left out unless `includeEmpty`.
 */
export function groupSkillsBySource<T extends Pick<SkillInfo, "name" | "source_path">>(
  skills: T[],
  sources: SkillSourceInfo[],
  includeEmpty = false,
): SkillSourceGroup<T>[] {
  const bySource = new Map<string, T[]>()
  for (const skill of skills) {
    const list = bySource.get(skill.source_path) ?? []
    list.push(skill)
    bySource.set(skill.source_path, list)
  }

  const known = new Set(sources.map((s) => s.path))
  const all: SkillSourceInfo[] = [...sources]
  for (const path of bySource.keys()) {
    if (!known.has(path)) {
      all.push({
        path,
        label: fallbackLabel(path),
        kind: "directory",
        exists: true,
        skill_count: bySource.get(path)!.length,
      })
    }
  }

  return all
    .map((source) => ({
      source,
      skills: (bySource.get(source.path) ?? []).sort((a, b) =>
        a.name.toLowerCase().localeCompare(b.name.toLowerCase())
      ),
    }))
    .filter((group) => includeEmpty || group.skills.length > 0)
}

/** Effective permission of a skill: skill -> source -> global */
export function resolveSkillPermission(
  permissions: SkillsPermissions | undefined,
  skill: Pick<SkillInfo, "name" | "source_path">,
): PermissionState {
  return (
    permissions?.skills?.[skill.name] ??
    permissions?.sources?.[skill.source_path] ??
    permissions?.global ??
    "off"
  )
}

/**
 * Marketplace installs are configured one source per skill, so listing each as
 * its own source adds a level with a single child. Split them out so views can
 * show them as one "Marketplace" group of skills.
 */
export function splitMarketplaceGroups<T extends Pick<SkillInfo, "name" | "source_path">>(
  groups: SkillSourceGroup<T>[],
): { sourceGroups: SkillSourceGroup<T>[]; marketplaceSkills: T[] } {
  return {
    sourceGroups: groups.filter((g) => g.source.kind !== "marketplace"),
    marketplaceSkills: groups
      .filter((g) => g.source.kind === "marketplace")
      .flatMap((g) => g.skills)
      .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase())),
  }
}
