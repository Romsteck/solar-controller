import { SensorReading } from '../api'

interface Props {
  sensor: SensorReading
  label: string
}

const card: React.CSSProperties = {
  background: '#1e293b',
  border: '1px solid #334155',
  borderRadius: '0.75rem',
  padding: '1rem 1.5rem',
  minWidth: '160px',
}

const value: React.CSSProperties = {
  fontSize: '1.5rem',
  fontWeight: 700,
  fontVariantNumeric: 'tabular-nums',
}

export function SensorCard({ sensor, label }: Props) {
  return (
    <div style={card}>
      <div style={{ color: '#94a3b8', marginBottom: '0.5rem', fontSize: '0.85rem' }}>{label}</div>
      <div style={value}>{sensor.bus_voltage_v.toFixed(2)} <span style={{ fontSize: '0.9rem', color: '#94a3b8' }}>V</span></div>
      <div style={{ ...value, marginTop: '0.25rem' }}>{sensor.current_ma.toFixed(1)} <span style={{ fontSize: '0.9rem', color: '#94a3b8' }}>mA</span></div>
    </div>
  )
}
