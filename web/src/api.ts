// Backend calls. The session cookie is sent automatically (same origin).

export type Role = 'admin' | 'user'

export interface User {
  id: string
  email: string
  name: string | null
  role: Role
  /** Master user (ADMIN_EMAIL): always an admin. */
  is_master: boolean
  /** Master only: the database the instance runs on. */
  database?: DatabaseInfo
  /** Master only, when the email transport has a key: last check (startup, then daily). */
  mail_key?: MailKeyStatus
  /** Present only while the instance's Telegram bot is active (valid token). */
  telegram?: { bot_username: string }
  /** Alert channels of the instance and their state. */
  alerts: AlertChannels
}

/**
 * `unreachable`: network error for less than an hour. `unverified`: SMTP without
 * login, nothing to check. `broken`: key refused, or unreachable for over an hour.
 */
export type ChannelHealth = 'checking' | 'ok' | 'unreachable' | 'unverified' | 'broken'

export interface AlertChannels {
  mode: 'mail+telegram' | 'mail' | 'telegram'
  /** Absent when email is off (`MAIL_TRANSPORT=none`). */
  mail?: ChannelHealth
  /** Absent without a Telegram token. */
  telegram?: ChannelHealth
  telegram_linked: boolean
  /** Master only: no channel works, the entanglement pings are held. */
  held?: boolean
}

export interface MailKeyStatus {
  /** Provider checked: Brevo, Resend, Postmark, Mailgun, Scaleway or SMTP. */
  provider: string
  state: 'ok' | 'rejected' | 'unreachable'
  /** Unix milliseconds. */
  checked_at: number
  error?: string
}

export interface DatabaseInfo {
  backend: 'postgres' | 'sqlite'
  /** `host:port/database` for Postgres, file path for SQLite. */
  location: string
}

export interface UserSummary {
  id: string
  email: string
  name: string | null
  role: Role
  created_at: number
  app_count: number
  is_master: boolean
}

export interface Invite {
  email: string
  role: Role
  created_at: number
}

export interface UsersOverview {
  users: UserSummary[]
  invites: Invite[]
}

export type AppState = 'new' | 'up' | 'down' | 'paused'

export interface App {
  id: string
  name: string
  key_id: string
  interval_secs: number
  grace_secs: number
  state: AppState
  last_seen_at: number | null
  next_deadline_at: number | null
  down_since: number | null
  created_at: number
  updated_at: number
  /** Configured recipients; empty = the account email. */
  recipients: string[]
  failures_24h: number
}

export interface FailureBucket {
  period_start: number
  flushed_at: number
  bad_signature: number
  replay: number
  clock_skew: number
  ips: Record<string, number>
  other_ips: number
}

export interface AppDetail extends App {
  failures: FailureBucket[]
}

export interface Credentials {
  key_id: string
  secret: string
}

export interface AppInput {
  name: string
  interval_secs: number
  grace_secs: number
  recipients: string[]
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: init?.body ? { 'content-type': 'application/json', ...init.headers } : init?.headers,
  })
  if (!response.ok) {
    let message = `Error ${response.status}`
    try {
      const body = (await response.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {
      // response without JSON
    }
    throw new ApiError(response.status, message)
  }
  return (response.status === 204 ? undefined : await response.json()) as T
}

const json = (method: string, body: unknown): RequestInit => ({ method, body: JSON.stringify(body) })

/** The signed-in user, or `null` without a session. */
export async function getMe(): Promise<User | null> {
  try {
    return await request<User>('/api/me')
  } catch (err) {
    if (err instanceof ApiError && err.status === 401) return null
    throw err
  }
}

export const logout = () => request<void>('/auth/logout', { method: 'POST' })
export const sendTestEmail = () => request<void>('/api/me/test-email', { method: 'POST' })

export const listApps = () => request<App[]>('/api/apps')
export const getApp = (id: string) => request<AppDetail>(`/api/apps/${id}`)
export const createApp = (input: AppInput) =>
  request<{ app: AppDetail; credentials: Credentials }>('/api/apps', json('POST', input))
export const updateApp = (id: string, input: Partial<AppInput> & { paused?: boolean }) =>
  request<AppDetail>(`/api/apps/${id}`, json('PATCH', input))
export const rotateKey = (id: string) =>
  request<{ app: AppDetail; credentials: Credentials }>(`/api/apps/${id}/rotate-key`, { method: 'POST' })
export const deleteApp = (id: string) => request<void>(`/api/apps/${id}`, { method: 'DELETE' })

export const listUsers = () => request<UsersOverview>('/api/admin/users')
export const inviteUser = (email: string, role: Role) =>
  request<UsersOverview>('/api/admin/users', json('POST', { email, role }))
export const setUserRole = (id: string, role: Role) =>
  request<UsersOverview>(`/api/admin/users/${id}`, json('PATCH', { role }))
export const removeUser = (id: string) =>
  request<UsersOverview>(`/api/admin/users/${id}`, { method: 'DELETE' })
export const revokeInvite = (email: string) =>
  request<UsersOverview>(`/api/admin/invites/${encodeURIComponent(email)}`, { method: 'DELETE' })

export interface Entanglement {
  id: string
  name: string
  base_url: string
  key_id: string
  interval_secs: number
  enabled: boolean
  next_ping_at: number | null
  last_attempt_at: number | null
  /** Last delivered ping (204): only the other instance knows whether it was valid. */
  last_success_at: number | null
  last_error: string | null
  created_at: number
}

export interface EntanglementInput {
  name: string
  base_url: string
  interval_secs: number
  enabled: boolean
  key_id?: string
  secret?: string
}

export const listEntanglements = () =>
  request<{ instance_url: string; items: Entanglement[] }>('/api/entanglements')
export const createEntanglement = (input: EntanglementInput) =>
  request<Entanglement>('/api/entanglements', json('POST', input))
export const updateEntanglement = (id: string, input: Partial<EntanglementInput>) =>
  request<Entanglement>(`/api/entanglements/${id}`, json('PATCH', input))
export const deleteEntanglement = (id: string) =>
  request<void>(`/api/entanglements/${id}`, { method: 'DELETE' })
export const pingEntanglement = (id: string) =>
  request<Entanglement>(`/api/entanglements/${id}/ping`, { method: 'POST' })

export interface TelegramLink {
  id: string
  /** "@username" or first name. */
  label: string
  created_at: number
  /** Telegram refused delivery (bot blocked): no alerts until the chat writes again. */
  disabled_at: number | null
}

export interface TelegramOverview {
  bot_username: string
  bot_url: string
  links: TelegramLink[]
}

export const getTelegram = () => request<TelegramOverview>('/api/me/telegram')
export const createTelegramLink = () =>
  request<{ url: string; expires_at: number }>('/api/me/telegram/link', { method: 'POST' })
export const sendTelegramTest = () => request<void>('/api/me/telegram/test', { method: 'POST' })
export const deleteTelegramLink = (id: string) => request<void>(`/api/me/telegram/${id}`, { method: 'DELETE' })

/** Login is a full navigation: it goes through Google and comes back with the cookie. */
export const LOGIN_URL = '/auth/google/login'
