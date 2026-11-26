import type { AppState } from './api'

export const STATE_LABEL: Record<AppState, string> = {
  new: 'In attesa',
  up: 'Attiva',
  down: 'Non risponde',
  paused: 'In pausa',
}

export const STATE_COLOR: Record<AppState, string> = {
  new: 'grey',
  up: 'success',
  down: 'error',
  paused: 'warning',
}

export const UNITS = [
  { title: 'secondi', value: 1 },
  { title: 'minuti', value: 60 },
  { title: 'ore', value: 3600 },
  { title: 'giorni', value: 86400 },
]

/** Sceglie l'unità più grande che divide esattamente i secondi. */
export function splitDuration(secs: number): { amount: number; unit: number } {
  const unit = [...UNITS].reverse().find((u) => secs >= u.value && secs % u.value === 0)?.value ?? 1
  return { amount: secs / unit, unit }
}

export function formatDuration(secs: number): string {
  if (secs === 0) return '0'
  const { amount, unit } = splitDuration(secs)
  const labels: Record<number, [string, string]> = {
    1: ['secondo', 'secondi'],
    60: ['minuto', 'minuti'],
    3600: ['ora', 'ore'],
    86400: ['giorno', 'giorni'],
  }
  const [one, many] = labels[unit] ?? ['secondo', 'secondi']
  return `${amount} ${amount === 1 ? one : many}`
}

const relative = new Intl.RelativeTimeFormat('it', { numeric: 'auto' })

export function formatRelative(ms: number | null, now = Date.now()): string {
  if (ms === null) return 'mai'
  const diff = (ms - now) / 1000
  const abs = Math.abs(diff)
  if (abs < 60) return relative.format(Math.round(diff), 'second')
  if (abs < 3600) return relative.format(Math.round(diff / 60), 'minute')
  if (abs < 86400) return relative.format(Math.round(diff / 3600), 'hour')
  return relative.format(Math.round(diff / 86400), 'day')
}

const dateTime = new Intl.DateTimeFormat('it', { dateStyle: 'short', timeStyle: 'short' })

export function formatDateTime(ms: number | null): string {
  return ms === null ? '—' : dateTime.format(ms)
}
