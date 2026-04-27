import { useEffect, useState, useCallback } from 'react'
import { getStatus, postSwitch, StatusResponse } from './api'
import { NetworkBadge } from './components/NetworkBadge'
import { SensorCard } from './components/SensorCard'
import { SwitchButton } from './components/SwitchButton'
import { UpsCard } from './components/UpsCard'
import { HISTORY_CAPACITY, pushHistory } from './history'

const SENSOR_LABELS: Record<number, string> = {
  0x40: 'Batterie / Réseau',
  0x41: 'Solaire',
}

interface History {
  sensorVoltage: Record<number, (number | null)[]>
  upsInputV: (number | null)[]
  upsBattV: (number | null)[]
}

const EMPTY_HISTORY: History = {
  sensorVoltage: {},
  upsInputV: [],
  upsBattV: [],
}

export default function App() {
  const [status, setStatus] = useState<StatusResponse | null>(null)
  const [error, setError] = useState(false)
  const [switchError, setSwitchError] = useState<string | null>(null)
  const [history, setHistory] = useState<History>(EMPTY_HISTORY)

  useEffect(() => {
    const tick = () => {
      getStatus()
        .then(s => {
          setStatus(s)
          setError(false)
          setHistory(prev => {
            const sensorVoltage: Record<number, (number | null)[]> = { ...prev.sensorVoltage }
            for (const sensor of s.sensors) {
              sensorVoltage[sensor.address] = pushHistory(prev.sensorVoltage[sensor.address] ?? [], sensor.bus_voltage_v)
            }
            return {
              sensorVoltage,
              upsInputV: pushHistory(prev.upsInputV, s.ups?.input_voltage_v ?? null),
              upsBattV: pushHistory(prev.upsBattV, s.ups?.battery_voltage_v ?? null),
            }
          })
        })
        .catch(() => setError(true))
    }
    tick()
    const id = setInterval(tick, 1000)
    return () => clearInterval(id)
  }, [])

  const handleSwitch = useCallback(async () => {
    setSwitchError(null)
    const r = await postSwitch()
    if (!r.ok) {
      const text = await r.text().catch(() => '')
      setSwitchError(text || `Échec : HTTP ${r.status}`)
    }
  }, [])

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1 className="app-title">Solar Controller</h1>
          <div className="app-subtitle">
            Échantillon par seconde · fenêtre {HISTORY_CAPACITY}s
          </div>
        </div>
        <div className="app-actions">
          {status && <NetworkBadge state={status.relay_state} />}
          {status && (
            <SwitchButton
              state={status.relay_state}
              switching={status.switching}
              onSwitch={handleSwitch}
            />
          )}
        </div>
      </header>

      {error && <div className="alert alert--danger">Connexion perdue…</div>}
      {switchError && <div className="alert alert--danger">{switchError}</div>}

      {status?.relay_state === 'open' && !status.switching && (
        <div className="alert alert--danger">
          État SÉCURITÉ — les deux relais sont ouverts. Aucune source active.
        </div>
      )}

      {status ? (
        <>
          <div className="grid grid-2" style={{ marginBottom: '1rem' }}>
            {status.sensors.length > 0 ? (
              status.sensors.map(s => (
                <SensorCard
                  key={s.address}
                  sensor={s}
                  label={SENSOR_LABELS[s.address] ?? `Capteur 0x${s.address.toString(16)}`}
                  voltageHistory={history.sensorVoltage[s.address] ?? []}
                />
              ))
            ) : (
              <div className="card dim">Aucune lecture capteur</div>
            )}
          </div>

          <UpsCard
            ups={status.ups}
            inputVoltageHistory={history.upsInputV}
            batteryVoltageHistory={history.upsBattV}
          />
        </>
      ) : (
        <div className="card dim">Connexion…</div>
      )}
    </div>
  )
}
