import { FileArchive, Folder, Store, User } from "lucide-react"
import type { SkillSourceKind } from "@/types/tauri-commands"

export const SOURCE_KIND_LABEL: Record<SkillSourceKind, string> = {
  user: "Created here",
  marketplace: "Marketplace",
  archive: "Archive",
  directory: "Folder",
}

export function SkillSourceIcon({ kind, className }: { kind: SkillSourceKind | "marketplace_group"; className?: string }) {
  switch (kind) {
    case "user":
      return <User className={className} />
    case "marketplace":
    case "marketplace_group":
      return <Store className={className} />
    case "archive":
      return <FileArchive className={className} />
    default:
      return <Folder className={className} />
  }
}
