import { useEffect, useState, useCallback } from 'react'
import { getHistory, getLiveHistory, getStatus, postAutoToggle, postSwitch, type HistoryPayload, type Range, type StatusResponse } from './api'
import { AutoControl } from './components/AutoControl'
import { NetworkBadge } from './components/NetworkBadge'
import { RangeSelector } from './components/RangeSelector'
import { SensorCard } from './components/SensorCard'
import { Sparkline } from './components/Sparkline'
import { SwitchButton } from './components/SwitchButton'
import { UpsCard } from './components/UpsCard'
import { HISTORY_CAPACITY, pushHistory } from './history'

const SENSOR_LABELS: Record<number, string> = {
  0x40: 'Batterie / Réseau',
  0x41: 'Solaire',
}

const POLL_INTERVAL_MS = 1000
const HISTORY_REFRESH_MS = 60_000

const RANGE_BUCKET_LABELS: Record<Range, string> = {
  hour: '1 min',
  day: '5 min',
  week: '1 h',
  month: '6 h',
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
  const [autoError, setAutoError] = useState<string | null>(null)
  const [autoPending, setAutoPending] = useState(false)
  const [history, setHistory] = useState<History>(EMPTY_HISTORY)
  const [range, setRange] = useState<Range>('hour')
  const [historyData, setHistoryData] = useState<HistoryPayload | null>(null)
  const [historyError, setHistoryError] = useState<string | null>(null)

  // Précharge du buffer live depuis le backend (vit sur la Pi, pas dans la
  // mémoire du navigateur). Les sparklines sont remplies dès le premier rendu.
  useEffect(() => {
    let cancelled = false
    getLiveHistory()
      .then(h => {
        if (cancelled) return
        setHistory({
          sensorVoltage: {
            0x40: h.sensor_grid_v.slice(),
            0x41: h.sensor_solar_v.slice(),
          },
          upsInputV: h.ups_input_v.slice(),
          upsBattV: h.ups_battery_v.slice(),
        })
      })
      .catch(() => {
        // Ce n'est pas fatal : le polling 1s va remplir le buffer client en
        // continuant comme avant. Pas de bandeau d'erreur dédié.
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Live status polling — toutes les 1s, append au ring buffer client (qui a
  // déjà été préchargé depuis le backend ci-dessus).
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
    const id = setInterval(tick, POLL_INTERVAL_MS)
    return () => clearInterval(id)
  }, [])

  // Historique persistant — refetch quand le range change + tail update toutes les 60s.
  useEffect(() => {
    let cancelled = false
    const fetchIt = () => {
      getHistory(range)
        .then(h => {
          if (cancelled) return
          setHistoryData(h)
          setHistoryError(null)
        })
        .catch(e => {
          if (!cancelled) setHistoryError(String(e?.message ?? e))
        })
    }
    fetchIt()
    const id = setInterval(fetchIt, HISTORY_REFRESH_MS)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [range])

  const handleSwitch = useCallback(async () => {
    setSwitchError(null)
    const r = await postSwitch()
    if (!r.ok) {
      const text = await r.text().catch(() => '')
      setSwitchError(text || `Échec : HTTP ${r.status}`)
    }
  }, [])

  const handleAutoToggle = useCallback(async (enabled: boolean) => {
    setAutoError(null)
    setAutoPending(true)
    try {
      const r = await postAutoToggle(enabled)
      if (!r.ok) {
        const text = await r.text().catch(() => '')
        setAutoError(text || `Échec : HTTP ${r.status}`)
      } else {
        // Optimistic update : le prochain getStatus confirmera.
        setStatus(prev => (prev ? { ...prev, auto_enabled: enabled } : prev))
      }
    } catch (e) {
      setAutoError(String((e as Error)?.message ?? e))
    } finally {
      setAutoPending(false)
    }
  }, [])

  const dbDown = status !== null && !status.db_connected
  const windowMinutes = Math.round(HISTORY_CAPACITY / 60)
  const overrideUntilLabel = status?.manual_override_until
    ? new Date(status.manual_override_until).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : null

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1 className="app-title">Solar Controller</h1>
          <div className="app-subtitle">
            Échantillon par seconde · fenêtre live {windowMinutes} min
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
          {status && (
            <AutoControl
              enabled={status.auto_enabled}
              reason={status.auto_reason}
              message={status.auto_message}
              manualOverride={status.manual_override_active}
              pending={autoPending}
              onToggle={handleAutoToggle}
            />
          )}
        </div>
      </header>

      {error && <div className="alert alert--danger">Connexion perdue…</div>}
      {switchError && <div className="alert alert--danger">{switchError}</div>}
      {autoError && <div className="alert alert--danger">Auto : {autoError}</div>}
      {dbDown && (
        <div className="alert alert--warn">
          Base de données injoignable — historisation suspendue. Le contrôleur reste opérationnel.
        </div>
      )}
      {status && !status.auto_enabled && (
        <div className="alert alert--warn">
          Auto-switch désactivé — bascule manuelle uniquement. La règle de sécurité tension reste active.
        </div>
      )}
      {status?.manual_override_active && overrideUntilLabel && (
        <div className="alert alert--warn">
          Override manuel actif jusqu'à {overrideUntilLabel} — l'auto-switch ne décide rien pendant ce temps.
        </div>
      )}

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
                  socPercent={s.address === 0x40 ? status.soc_percent : null}
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

          <section className="history-section">
            <div className="history-section__header">
              <span className="history-section__title">
                Historique · résolution {RANGE_BUCKET_LABELS[range]}
              </span>
              <RangeSelector value={range} onChange={setRange} />
            </div>

            {historyError && !dbDown && (
              <div className="alert alert--warn">Historique indisponible : {historyError}</div>
            )}

            {historyData ? (
              <div className="history-grid">
                <HistoryTile label="Tension réseau (0x40)" values={historyData.sensor_grid_v} />
                <HistoryTile label="Tension solaire (0x41)" values={historyData.sensor_solar_v} accent="var(--solar)" />
                <HistoryTile label="UPS — entrée" values={historyData.ups_input_v} accent="var(--accent)" />
                <HistoryTile label="UPS — batterie" values={historyData.ups_battery_v} accent="var(--ok)" />
                <HistoryTile label="Météo — radiation (W/m²)" values={historyData.weather_radiation} accent="var(--warn)" />
                <HistoryTile label="Météo — couverture nuageuse (%)" values={historyData.weather_cloud_pct} accent="#94a3b8" />
              </div>
            ) : !historyError ? (
              <div className="card dim">Chargement de l'historique…</div>
            ) : null}
          </section>
        </>
      ) : (
        <div className="card dim">Connexion…</div>
      )}
    </div>
  )
}

function HistoryTile({ label, values, accent }: { label: string; values: (number | null)[]; accent?: string }) {
  return (
    <div className="history-tile">
      <span className="history-tile__label">{label}</span>
      <Sparkline values={values} accent={accent} />
    </div>
  )
}
