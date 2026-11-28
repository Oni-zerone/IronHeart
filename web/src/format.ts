import type { AppState, GroupRole, Role } from './api'

export const STATE_LABEL: Record<AppState, string> = {
  new: 'waiting',
  up: 'up',
  down: 'not responding',
  paused: 'paused',
}

/** Glyph in front of the state: a heart while the app beats. */
export const STATE_GLYPH: Record<AppState, string> = {
  new: '..',
  up: '<3',
  down: '</3',
  paused: '||',
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

/** Elapsed time without direction: "23 minutes". */
export function formatElapsed(ms: number, now = Date.now()): string {
  const secs = Math.max(0, Math.round((now - ms) / 1000))
  const [amount, one, many] =
    secs < 60
      ? [secs, 'second', 'seconds']
      : secs < 3600
        ? [Math.round(secs / 60), 'minute', 'minutes']
        : secs < 86400
          ? [Math.round(secs / 3600), 'hour', 'hours']
          : [Math.round(secs / 86400), 'day', 'days']
  return `${amount} ${amount === 1 ? one : many}`
}

const pad = (n: number) => String(n).padStart(2, '0')

/** Local date and time, "2026-11-27 10:54". */
export function formatDateTime(ms: number | null): string {
  if (ms === null) return '-'
  const d = new Date(ms)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function formatTime(ms: number | null): string {
  if (ms === null) return '-'
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export const ROLE_OPTIONS: { title: string; value: Role }[] = [
  { title: 'user', value: 'user' },
  { title: 'admin', value: 'admin' },
]

export const ROLE_LABEL: Record<Role, string> = { user: 'user', admin: 'admin' }

export const GROUP_ROLE_OPTIONS: { title: string; value: GroupRole }[] = [
  { title: 'member', value: 'member' },
  { title: 'group admin', value: 'admin' },
]

export const GROUP_ROLE_LABEL: Record<GroupRole, string> = { member: 'member', admin: 'group admin' }
