import { useEffect, useState } from 'react'
import { Layers, Send, User } from 'lucide-react'
import { api } from '@/lib/api'
import type { GatewayPeer, PairingCode } from '@/lib/types'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { InputField, ScrollPage, type SecondaryTabsVariant } from './shared'

// ─── Gateways View ────────────────────────────────────────────────────────────

export function GatewaysView({
  toast,
  tabsVariant = 'pill',
  smoothScroll = true,
}: {
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
  tabsVariant?: SecondaryTabsVariant
  smoothScroll?: boolean
}) {
  const [gwTab, setGwTab] = useState<'all' | 'setup' | 'peers'>('all')
  const [data, setData] = useState<{
    config: any
    connected: boolean
    peers: GatewayPeer[]
    pairing_codes: PairingCode[]
  } | null>(null)
  const [token, setToken] = useState('')
  const [role, setRole] = useState('owner')

  const load = async () => {
    try {
      setData(await api('/api/gateways/telegram'))
    } catch (e: any) {
      toast(e.message, 'error')
    }
  }
  useEffect(() => { load() }, [])

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[860px] flex-col gap-6">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
              Telegram Gateway
            </h1>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Zero-trust pairing codes, Forum Topic session isolation, live draft streaming, and inline keyboard approvals.
            </p>
          </div>
          <span
            className="inline-flex items-center gap-1.5 rounded-full border bg-[var(--surface)] px-3 py-1 font-mono text-[11.5px]"
            style={{
              color: data?.connected ? 'var(--fg)' : 'var(--muted)',
              borderColor: data?.connected
                ? 'color-mix(in srgb, var(--accent) 42%, var(--border))'
                : 'var(--border)',
            }}
          >
            {data?.connected ? `Connected @${data?.config?.bot_username || 'bot'}` : 'Offline'}
          </span>
        </div>

        <Tabs
          value={gwTab}
          onValueChange={(v) => setGwTab(v as 'all' | 'setup' | 'peers')}
          variant={tabsVariant}
        >
          <TabsList>
            <TabsTrigger value="all" icon={<Layers size={13} />}>Overview</TabsTrigger>
            <TabsTrigger value="setup" icon={<Send size={13} />}>Connection &amp; Pairing</TabsTrigger>
            <TabsTrigger value="peers" icon={<User size={13} />}>Paired Accounts ({data?.peers.length || 0})</TabsTrigger>
          </TabsList>
        </Tabs>

        {gwTab !== 'peers' && (
        <div className="grid gap-5 md:grid-cols-2">
          <div className="z-surface-card flex flex-col gap-3">
            <strong className="text-[14px]">Bot Connection</strong>
            <InputField label="New @BotFather Token (optional)" type="password" value={token} onChange={setToken} placeholder="Leave blank to keep stored token" mono />
            <label className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                checked={Boolean(data?.config?.enabled)}
                onChange={(e) => setData((d) => (d ? { ...d, config: { ...d.config, enabled: e.target.checked } } : d))}
                className="accent-[var(--accent)]"
              />
              <span>Gateway Enabled</span>
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                checked={Boolean(data?.config?.stream_drafts)}
                onChange={(e) => setData((d) => (d ? { ...d, config: { ...d.config, stream_drafts: e.target.checked } } : d))}
                className="accent-[var(--accent)]"
              />
              <span>Live Token Draft Streaming</span>
            </label>
            <button
              type="button"
              className="z-btn z-btn-primary"
              onClick={async () => {
                try {
                  await api('/api/gateways/telegram', {
                    method: 'POST',
                    body: JSON.stringify({
                      token: token.trim() || null,
                      enabled: Boolean(data?.config?.enabled),
                      stream_drafts: Boolean(data?.config?.stream_drafts),
                    }),
                  })
                  setToken('')
                  toast('Telegram settings updated', 'success')
                  load()
                } catch (e: any) {
                  toast(e.message, 'error')
                }
              }}
            >
              Save Gateway
            </button>
          </div>

          <div className="z-surface-card flex flex-col gap-3">
            <strong className="text-[14px]">One-Time Pairing Code</strong>
            <p className="text-[12.5px] text-[var(--muted)]">
              Generate a 6-character code, then message <code className="font-mono">/pair CODE</code> to your bot.
            </p>
            <div className="flex gap-2">
              <div className="min-w-0 flex-1">
                <ZenkoSelect
                  ariaLabel="Telegram Pairing Role"
                  value={role}
                  onChange={setRole}
                  options={[
                    { value: 'owner', label: 'Owner (Full + Approvals)' },
                    { value: 'trusted', label: 'Trusted' },
                    { value: 'readonly', label: 'Read-Only' },
                  ]}
                />
              </div>
              <button
                type="button"
                className="z-btn z-btn-primary shrink-0"
                onClick={async () => {
                  const res = await api('/api/gateways/telegram/pair', {
                    method: 'POST',
                    body: JSON.stringify({ role }),
                  })
                  toast(`Pairing code: ${res.code}`, 'success')
                  load()
                }}
              >
                Generate
              </button>
            </div>
            {(data?.pairing_codes || []).map((c) => (
              <div
                key={c.code}
                className="flex items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[12px]"
              >
                <span>
                  /pair <strong style={{ color: 'var(--accent)' }}>{c.code}</strong>
                </span>
                <span className="text-[var(--muted)]">{c.role}</span>
              </div>
            ))}
          </div>
        </div>
        )}

        {gwTab !== 'setup' && (
        <div className="z-surface-card flex flex-col gap-3">
          <strong className="text-[14px]">Paired Accounts ({data?.peers.length || 0})</strong>
          {(data?.peers || []).length === 0 ? (
            <p className="text-[13px] text-[var(--muted)]">No paired Telegram users yet.</p>
          ) : (
            data?.peers.map((p) => (
              <div
                key={p.peer_id}
                className="flex items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3.5 py-2.5 text-[13px]"
              >
                <div>
                  <span className="font-medium">{p.display_name || 'User'}</span>
                  {p.username && <span className="ml-1.5 text-[var(--muted)]">@{p.username}</span>}
                  <span className="ml-2 font-mono text-[11px] text-[var(--muted)]">#{p.peer_id}</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="rounded-full border border-[var(--border)] px-2 py-0.5 font-mono text-[11px]">
                    {p.role}
                  </span>
                  <button
                    type="button"
                    className="z-btn z-btn-danger z-btn-sm"
                    onClick={async () => {
                      await api(`/api/gateways/telegram/peers/${encodeURIComponent(p.peer_id)}`, { method: 'DELETE' })
                      load()
                    }}
                  >
                    Revoke
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
        )}
      </div>
    </ScrollPage>
  )
}

