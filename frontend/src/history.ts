// Buffer in-memory pour les sparklines temps réel.
// 300 points × 1s = fenêtre 5 minutes au rafraîchissement seconde.
export const HISTORY_CAPACITY = 300

export function pushHistory<T>(arr: T[], value: T, capacity = HISTORY_CAPACITY): T[] {
  const next = arr.length >= capacity ? arr.slice(arr.length - capacity + 1) : arr.slice()
  next.push(value)
  return next
}
