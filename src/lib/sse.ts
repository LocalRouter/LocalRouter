/** Decode SSE data fields across arbitrary UTF-8 chunks and LF/CRLF framing. */
export async function* readSseData(body: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let boundary: RegExpExecArray | null
      while ((boundary = /\r?\n\r?\n/.exec(buffer))) {
        const frame = buffer.slice(0, boundary.index)
        buffer = buffer.slice(boundary.index + boundary[0].length)
        const data = frame.split(/\r?\n/)
          .filter(line => line.startsWith('data:'))
          .map(line => line.slice(5).replace(/^ /, ''))
        if (data.length) yield data.join('\n')
      }
    }
  } finally {
    // Also release the connection when a consumer throws or stops early.
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
