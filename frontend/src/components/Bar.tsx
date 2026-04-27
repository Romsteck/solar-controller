type Tone = 'accent' | 'ok' | 'warn' | 'danger'

interface Props {
  value: number | null
  max?: number
  tone?: Tone
}

export function Bar({ value, max = 100, tone = 'accent' }: Props) {
  const pct = value === null || !Number.isFinite(value)
    ? 0
    : Math.max(0, Math.min(100, (value / max) * 100))

  return (
    <div className="bar" role="progressbar" aria-valuemin={0} aria-valuemax={max} aria-valuenow={value ?? undefined}>
      <div className={`bar__fill bar__fill--${tone}`} style={{ width: `${pct}%` }} />
    </div>
  )
}
