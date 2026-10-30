/** "3 min ago" / "in 25 min" for an RFC 3339 timestamp. */
export function formatRelativeTime(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const diff = Math.round((t - now) / 1000)
  const abs = Math.abs(diff)
  const [value, unit] =
    abs < 60 ? [abs, "s"]
    : abs < 3600 ? [Math.round(abs / 60), "min"]
    : abs < 86400 ? [Math.round(abs / 3600), "h"]
    : [Math.round(abs / 86400), "d"]
  if (abs < 5) return "just now"
  return diff < 0 ? `${value} ${unit} ago` : `in ${value} ${unit}`
}
