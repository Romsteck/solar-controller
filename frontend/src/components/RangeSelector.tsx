import type { Range } from '../api'

const OPTIONS: { value: Range; label: string }[] = [
  { value: 'hour', label: 'Heure' },
  { value: 'day', label: 'Jour' },
  { value: 'week', label: 'Semaine' },
  { value: 'month', label: 'Mois' },
]

interface Props {
  value: Range
  onChange: (r: Range) => void
}

export function RangeSelector({ value, onChange }: Props) {
  return (
    <div className="range-selector" role="tablist" aria-label="Plage temporelle">
      {OPTIONS.map(opt => {
        const active = opt.value === value
        return (
          <button
            key={opt.value}
            type="button"
            role="tab"
            aria-selected={active}
            className={`range-selector__btn${active ? ' range-selector__btn--active' : ''}`}
            onClick={() => onChange(opt.value)}
          >
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
