import type { RelayState } from '../api'
import { StatusPill, type Tone } from './StatusPill'

interface Props {
  state: RelayState
}

const LABELS: Record<RelayState, string> = {
  grid: 'Réseau',
  solar: 'Solaire',
  open: 'Sécurité',
}

const TONES: Record<RelayState, Tone> = {
  grid: 'grid',
  solar: 'solar',
  open: 'danger',
}

export function NetworkBadge({ state }: Props) {
  return <StatusPill tone={TONES[state]}>{LABELS[state]}</StatusPill>
}
