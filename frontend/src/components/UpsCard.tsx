import { UpsReading } from '../api'
import { Bar } from './Bar'
import { Sparkline } from './Sparkline'
import { StatusPill, type Tone } from './StatusPill'

interface Props {
  ups: UpsReading | null
  inputVoltageHistory: (number | null)[]
  batteryVoltageHistory: (number | null)[]
}

const STALE_AFTER_S = 10

function fmt(v: number | null, unit: string, digits = 1): string {
  return v === null ? '—' : `${v.toFixed(digits)} ${unit}`
}

interface StatusInfo {
  label: string
  tone: Tone
}

function statusInfo(s: string | null): StatusInfo {
  if (!s) return { label: '—', tone: 'muted' }
  const tokens = s.split(/\s+/)
  const map: Record<string, { label: string; tone: Tone }> = {
    OL: { label: 'Secteur', tone: 'ok' },
    OB: { label: 'Batterie', tone: 'warn' },
    LB: { label: 'Batterie faible', tone: 'danger' },
    CHRG: { label: 'En charge', tone: 'accent' },
    DISCHRG: { label: 'Décharge', tone: 'warn' },
    RB: { label: 'Remplacer batt.', tone: 'danger' },
    BYPASS: { label: 'Bypass', tone: 'warn' },
  }

  let tone: Tone = 'muted'
  const labels: string[] = []
  for (const t of tokens) {
    const entry = map[t]
    if (entry) {
      labels.push(entry.label)
      // Le ton le plus alarmant l'emporte (danger > warn > accent > ok > muted).
      const order: Tone[] = ['muted', 'ok', 'accent', 'warn', 'danger']
      if (order.indexOf(entry.tone) > order.indexOf(tone)) tone = entry.tone
    } else {
      labels.push(t)
    }
  }
  return { label: labels.join(' · '), tone }
}

function loadTone(load: number | null): 'accent' | 'warn' | 'danger' {
  if (load === null) return 'accent'
  if (load >= 90) return 'danger'
  if (load >= 70) return 'warn'
  return 'accent'
}

export function UpsCard({ ups, inputVoltageHistory, batteryVoltageHistory }: Props) {
  if (ups === null) {
    return (
      <div className="card">
        <div className="card-header">
          <span className="label">UPS</span>
          <StatusPill tone="muted">Non détecté</StatusPill>
        </div>
        <div className="dim" style={{ fontSize: '0.85rem' }}>Aucune donnée NUT disponible.</div>
      </div>
    )
  }

  const ageS = Math.floor(Date.now() / 1000) - ups.last_seen
  const stale = ageS > STALE_AFTER_S
  const status = statusInfo(ups.status)

  return (
    <div className={`card${stale ? ' section-stale' : ''}`}>
      <div className="card-header">
        <span className="label">UPS</span>
        <StatusPill tone={status.tone}>{status.label}</StatusPill>
      </div>

      <div className="metric-row">
        <div className="metric">
          <span className="metric-label">Entrée</span>
          <span className="metric-value">{fmt(ups.input_voltage_v, 'V')}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Sortie</span>
          <span className="metric-value">{fmt(ups.output_voltage_v, 'V')}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Fréq.</span>
          <span className="metric-value">{fmt(ups.input_frequency_hz, 'Hz')}</span>
        </div>
      </div>

      <div style={{ marginBottom: '0.85rem' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '0.35rem' }}>
          <span className="metric-label">Charge UPS</span>
          <span className="dim" style={{ fontSize: '0.8rem', fontVariantNumeric: 'tabular-nums' }}>
            {fmt(ups.load_pct, '%', 0)}
          </span>
        </div>
        <Bar value={ups.load_pct} tone={loadTone(ups.load_pct)} />
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '0.75rem' }}>
        <div>
          <div className="metric-label" style={{ marginBottom: '0.25rem' }}>
            Entrée — {fmt(ups.input_voltage_v, 'V')}
          </div>
          <Sparkline values={inputVoltageHistory} />
        </div>
        <div>
          <div className="metric-label" style={{ marginBottom: '0.25rem' }}>
            Batterie — {fmt(ups.battery_voltage_v, 'V', 2)}
          </div>
          <Sparkline values={batteryVoltageHistory} accent="var(--ok)" />
        </div>
      </div>

      {stale && (
        <div className="alert alert--warn" style={{ marginTop: '0.75rem', marginBottom: 0 }}>
          Données obsolètes ({ageS}s)
        </div>
      )}
    </div>
  )
}
