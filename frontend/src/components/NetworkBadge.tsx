import type { RelayState } from '../api'

interface Props {
  state: RelayState
}

const styles: Record<string, React.CSSProperties> = {
  badge: {
    display: 'inline-block',
    padding: '0.4rem 1.2rem',
    borderRadius: '999px',
    fontWeight: 700,
    fontSize: '1rem',
    letterSpacing: '0.05em',
    textTransform: 'uppercase',
  },
  grid: { background: '#1e40af', color: '#bfdbfe' },
  solar: { background: '#166534', color: '#bbf7d0' },
  open: { background: '#7f1d1d', color: '#fecaca' },
}

const LABELS: Record<RelayState, string> = {
  grid: 'Réseau GRID',
  solar: 'Réseau Solaire',
  open: 'Sécurité — tous relais ouverts',
}

export function NetworkBadge({ state }: Props) {
  return (
    <span style={{ ...styles.badge, ...styles[state] }}>
      {LABELS[state]}
    </span>
  )
}
