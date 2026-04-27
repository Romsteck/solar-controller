import { SensorReading } from '../api'
import { Sparkline } from './Sparkline'

interface Props {
  sensor: SensorReading
  label: string
  voltageHistory: (number | null)[]
}

export function SensorCard({ sensor, label, voltageHistory }: Props) {
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

      <Sparkline values={voltageHistory} />
    </div>
  )
}
