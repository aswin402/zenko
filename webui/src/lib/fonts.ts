export interface FontPreset {
  id: string
  label: string
  hint: string
  stack: string
}

export const UI_FONT_PRESETS: readonly FontPreset[] = [
  {
    id: 'inter',
    label: 'Plus Jakarta Sans',
    hint: 'Clean editorial geometric sans',
    stack: "'Plus Jakarta Sans', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'geist',
    label: 'Space Grotesk',
    hint: 'Sharp technical display sans',
    stack: "'Space Grotesk', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'obel_mono',
    label: 'Obel Mono',
    hint: 'Typographic Systems · Variable editorial mono',
    stack: "'Obel Mono', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'halden',
    label: 'Halden',
    hint: 'Fontzz · Neo-grotesque sans',
    stack: "'Halden', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'rondel',
    label: 'Rondel',
    hint: 'Fontzz · Precision geometric sans',
    stack: "'Rondel', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'mallow',
    label: 'Mallow',
    hint: 'Fontzz · Friendly rounded sans',
    stack: "'Mallow', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'strata',
    label: 'Strata',
    hint: 'Fontzz · Condensed architectural sans',
    stack: "'Strata', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'vector',
    label: 'Vector',
    hint: 'Fontzz · Crisp technical sans',
    stack: "'Vector', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'folio',
    label: 'Folio',
    hint: 'Fontzz · Editorial book serif',
    stack: "'Folio', ui-serif, Georgia, Cambria, serif",
  },
  {
    id: 'meridian',
    label: 'Meridian',
    hint: 'Fontzz · Transitional serif',
    stack: "'Meridian', ui-serif, Georgia, Cambria, serif",
  },
  {
    id: 'ledger',
    label: 'Ledger',
    hint: 'Fontzz · Confident slab serif',
    stack: "'Ledger', ui-serif, Georgia, Cambria, serif",
  },
  {
    id: 'opaline',
    label: 'Opaline',
    hint: 'Fontzz · High-contrast Didone serif',
    stack: "'Opaline', ui-serif, Georgia, Cambria, serif",
  },
  {
    id: 'vesper',
    label: 'Vesper',
    hint: 'Fontzz · Sculptural display',
    stack: "'Vesper', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'orbit',
    label: 'Orbit',
    hint: 'Fontzz · Futuristic display',
    stack: "'Orbit', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'diode',
    label: 'Diode',
    hint: 'Fontzz · Modular geometric display',
    stack: "'Diode', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
  },
  {
    id: 'system',
    label: 'System UI',
    hint: 'Native OS interface font',
    stack: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  },
  {
    id: 'mono',
    label: 'Monospace UI',
    hint: 'Obel Mono terminal-first font',
    stack: "'Obel Mono', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
] as const

export const MONO_FONT_PRESETS: readonly FontPreset[] = [
  {
    id: 'obel_mono',
    label: 'Obel Mono',
    hint: 'Typographic Systems · Variable code mono',
    stack: "'Obel Mono', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'jetbrains',
    label: 'JetBrains Mono',
    hint: 'Default developer mono',
    stack: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'signal',
    label: 'Signal',
    hint: 'Fontzz · Crisp engineering mono',
    stack: "'Signal', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'tally',
    label: 'Tally',
    hint: 'Fontzz · Clean tabular telemetry mono',
    stack: "'Tally', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'fathom',
    label: 'Fathom',
    hint: 'Fontzz · Warm readable code mono',
    stack: "'Fathom', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
  },
  {
    id: 'fira',
    label: 'Fira Code',
    hint: 'Classic ligature mono',
    stack: "'Fira Code', 'JetBrains Mono', ui-monospace, monospace",
  },
  {
    id: 'cascadia',
    label: 'Cascadia Code',
    hint: 'Clean wide monospace',
    stack: "'Cascadia Code', 'JetBrains Mono', ui-monospace, monospace",
  },
  {
    id: 'system',
    label: 'System Mono',
    hint: 'Native OS monospace',
    stack: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
  },
] as const

const UI_FONT_MAP: Record<string, string> = Object.fromEntries(
  UI_FONT_PRESETS.map((f) => [f.id, f.stack]),
)
const MONO_FONT_MAP: Record<string, string> = Object.fromEntries(
  MONO_FONT_PRESETS.map((f) => [f.id, f.stack]),
)

export const CUSTOM_FONT_STORAGE_KEY = 'zenko.custom_fonts.v1'
export const MAX_CUSTOM_FONT_BYTES = 3 * 1024 * 1024

interface SavedFontPayload {
  family: string
  dataUrl: string
}

type SavedFontMap = Partial<Record<'ui' | 'mono', SavedFontPayload>>

const loadedFontKeys = new Set<string>()

/** Safe CSS font family quoting using single-pass character scan (zero regex). */
export function sanitizeFontFamilyName(raw: string): string {
  let out = ''
  for (let i = 0; i < raw.length; i += 1) {
    const ch = raw[i]
    const code = raw.charCodeAt(i)
    const allowed =
      (code >= 48 && code <= 57) || // 0-9
      (code >= 65 && code <= 90) || // A-Z
      (code >= 97 && code <= 122) || // a-z
      ch === ' ' ||
      ch === '-' ||
      ch === '_'
    if (allowed) out += ch
  }
  return out.trim().slice(0, 64)
}

export function resolveUiFontStack(presetId: string, customFamily?: string): string {
  const base = UI_FONT_MAP[presetId] || UI_FONT_MAP.inter
  const clean = sanitizeFontFamilyName(customFamily || '')
  return clean ? `'${clean}', ${base}` : base
}

export function resolveMonoFontStack(presetId: string, customFamily?: string): string {
  const base = MONO_FONT_MAP[presetId] || MONO_FONT_MAP.jetbrains
  const clean = sanitizeFontFamilyName(customFamily || '')
  return clean ? `'${clean}', ${base}` : base
}

function readSavedFonts(): SavedFontMap {
  try {
    const raw = window.localStorage.getItem(CUSTOM_FONT_STORAGE_KEY)
    return raw ? (JSON.parse(raw) as SavedFontMap) : {}
  } catch {
    return {}
  }
}

function writeSavedFonts(map: SavedFontMap) {
  try {
    window.localStorage.setItem(CUSTOM_FONT_STORAGE_KEY, JSON.stringify(map))
  } catch {
    /* ignore quota errors */
  }
}

async function injectFontFace(family: string, dataUrl: string): Promise<void> {
  const key = `${family}::${dataUrl.length}`
  if (loadedFontKeys.has(key) || typeof FontFace === 'undefined') return
  const face = new FontFace(family, `url(${dataUrl})`)
  await face.load()
  document.fonts.add(face)
  loadedFontKeys.add(key)
}

export async function hydrateSavedCustomFonts(): Promise<void> {
  if (typeof window === 'undefined') return
  const saved = readSavedFonts()
  for (const slot of ['ui', 'mono'] as const) {
    const entry = saved[slot]
    if (entry?.family && entry?.dataUrl) {
      try {
        await injectFontFace(entry.family, entry.dataUrl)
      } catch {
        /* ignore corrupted cached font */
      }
    }
  }
}

function deriveFontNameFromFilename(filename: string): string {
  const dot = filename.lastIndexOf('.')
  const stem = dot > 0 ? filename.slice(0, dot) : filename
  return sanitizeFontFamilyName(stem.split('_').join(' ')) || 'Zenko Custom Font'
}

export async function registerCustomFontFile(
  slot: 'ui' | 'mono',
  file: File,
): Promise<string> {
  if (file.size > MAX_CUSTOM_FONT_BYTES) {
    throw new Error('Font file must be under 3 MB (.woff2, .ttf, or .otf)')
  }
  const family = deriveFontNameFromFilename(file.name)
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result || ''))
    reader.onerror = () => reject(new Error('Could not read font file'))
    reader.readAsDataURL(file)
  })
  await injectFontFace(family, dataUrl)
  const saved = readSavedFonts()
  saved[slot] = { family, dataUrl }
  writeSavedFonts(saved)
  return family
}

export function clearSavedCustomFontFile(slot: 'ui' | 'mono') {
  const saved = readSavedFonts()
  delete saved[slot]
  writeSavedFonts(saved)
}

export async function queryInstalledSystemFonts(): Promise<string[]> {
  const api = (window as any).queryLocalFonts as
    | (() => Promise<Array<{ family: string }>>)
    | undefined
  if (typeof api !== 'function') return []
  const list = await api()
  const unique = new Set<string>()
  for (const item of list) {
    const clean = sanitizeFontFamilyName(item.family || '')
    if (clean) unique.add(clean)
  }
  return Array.from(unique).sort((a, b) => a.localeCompare(b))
}
