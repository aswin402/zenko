import type { CitationItem } from '@/components/agents/citations'

const MAX_CITATIONS = 10

function safeHostname(url: string): string | undefined {
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return undefined
    return parsed.hostname.replace(/^www\./, '')
  } catch {
    return undefined
  }
}

function isUrlTerminator(ch: string): boolean {
  return (
    ch === ' ' ||
    ch === '\n' ||
    ch === '\r' ||
    ch === '\t' ||
    ch === ')' ||
    ch === ']' ||
    ch === '"' ||
    ch === "'" ||
    ch === '<' ||
    ch === '>' ||
    ch === '`'
  )
}

function stripTrailingPunctuation(url: string): string {
  let end = url.length
  while (end > 0) {
    const last = url[end - 1]
    if (last === '.' || last === ',' || last === ';' || last === ':' || last === '!' || last === '?') {
      end -= 1
    } else {
      break
    }
  }
  return url.slice(0, end)
}

/**
 * Deterministic zero-regex parser extracting `[Title](https://...)` markdown links
 * and bare `https://...` URLs from assistant prose and tool outputs (e.g. `web_search` / `web_fetch`).
 */
export function extractCitationsFromTurn(
  text: string,
  tools: ReadonlyArray<{ name: string; content: string }>,
): CitationItem[] {
  const seen = new Map<string, CitationItem>()

  const addUrl = (rawUrl: string, rawTitle?: string) => {
    if (seen.size >= MAX_CITATIONS) return
    const cleaned = stripTrailingPunctuation(rawUrl.trim())
    const domain = safeHostname(cleaned)
    if (!domain) return
    if (seen.has(cleaned)) return
    const title = rawTitle && rawTitle.trim().length > 0 ? rawTitle.trim() : domain
    seen.set(cleaned, {
      id: `src-${seen.size + 1}`,
      title,
      domain,
      url: cleaned,
    })
  }

  // 1. Scan markdown links `[label](https://...)` deterministically
  const scanMarkdownLinks = (src: string) => {
    let i = 0
    while (i < src.length && seen.size < MAX_CITATIONS) {
      const openBracket = src.indexOf('[', i)
      if (openBracket < 0) break
      const closeBracket = src.indexOf(']', openBracket + 1)
      if (closeBracket < 0) break
      if (src[closeBracket + 1] === '(') {
        const closeParen = src.indexOf(')', closeBracket + 2)
        if (closeParen > closeBracket + 2) {
          const label = src.slice(openBracket + 1, closeBracket)
          const href = src.slice(closeBracket + 2, closeParen).split(' ')[0] || ''
          if (href.startsWith('http://') || href.startsWith('https://')) {
            addUrl(href, label)
          }
          i = closeParen + 1
          continue
        }
      }
      i = closeBracket + 1
    }
  }

  // 2. Scan bare http(s) URLs from web tools & prose
  const scanBareUrls = (src: string) => {
    let i = 0
    while (i < src.length && seen.size < MAX_CITATIONS) {
      const idxHttps = src.indexOf('https://', i)
      const idxHttp = src.indexOf('http://', i)
      let start = -1
      if (idxHttps >= 0 && idxHttp >= 0) start = Math.min(idxHttps, idxHttp)
      else start = Math.max(idxHttps, idxHttp)
      if (start < 0) break

      let end = start
      while (end < src.length && !isUrlTerminator(src[end])) {
        end += 1
      }
      addUrl(src.slice(start, end))
      i = end + 1
    }
  }

  scanMarkdownLinks(text)
  for (const t of tools) {
    if (
      t.name.includes('web') ||
      t.name.includes('search') ||
      t.name.includes('fetch') ||
      t.name.includes('browse')
    ) {
      scanMarkdownLinks(t.content)
      scanBareUrls(t.content)
    }
  }
  scanBareUrls(text)

  return Array.from(seen.values())
}
