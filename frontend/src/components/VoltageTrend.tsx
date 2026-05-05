import { useMemo } from 'react'

interface Props {
  values: (number | null)[]
  /** Largeur du lissage (samples = secondes ici) appliquée aux deux bouts. */
  smoothWindowSec?: number
  /** Distance temporelle entre le past et le now (samples = secondes). */
  diffWindowSec?: number
  /** Δ V à partir duquel la jauge sature. */
  saturationV?: number
  /** |Δ| sous lequel l'état est considéré stable. */
  stableThreshold?: number
}

function meanLastN(
  values: (number | null)[],
  endIdx: number,
  count: number,
): number | null {
  if (endIdx < 0) return null
  const lo = Math.max(0, endIdx - count + 1)
  let sum = 0
  let n = 0
  for (let i = lo; i <= endIdx; i++) {
    const v = values[i]
    if (v != null && Number.isFinite(v)) {
      sum += v
      n++
    }
  }
  return n > 0 ? sum / n : null
}

export function VoltageTrend({
  values,
  smoothWindowSec = 5,
  diffWindowSec = 50,
  saturationV = 0.2,
  stableThreshold = 0.02,
}: Props) {
  const computed = useMemo(() => {
    const lastIdx = values.length - 1
    const now = meanLastN(values, lastIdx, smoothWindowSec)
    const past = meanLastN(values, lastIdx - diffWindowSec, smoothWindowSec)
    if (now === null || past === null) return null
    return { delta: now - past }
  }, [values, smoothWindowSec, diffWindowSec])

  if (!computed) return null

  const { delta } = computed
  const stable = Math.abs(delta) < stableThreshold
  const direction = stable ? 'stable' : delta > 0 ? 'rising' : 'falling'

  // Géométrie SVG : ruban vertical, ligne zéro au milieu, fill qui pousse
  // depuis le zéro vers le haut (rising) ou le bas (falling).
  const W = 14
  const H = 56
  const PAD = 3
  const center = H / 2
  const usable = H / 2 - PAD
  const norm = Math.max(-1, Math.min(1, delta / saturationV))
  const fillH = Math.max(2, Math.abs(norm) * usable)
  const fillY = delta >= 0 ? center - fillH : center

  const sign = delta > 0 ? '+' : delta < 0 ? '−' : ''
  const label = stable ? 'stable' : `${sign}${Math.abs(delta).toFixed(2)} V`

  return (
    <span
      className={`trend trend--${direction}`}
      title={`Variation lissée ${smoothWindowSec}s, comparée à il y a ${diffWindowSec}s`}
    >
      <svg
        className="trend__gauge"
        width={W}
        height={H}
        viewBox={`0 0 ${W} ${H}`}
        aria-hidden="true"
      >
        <rect
          className="trend__track"
          x={W / 2 - 2}
          y={PAD}
          width={4}
          height={H - PAD * 2}
          rx={2}
        />
        <line
          className="trend__zero"
          x1={1}
          y1={center}
          x2={W - 1}
          y2={center}
        />
        {!stable && (
          <rect
            className="trend__fill"
            x={W / 2 - 2}
            y={fillY}
            width={4}
            height={fillH}
            rx={2}
          />
        )}
      </svg>
      <span className="trend__value">{label}</span>
    </span>
  )
}
