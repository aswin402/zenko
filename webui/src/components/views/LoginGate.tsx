import { useState } from 'react'
import { api } from '@/lib/api'
import { ZenkoFoxLogo } from '@/components/beui/ZenkoFoxLogo'

// ─── Minimal Token Login Gate ─────────────────────────────────────────────────

export function LoginGate({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [token, setToken] = useState('')
  const [error, setError] = useState('')

  return (
    <div className="flex h-full w-full items-center justify-center bg-[var(--bg)] p-6">
      <form
        onSubmit={async (e) => {
          e.preventDefault()
          setError('')
          try {
            await api('/api/login', { method: 'POST', body: JSON.stringify({ token }) })
            onLoggedIn()
          } catch (err: any) {
            setError(err.message)
          }
        }}
        className="z-surface-card flex w-full max-w-[360px] flex-col gap-4 p-6"
      >
        <div className="flex items-center gap-2.5">
          <ZenkoFoxLogo size={26} />
          <h1 className="text-[18px] font-semibold" style={{ fontFamily: 'var(--font-display)' }}>
            Zenko Control Plane
          </h1>
        </div>
        <p className="text-[12.5px] text-[var(--muted)]">
          Enter the admin token printed by <code className="font-mono">zenko setup</code> or{' '}
          <code className="font-mono">zenko web</code>.
        </p>
        {error && (
          <div className="rounded-lg border border-[var(--destructive)]/40 bg-[var(--destructive)]/10 px-3 py-1.5 text-[12px] text-[var(--destructive)]">
            {error}
          </div>
        )}
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="Admin token..."
          autoFocus
          required
          className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[13px] focus:border-[var(--accent)] focus:outline-none"
        />
        <button type="submit" className="z-btn z-btn-primary w-full py-2.5">
          Unlock Zenko
        </button>
      </form>
    </div>
  )
}
