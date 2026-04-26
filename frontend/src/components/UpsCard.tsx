import { UpsReading } from '../api'

interface Props {
  ups: UpsReading | null
}

const card: React.CSSProperties = {
  background: '#1e293b',
  border: '1px solid #334155',
  borderRadius: '0.75rem',
  padding: '1rem 1.5rem',
  marginBottom: '1.5rem',
}

const grid: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(2, 1fr)',
  gap: '0.5rem 1rem',
  fontVariantNumeric: 'tabular-nums',
  fontSize: '0.95rem',
}

const STALE_AFTER_S = 10

function fmt(v: number | null, unit: string, digits = 1): string {
  return v === null ? '—' : `${v.toFixed(digits)} ${unit}`
}

function statusLabel(s: string | null): string {
  if (!s) return '—'
  // Codes NUT standards : OL=on line, OB=on battery, LB=low battery, CHRG=charging, DISCHRG, RB=replace battery
  const tokens = s.split(/\s+/)
  const map: Record<string, string> = {
    OL: 'Secteur',
    OB: 'Batterie',
    LB: 'Batterie faible',
    CHRG: 'En charge',
    DISCHRG: 'Décharge',
    RB: 'Remplacer batt.',
    BYPASS: 'Bypass',
  }
  return tokens.map(t => map[t] ?? t).join(' · ')
}

export function UpsCard({ ups }: Props) {
  if (ups === null) {
    return (
      <div style={{ ...card, color: '#64748b' }}>
        <div style={{ color: '#94a3b8', marginBottom: '0.5rem', fontSize: '0.85rem' }}>UPS</div>
        <div>UPS non détecté</div>
      </div>
    )
  }

  const ageS = Math.floor(Date.now() / 1000) - ups.last_seen
  const stale = ageS > STALE_AFTER_S

  return (
    <div style={{ ...card, opacity: stale ? 0.5 : 1 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: '0.75rem' }}>
        <span style={{ color: '#94a3b8', fontSize: '0.85rem' }}>UPS</span>
        <span style={{ color: '#cbd5e1', fontWeight: 600 }}>{statusLabel(ups.status)}</span>
      </div>
      <div style={grid}>
        <div><span style={{ color: '#94a3b8' }}>Entrée</span> {fmt(ups.input_voltage_v, 'V')}</div>
        <div><span style={{ color: '#94a3b8' }}>Fréq</span> {fmt(ups.input_frequency_hz, 'Hz')}</div>
        <div><span style={{ color: '#94a3b8' }}>Sortie</span> {fmt(ups.output_voltage_v, 'V')}</div>
        <div><span style={{ color: '#94a3b8' }}>Charge</span> {fmt(ups.load_pct, '%', 0)}</div>
        <div><span style={{ color: '#94a3b8' }}>Batt. V</span> {fmt(ups.battery_voltage_v, 'V', 2)}</div>
        {ups.battery_pct !== null && (
          <div><span style={{ color: '#94a3b8' }}>Batt. %</span> {fmt(ups.battery_pct, '%', 0)}</div>
        )}
      </div>
      {stale && (
        <div style={{ marginTop: '0.5rem', fontSize: '0.8rem', color: '#fbbf24' }}>
          Données obsolètes ({ageS}s)
        </div>
      )}
    </div>
  )
}
