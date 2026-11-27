// Chiamate al backend. Il cookie di sessione viaggia da solo (stessa origine).

export type Role = 'admin' | 'user'

export interface User {
  id: string
  email: string
  name: string | null
  role: Role
  /** Utente master (ADMIN_EMAIL): sempre admin. */
  is_master: boolean
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
  /** Destinatari configurati; vuoto = email dell'account. */
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
    let message = `Errore ${response.status}`
    try {
      const body = (await response.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {
      // risposta senza JSON
    }
    throw new ApiError(response.status, message)
  }
  return (response.status === 204 ? undefined : await response.json()) as T
}

const json = (method: string, body: unknown): RequestInit => ({ method, body: JSON.stringify(body) })

/** L'utente collegato, oppure `null` senza sessione. */
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
  /** Ultimo ping consegnato (204): la validità la vede solo l'altra istanza. */
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

/** Il login è una navigazione completa: passa da Google e torna con il cookie. */
export const LOGIN_URL = '/auth/google/login'
