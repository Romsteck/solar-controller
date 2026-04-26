export interface SensorReading {
  address: number
  bus_voltage_v: number
  current_ma: number
}

export type RelayState = 'open' | 'grid' | 'solar'

export interface UpsReading {
  input_voltage_v: number | null
  input_frequency_hz: number | null
  output_voltage_v: number | null
  load_pct: number | null
  battery_pct: number | null
  battery_voltage_v: number | null
  runtime_s: number | null
  status: string | null
  last_seen: number
}

export interface StatusResponse {
  relay_state: RelayState
  switching: boolean
  sensors: SensorReading[]
  ups: UpsReading | null
}

export async function getStatus(): Promise<StatusResponse> {
  const r = await fetch('/api/status')
  if (!r.ok) throw new Error('status fetch failed')
  return r.json()
}

export async function postSwitch(): Promise<Response> {
  return fetch('/api/switch', { method: 'POST' })
}
