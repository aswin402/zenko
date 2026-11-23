/** BeautifulUI-inspired compact circular context/token ProgressRing for the composer. */
export function ProgressRing({
  value,
  max,
  size = 15,
}: {
  value: number
  max: number
  size?: number
}) {
  const pct = Math.min(1, Math.max(0.03, max > 0 ? value / max : 0.03))
  const stroke = 2
  const r = (size - stroke) / 2
  const circ = 2 * Math.PI * r
  const offset = circ * (1 - pct)

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="-rotate-90 shrink-0">
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="var(--border)"
        strokeWidth={stroke}
      />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={pct > 0.92 ? 'var(--destructive)' : 'var(--accent)'}
        strokeWidth={stroke}
        strokeDasharray={circ}
        strokeDashoffset={offset}
        strokeLinecap="round"
      />
    </svg>
  )
}
