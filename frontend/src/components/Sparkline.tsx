import { useId } from 'react'

interface Props {
  values: (number | null)[]
  accent?: string
  fill?: boolean
}

export function Sparkline({ values, accent = 'var(--accent)', fill = true }: Props) {
  const gradId = useId()
  const numeric: number[] = values.filter((v): v is number => v !== null && Number.isFinite(v))

  if (numeric.length < 2) {
    return <div className="sparkline__placeholder">— pas encore d'historique</div>
  }

  const min = Math.min(...numeric)
  const max = Math.max(...numeric)
  const range = max - min || 1
  const W = 100
  const H = 100

  const stepX = W / (values.length - 1)
  const points = values.map((v, i) => {
    const x = i * stepX
    if (v === null || !Number.isFinite(v)) return null
    const y = H - ((v - min) / range) * H
    return { x, y }
  })

  const segments: string[] = []
  let current: string[] = []
  for (const p of points) {
    if (p === null) {
      if (current.length) {
        segments.push(current.join(' '))
        current = []
      }
    } else {
      current.push(`${current.length === 0 ? 'M' : 'L'} ${p.x.toFixed(2)} ${p.y.toFixed(2)}`)
    }
  }
  if (current.length) segments.push(current.join(' '))
  const linePath = segments.join(' ')

  const lastValid = [...points].reverse().find(p => p !== null) ?? null
  const firstValid = points.find(p => p !== null) ?? null

  const areaPath = fill && firstValid && lastValid
    ? `${linePath} L ${lastValid.x.toFixed(2)} ${H} L ${firstValid.x.toFixed(2)} ${H} Z`
    : null

  return (
    <svg className="sparkline" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      {fill && areaPath && (
        <>
          <defs>
            <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={accent} stopOpacity="0.35" />
              <stop offset="100%" stopColor={accent} stopOpacity="0" />
            </linearGradient>
          </defs>
          <path d={areaPath} fill={`url(#${gradId})`} stroke="none" />
        </>
      )}
      <path d={linePath} fill="none" stroke={accent} strokeWidth="1.5" vectorEffect="non-scaling-stroke" strokeLinecap="round" strokeLinejoin="round" />
      {lastValid && (
        <circle cx={lastValid.x} cy={lastValid.y} r="1.6" fill={accent} vectorEffect="non-scaling-stroke" />
      )}
    </svg>
  )
}
