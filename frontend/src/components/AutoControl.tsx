import { StatusPill, type Tone } from './StatusPill'

interface Props {
  enabled: boolean
  reason: string | null
  message: string | null
  pending: boolean
  onToggle: (next: boolean) => void
  eodAt: string | null
  eodThresholdV: number | null
  eodLockout: boolean
  floatReachedToday: boolean
}

const REASON_TONE: Record<string, Tone> = {
  emergency_low_voltage: 'danger',
  voltage_low_sustained: 'warn',
  eod_recharge: 'accent',
  voltage_recovered: 'ok',
  auto_disabled: 'muted',
  anti_oscillation: 'muted',
  hold: 'muted',
}

function formatTime(iso: string | null): string | null {
  if (!iso) return null
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return null
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export function AutoControl({
  enabled,
  reason,
  message,
  pending,
  onToggle,
  eodAt,
  eodThresholdV,
  eodLockout,
  floatReachedToday,
}: Props) {
  const tone: Tone = enabled
    ? REASON_TONE[reason ?? 'hold'] ?? 'accent'
    : 'muted'

  const label = enabled ? 'Auto ON' : 'Auto OFF'
  const action = enabled ? 'Désactiver' : 'Activer'

  const eodTimeLabel = formatTime(eodAt)
  const now = new Date()
  const eodPassed = eodAt ? new Date(eodAt).getTime() <= now.getTime() : false

  let eodLine: string | null = null
  if (eodLockout) {
    eodLine = 'EOD déclenché — lockout jusqu’au prochain lever'
  } else if (eodTimeLabel && eodThresholdV != null) {
    const passed = eodPassed ? ' (passé)' : ''
    const floatBadge = floatReachedToday ? '' : ' (+0.2V pénalité, Float pas atteint)'
    eodLine = `EOD à ${eodTimeLabel}${passed} · seuil ${eodThresholdV.toFixed(1)} V${floatBadge}`
  } else if (eodTimeLabel) {
    eodLine = `EOD à ${eodTimeLabel}`
  }

  return (
    <div className="auto-control">
      <div className="auto-control__row">
        <StatusPill tone={tone}>{label}</StatusPill>
        <button
          className="btn"
          onClick={() => onToggle(!enabled)}
          disabled={pending}
        >
          {pending ? '…' : action}
        </button>
      </div>
      {message && (
        <div className="auto-control__reason" title={reason ?? undefined}>
          {message}
        </div>
      )}
      {eodLine && <div className="auto-control__eod">{eodLine}</div>}
    </div>
  )
}
