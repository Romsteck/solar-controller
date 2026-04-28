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
  db_connected: boolean
  auto_enabled: boolean
  auto_reason: string | null
  auto_message: string | null
  soc_percent: number | null
  float_reached_today: boolean
  eod_lockout: boolean
  eod_at: string | null
  eod_threshold_v: number | null
}

export type Range = 'hour' | 'day' | 'week' | 'month'

export interface HistoryPayload {
  range: Range
  bucket: string
  ts: number[]
  sensor_grid_v: (number | null)[]
  sensor_grid_ma: (number | null)[]
  sensor_solar_v: (number | null)[]
  sensor_solar_ma: (number | null)[]
  ups_input_v: (number | null)[]
  ups_battery_v: (number | null)[]
  weather_temp_c: (number | null)[]
  weather_cloud_pct: (number | null)[]
  weather_radiation: (number | null)[]
}

export async function getStatus(): Promise<StatusResponse> {
  const r = await fetch('/api/status')
  if (!r.ok) throw new Error('status fetch failed')
  return r.json()
}

export async function postSwitch(): Promise<Response> {
  return fetch('/api/switch', { method: 'POST' })
}

export async function postAutoToggle(enabled: boolean): Promise<Response> {
  return fetch('/api/auto', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ enabled }),
  })
}

export async function getHistory(range: Range): Promise<HistoryPayload> {
  const r = await fetch(`/api/history?range=${range}`)
  if (!r.ok) throw new Error(`history fetch failed: ${r.status}`)
  return r.json()
}

export interface LiveHistoryResponse {
  capacity: number
  ts: number[]
  sensor_grid_v: (number | null)[]
  sensor_solar_v: (number | null)[]
  ups_input_v: (number | null)[]
  ups_battery_v: (number | null)[]
}

export async function getLiveHistory(): Promise<LiveHistoryResponse> {
  const r = await fetch('/api/live-history')
  if (!r.ok) throw new Error(`live-history fetch failed: ${r.status}`)
  return r.json()
}
