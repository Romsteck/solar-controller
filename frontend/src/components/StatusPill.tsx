import type { ReactNode } from 'react'

export type Tone = 'accent' | 'ok' | 'warn' | 'danger' | 'grid' | 'solar' | 'muted'

interface Props {
  tone?: Tone
  children: ReactNode
}

export function StatusPill({ tone = 'muted', children }: Props) {
  return <span className={`pill pill--${tone}`}>{children}</span>
}
