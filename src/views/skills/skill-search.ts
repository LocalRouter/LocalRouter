import type { SkillInfo } from "@/types/tauri-commands"

/** Whether a skill matches a lowercase search query (name, description, tags) */
export function matchesSkill(skill: SkillInfo, query: string): boolean {
  if (!query) return true
  return (
    skill.name.toLowerCase().includes(query) ||
    (skill.description?.toLowerCase().includes(query) ?? false) ||
    skill.tags.some((t) => t.toLowerCase().includes(query))
  )
}
