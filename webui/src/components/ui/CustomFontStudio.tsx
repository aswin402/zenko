import { useRef, useState } from 'react'
import { RotateCcw, Search, Type, Upload, X } from 'lucide-react'
import {
  MONO_FONT_PRESETS,
  UI_FONT_PRESETS,
  clearSavedCustomFontFile,
  queryInstalledSystemFonts,
  registerCustomFontFile,
  sanitizeFontFamilyName,
} from '@/lib/fonts'
import type { UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'

/**
 * Global Theme Custom Font Studio — pick curated UI & Code presets, type any OS-installed font,
 * browse local system fonts (when supported), or upload custom `.woff2` / `.ttf` / `.otf` files.
 */
export function CustomFontStudio({
  typography,
  onPatchTypography,
}: {
  typography: UiCustomization['typography']
  onPatchTypography: (mutator: (draft: UiCustomization['typography']) => void) => void
}) {
  const uiFileRef = useRef<HTMLInputElement>(null)
  const monoFileRef = useRef<HTMLInputElement>(null)
  const [uiInput, setUiInput] = useState(typography.custom_ui_font || '')
  const [monoInput, setMonoInput] = useState(typography.custom_mono_font || '')
  const [fontMessage, setFontMessage] = useState('')
  const [systemFonts, setSystemFonts] = useState<string[]>([])
  const [systemFontFilter, setSystemFontFilter] = useState('')
  const [systemPickerTarget, setSystemPickerTarget] = useState<'ui' | 'mono' | null>(null)
  const [loadingSystemFonts, setLoadingSystemFonts] = useState(false)

  const applyCustomName = (slot: 'ui' | 'mono', raw: string) => {
    const clean = sanitizeFontFamilyName(raw)
    if (slot === 'ui') {
      setUiInput(clean)
      onPatchTypography((t) => {
        t.custom_ui_font = clean
      })
    } else {
      setMonoInput(clean)
      onPatchTypography((t) => {
        t.custom_mono_font = clean
      })
    }
    setFontMessage(clean ? `Applied custom ${slot.toUpperCase()} font: ${clean}` : '')
  }

  const handleUpload = async (slot: 'ui' | 'mono', file: File | undefined) => {
    if (!file) return
    try {
      const family = await registerCustomFontFile(slot, file)
      applyCustomName(slot, family)
      setFontMessage(`Loaded & saved "${family}" (${file.name})`)
    } catch (err: any) {
      setFontMessage(err?.message || 'Could not load font file')
    }
  }

  const openSystemFonts = async (target: 'ui' | 'mono') => {
    if (systemPickerTarget === target) {
      setSystemPickerTarget(null)
      return
    }
    setSystemPickerTarget(target)
    if (systemFonts.length > 0) return
    setLoadingSystemFonts(true)
    try {
      const list = await queryInstalledSystemFonts()
      setSystemFonts(list)
      if (list.length === 0) {
        setFontMessage('Browser local font picker unavailable — type any installed font name or upload .woff2/.ttf/.otf.')
      }
    } catch {
      setFontMessage('Local font access declined — type any installed font name or upload a font file.')
    } finally {
      setLoadingSystemFonts(false)
    }
  }

  const filteredSystemFonts = systemFontFilter.trim()
    ? systemFonts.filter((f) => f.toLowerCase().includes(systemFontFilter.trim().toLowerCase()))
    : systemFonts

  return (
    <div className="flex flex-col gap-5 rounded-2xl border border-[var(--border)] bg-[var(--surface)]/70 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 text-[15px] font-semibold text-[var(--fg)]">
            <Type size={15} style={{ color: 'var(--accent)' }} />
            <span>Custom Font Studio</span>
          </div>
          <p className="mt-0.5 text-[12px] text-[var(--muted)]">
            Choose a curated typeface, enter any installed system font, or upload your own .woff2 / .ttf / .otf file.
          </p>
        </div>
        <button
          type="button"
          onClick={() => {
            clearSavedCustomFontFile('ui')
            clearSavedCustomFontFile('mono')
            setUiInput('')
            setMonoInput('')
            setFontMessage('')
            onPatchTypography((t) => {
              t.ui_font = 'inter'
              t.mono_font = 'jetbrains'
              t.custom_ui_font = ''
              t.custom_mono_font = ''
            })
          }}
          className="z-btn z-btn-secondary z-btn-sm"
        >
          <RotateCcw size={12} /> Restore Default Fonts
        </button>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        {/* Interface Font Column */}
        <div className="flex flex-col gap-3 rounded-xl border border-[var(--border)] bg-[var(--card)]/70 p-3.5">
          <div className="flex items-center justify-between gap-2">
            <div>
              <div className="text-[12.5px] font-medium text-[var(--fg)]">Interface Font</div>
              <div className="text-[10.5px] text-[var(--muted)]">
                Headings, chat prose, buttons, and menus
              </div>
            </div>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={() => openSystemFonts('ui')}
                className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
              >
                OS Fonts
              </button>
              <button
                type="button"
                onClick={() => uiFileRef.current?.click()}
                className="z-btn z-btn-secondary z-btn-sm font-mono text-[10.5px]"
              >
                <Upload size={11} /> Upload
              </button>
              <input
                ref={uiFileRef}
                type="file"
                accept=".woff2,.woff,.ttf,.otf"
                className="hidden"
                onChange={(e) => {
                  handleUpload('ui', e.target.files?.[0])
                  e.target.value = ''
                }}
              />
            </div>
          </div>

          <div className="flex flex-wrap gap-1.5">
            {UI_FONT_PRESETS.map((preset) => {
              const active = !typography.custom_ui_font && typography.ui_font === preset.id
              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => {
                    setUiInput('')
                    onPatchTypography((t) => {
                      t.ui_font = preset.id
                      t.custom_ui_font = ''
                    })
                  }}
                  style={{ fontFamily: preset.stack }}
                  className={cn(
                    'cursor-pointer rounded-lg border px-2.5 py-1 text-[11.5px] transition-colors',
                    active
                      ? 'border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_12%,var(--bg))] text-[var(--fg)]'
                      : 'border-[var(--border)] bg-[var(--bg)] text-[var(--muted)] hover:text-[var(--fg)]',
                  )}
                >
                  {preset.label}
                </button>
              )
            })}
          </div>

          <div className="flex items-center gap-1.5">
            <input
              value={uiInput}
              onChange={(e) => setUiInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') applyCustomName('ui', uiInput)
              }}
              placeholder="Custom UI font family (e.g. Geist, Poppins, SF Pro)..."
              className="min-w-0 flex-1 rounded-lg border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5 text-[12px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:border-[var(--accent)] focus:outline-none"
            />
            {uiInput && (
              <button
                type="button"
                onClick={() => applyCustomName('ui', '')}
                aria-label="Clear custom UI font"
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--muted)] hover:text-[var(--fg)]"
              >
                <X size={12} />
              </button>
            )}
            <button
              type="button"
              onClick={() => applyCustomName('ui', uiInput)}
              className="z-btn z-btn-primary z-btn-sm"
            >
              Apply
            </button>
          </div>
        </div>

        {/* Code / Monospace Font Column */}
        <div className="flex flex-col gap-3 rounded-xl border border-[var(--border)] bg-[var(--card)]/70 p-3.5">
          <div className="flex items-center justify-between gap-2">
            <div>
              <div className="text-[12.5px] font-medium text-[var(--fg)]">Code &amp; Terminal Font</div>
              <div className="text-[10.5px] text-[var(--muted)]">
                Code blocks, diffs, terminal output, and badges
              </div>
            </div>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={() => openSystemFonts('mono')}
                className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
              >
                OS Fonts
              </button>
              <button
                type="button"
                onClick={() => monoFileRef.current?.click()}
                className="z-btn z-btn-secondary z-btn-sm font-mono text-[10.5px]"
              >
                <Upload size={11} /> Upload
              </button>
              <input
                ref={monoFileRef}
                type="file"
                accept=".woff2,.woff,.ttf,.otf"
                className="hidden"
                onChange={(e) => {
                  handleUpload('mono', e.target.files?.[0])
                  e.target.value = ''
                }}
              />
            </div>
          </div>

          <div className="flex flex-wrap gap-1.5">
            {MONO_FONT_PRESETS.map((preset) => {
              const active = !typography.custom_mono_font && typography.mono_font === preset.id
              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => {
                    setMonoInput('')
                    onPatchTypography((t) => {
                      t.mono_font = preset.id
                      t.custom_mono_font = ''
                    })
                  }}
                  style={{ fontFamily: preset.stack }}
                  className={cn(
                    'cursor-pointer rounded-lg border px-2.5 py-1 text-[11.5px] transition-colors',
                    active
                      ? 'border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_12%,var(--bg))] text-[var(--fg)]'
                      : 'border-[var(--border)] bg-[var(--bg)] text-[var(--muted)] hover:text-[var(--fg)]',
                  )}
                >
                  {preset.label}
                </button>
              )
            })}
          </div>

          <div className="flex items-center gap-1.5">
            <input
              value={monoInput}
              onChange={(e) => setMonoInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') applyCustomName('mono', monoInput)
              }}
              placeholder="Custom mono font (e.g. Iosevka, Geist Mono, Hack)..."
              className="min-w-0 flex-1 rounded-lg border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5 font-mono text-[11.5px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:border-[var(--accent)] focus:outline-none"
            />
            {monoInput && (
              <button
                type="button"
                onClick={() => applyCustomName('mono', '')}
                aria-label="Clear custom mono font"
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--muted)] hover:text-[var(--fg)]"
              >
                <X size={12} />
              </button>
            )}
            <button
              type="button"
              onClick={() => applyCustomName('mono', monoInput)}
              className="z-btn z-btn-primary z-btn-sm"
            >
              Apply
            </button>
          </div>
        </div>
      </div>

      {/* Local OS Font Picker Drawer (when browser supports window.queryLocalFonts) */}
      {systemPickerTarget && (
        <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)] p-3">
          <div className="flex items-center justify-between gap-2">
            <div className="relative flex-1">
              <Search size={12} className="absolute left-2.5 top-2 text-[var(--muted)]" />
              <input
                value={systemFontFilter}
                onChange={(e) => setSystemFontFilter(e.target.value)}
                placeholder={`Filter installed OS fonts for ${systemPickerTarget.toUpperCase()}...`}
                className="w-full rounded-lg border border-[var(--border)] bg-[var(--surface)] py-1 pl-7 pr-2.5 text-[11.5px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none"
              />
            </div>
            <button
              type="button"
              onClick={() => setSystemPickerTarget(null)}
              className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
            >
              <X size={13} />
            </button>
          </div>
          {loadingSystemFonts ? (
            <div className="py-2 font-mono text-[11px] text-[var(--muted)]">
              Reading installed system fonts...
            </div>
          ) : filteredSystemFonts.length === 0 ? (
            <div className="py-2 font-mono text-[11px] text-[var(--muted)]">
              No local system fonts listed — type a font name above or upload a .woff2 / .ttf / .otf file.
            </div>
          ) : (
            <CompactScrollArea autoHeight viewportClassName="max-h-36 pr-2.5">
              <div className="flex flex-wrap gap-1.5">
                {filteredSystemFonts.slice(0, 80).map((fam) => (
                  <button
                    key={fam}
                    type="button"
                    onClick={() => {
                      applyCustomName(systemPickerTarget, fam)
                      setSystemPickerTarget(null)
                    }}
                    style={{ fontFamily: `'${fam}', sans-serif` }}
                    className="cursor-pointer rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 py-1 text-[11.5px] text-[var(--fg)] hover:border-[var(--accent)]"
                  >
                    {fam}
                  </button>
                ))}
              </div>
            </CompactScrollArea>
          )}
        </div>
      )}

      {/* Live Font Specimen Bar */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border)]/70 bg-[var(--bg)] px-3.5 py-2.5">
        <div className="text-[13px] text-[var(--fg)]">
          Zenko builds fast, local-first Rust agents.
        </div>
        <code className="font-mono text-[11.5px]" style={{ color: 'var(--accent)' }}>
          {typography.custom_ui_font || typography.ui_font} · {typography.custom_mono_font || typography.mono_font}
        </code>
      </div>

      {fontMessage && (
        <div className="font-mono text-[11px] text-[var(--muted)]">{fontMessage}</div>
      )}
    </div>
  )
}
