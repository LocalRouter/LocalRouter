// Research content loaded from markdown files.
// Uses the same <!-- @entry entry-id --> marker system as docs.

const modules = import.meta.glob('./research/content/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const researchContent: Record<string, string> = {}

for (const raw of Object.values(modules)) {
  const parts = raw.split(/<!--\s*@entry\s+([\w-]+)\s*-->/)
  for (let i = 1; i < parts.length; i += 2) {
    const id = parts[i]
    const content = parts[i + 1]?.trim()
    if (id && content) {
      researchContent[id] = content
    }
  }
}

export default researchContent
