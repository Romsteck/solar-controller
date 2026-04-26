import { useEffect, useState, useCallback } from 'react'
import { getStatus, postSwitch, StatusResponse } from './api'
import { NetworkBadge } from './components/NetworkBadge'
import { SensorCard } from './components/SensorCard'
import { SwitchButton } from './components/SwitchButton'
import { UpsCard } from './components/UpsCard'

const SENSOR_LABELS: Record<number, string> = {
  0x40: 'Capteur 0x40',
  0x41: 'Capteur 0x41',
}

export default function App() {
  const [status, setStatus] = useState<StatusResponse | null>(null)
  const [error, setError] = useState(false)
  const [switchError, setSwitchError] = useState<string | null>(null)

  useEffect(() => {
    const tick = () => getStatus().then(s => { setStatus(s); setError(false) }).catch(() => setError(true))
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
    <div style={{ maxWidth: '480px', width: '100%' }}>
      <h1 style={{ fontSize: '1.5rem', fontWeight: 700, marginBottom: '1.5rem', color: '#f1f5f9' }}>
        Solar Controller
      </h1>

      {error && (
        <div style={{ background: '#7f1d1d', color: '#fca5a5', padding: '0.75rem 1rem', borderRadius: '0.5rem', marginBottom: '1rem' }}>
          Connexion perdue…
        </div>
      )}

      {switchError && (
        <div style={{ background: '#7f1d1d', color: '#fca5a5', padding: '0.75rem 1rem', borderRadius: '0.5rem', marginBottom: '1rem' }}>
          {switchError}
        </div>
      )}

      {status?.relay_state === 'open' && !status.switching && (
        <div style={{ background: '#7f1d1d', color: '#fecaca', padding: '0.75rem 1rem', borderRadius: '0.5rem', marginBottom: '1rem', border: '2px solid #fca5a5' }}>
          ⚠ État SÉCURITÉ : les deux relais sont ouverts. Aucune source active.
        </div>
      )}

      {status ? (
        <>
          <div style={{ marginBottom: '1.5rem' }}>
            <NetworkBadge state={status.relay_state} />
          </div>

          <div style={{ marginBottom: '1.5rem' }}>
            <SwitchButton
              state={status.relay_state}
              switching={status.switching}
              onSwitch={handleSwitch}
            />
          </div>

          <UpsCard ups={status.ups} />

          {status.sensors.length > 0 ? (
            <div style={{ display: 'flex', gap: '1rem', flexWrap: 'wrap' }}>
              {status.sensors.map(s => (
                <SensorCard
                  key={s.address}
                  sensor={s}
                  label={SENSOR_LABELS[s.address] ?? `0x${s.address.toString(16)}`}
                />
              ))}
            </div>
          ) : (
            <p style={{ color: '#64748b', fontSize: '0.9rem' }}>Aucune lecture capteur</p>
          )}
        </>
      ) : (
        <p style={{ color: '#64748b' }}>Connexion…</p>
      )}
    </div>
  )
}
