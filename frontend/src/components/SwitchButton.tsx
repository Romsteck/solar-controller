import type { RelayState } from '../api'

interface Props {
  state: RelayState
  switching: boolean
  onSwitch: () => void
}

const TARGET_LABEL: Record<RelayState, string> = {
  grid: 'Passer en Solaire',
  solar: 'Passer en GRID',
  open: 'Passer en GRID',
}

export function SwitchButton({ state, switching, onSwitch }: Props) {
  const label = switching ? 'Basculement…' : TARGET_LABEL[state]

  const style: React.CSSProperties = {
    padding: '0.75rem 2rem',
    fontSize: '1rem',
    fontWeight: 600,
    borderRadius: '0.5rem',
    border: 'none',
    cursor: switching ? 'not-allowed' : 'pointer',
    background: switching ? '#475569' : state === 'grid' ? '#16a34a' : '#2563eb',
    color: '#fff',
    transition: 'background 0.2s',
    opacity: switching ? 0.7 : 1,
  }

  return (
    <button style={style} onClick={onSwitch} disabled={switching}>
      {label}
    </button>
  )
}
