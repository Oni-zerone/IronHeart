import type { AppState, Role } from './api'

export const STATE_LABEL: Record<AppState, string> = {
  new: 'Waiting',
  up: 'Up',
  down: 'Down',
  paused: 'Paused',
}

export const STATE_COLOR: Record<AppState, string> = {
  new: 'grey',
  up: 'success',
  down: 'error',
  paused: 'warning',
}

export const UNITS = [
  { title: 'seconds', value: 1 },
  { title: 'minutes', value: 60 },
  { title: 'hours', value: 3600 },
  { title: 'days', value: 86400 },
]

/** Picks the largest unit that divides the seconds exactly. */
export function splitDuration(secs: number): { amount: number; unit: number } {
  const unit = [...UNITS].reverse().find((u) => secs >= u.value && secs % u.value === 0)?.value ?? 1
  return { amount: secs / unit, unit }
}

export function formatDuration(secs: number): string {
  if (secs === 0) return '0'
  const { amount, unit } = splitDuration(secs)
  const labels: Record<number, [string, string]> = {
    1: ['second', 'seconds'],
    60: ['minute', 'minutes'],
    3600: ['hour', 'hours'],
    86400: ['day', 'days'],
  }
  const [one, many] = labels[unit] ?? ['second', 'seconds']
  return `${amount} ${amount === 1 ? one : many}`
}

const relative = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })

export function formatRelative(ms: number | null, now = Date.now()): string {
  if (ms === null) return 'never'
  const diff = (ms - now) / 1000
  const abs = Math.abs(diff)
  if (abs < 60) return relative.format(Math.round(diff), 'second')
  if (abs < 3600) return relative.format(Math.round(diff / 60), 'minute')
  if (abs < 86400) return relative.format(Math.round(diff / 3600), 'hour')
  return relative.format(Math.round(diff / 86400), 'day')
}

const dateTime = new Intl.DateTimeFormat('en', { dateStyle: 'short', timeStyle: 'short' })

export function formatDateTime(ms: number | null): string {
  return ms === null ? '—' : dateTime.format(ms)
}

export const ROLE_OPTIONS: { title: string; value: Role }[] = [
  { title: 'User', value: 'user' },
  { title: 'Administrator', value: 'admin' },
]

export const ROLE_LABEL: Record<Role, string> = { user: 'User', admin: 'Administrator' }
