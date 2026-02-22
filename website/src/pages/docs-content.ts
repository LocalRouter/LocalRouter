// Documentation content loaded from markdown files.
// Each .md file uses <!-- @entry entry-id --> markers to delimit individual entries.
// Content is rendered as markdown via ReactMarkdown in Docs.tsx.

const modules = import.meta.glob('./docs/content/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const docsContent: Record<string, string> = {}

for (const raw of Object.values(modules)) {
  // Split on <!-- @entry ... --> markers
  const parts = raw.split(/<!--\s*@entry\s+([\w-]+)\s*-->/)
  // parts: ["preamble", "id1", "content1", "id2", "content2", ...]
  for (let i = 1; i < parts.length; i += 2) {
    const id = parts[i]
    const content = parts[i + 1]?.trim()
    if (id && content) {
      docsContent[id] = content
    }
  }
}

export default docsContent
