export const HISTORY_CAPACITY = 120

export function pushHistory(arr: (number | null)[], value: number | null, capacity = HISTORY_CAPACITY): (number | null)[] {
  const next = arr.length >= capacity ? arr.slice(arr.length - capacity + 1) : arr.slice()
  next.push(value)
  return next
}
