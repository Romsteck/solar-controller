import { SensorReading } from '../api'
import { Bar } from './Bar'
import { Sparkline } from './Sparkline'

interface Props {
  sensor: SensorReading
  label: string
  voltageHistory: (number | null)[]
  socPercent?: number | null
}

function socTone(soc: number): 'danger' | 'warn' | 'ok' {
  if (soc < 50) return 'danger'
  if (soc < 70) return 'warn'
  return 'ok'
}

export function SensorCard({ sensor, label, voltageHistory, socPercent }: Props) {
  const showSoc = sensor.address === 0x40 && socPercent != null && Number.isFinite(socPercent)

  return (
    <div className="card">
      <div className="card-header">
        <span className="label">{label}</span>
        <span className="dim" style={{ fontSize: '0.78rem', fontVariantNumeric: 'tabular-nums' }}>
          0x{sensor.address.toString(16).padStart(2, '0')}
        </span>
      </div>

      <div style={{ marginBottom: '0.85rem' }}>
        <span className="metric-label">Tension</span>
        <div className="metric-value" style={{ fontSize: '2rem', marginTop: '0.15rem' }}>
          {sensor.bus_voltage_v.toFixed(2)}<span className="metric-unit">V</span>
        </div>
      </div>

      {showSoc && (
        <div style={{ marginBottom: '0.85rem' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '0.35rem' }}>
            <span className="metric-label">Charge batterie (estimée)</span>
            <span className="dim" style={{ fontSize: '0.8rem', fontVariantNumeric: 'tabular-nums' }}>
              ≈ {Math.round(socPercent as number)} %
            </span>
          </div>
          <Bar value={socPercent as number} tone={socTone(socPercent as number)} />
        </div>
      )}

      <Sparkline values={voltageHistory} />
    </div>
  )
}
