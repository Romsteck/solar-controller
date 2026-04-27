import { StatusPill, type Tone } from './StatusPill'

interface Props {
  enabled: boolean
  reason: string | null
  message: string | null
  pending: boolean
  onToggle: (next: boolean) => void
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

export function AutoControl({ enabled, reason, message, pending, onToggle }: Props) {
  const tone: Tone = enabled
    ? REASON_TONE[reason ?? 'hold'] ?? 'accent'
    : 'muted'

  const label = enabled ? 'Auto ON' : 'Auto OFF'
  const action = enabled ? 'Désactiver' : 'Activer'

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
    </div>
  )
}
