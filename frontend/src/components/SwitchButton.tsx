import type { RelayState } from '../api'

interface Props {
  state: RelayState
  switching: boolean
  onSwitch: () => void
}

const TARGET_LABEL: Record<RelayState, string> = {
  grid: 'Passer en Solaire',
  solar: 'Passer en Réseau',
  open: 'Passer en Réseau',
}

export function SwitchButton({ state, switching, onSwitch }: Props) {
  const label = switching ? 'Basculement…' : TARGET_LABEL[state]

  return (
    <button className="btn btn--primary" onClick={onSwitch} disabled={switching}>
      {label}
    </button>
  )
}
