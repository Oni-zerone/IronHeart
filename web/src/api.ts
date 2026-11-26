// Chiamate al backend. Il cookie di sessione viaggia da solo (stessa origine).

export interface User {
  id: string
  email: string
  name: string | null
}

export class ApiError extends Error {
  constructor(readonly status: number) {
    super(`HTTP ${status}`)
  }
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(path, { credentials: 'same-origin', ...init })
  if (!response.ok) throw new ApiError(response.status)
  return response
}

/** L'utente collegato, oppure `null` senza sessione. */
export async function getMe(): Promise<User | null> {
  try {
    return (await (await request('/api/me')).json()) as User
  } catch (err) {
    if (err instanceof ApiError && err.status === 401) return null
    throw err
  }
}

export async function logout(): Promise<void> {
  await request('/auth/logout', { method: 'POST' })
}

export async function sendTestEmail(): Promise<void> {
  await request('/api/me/test-email', { method: 'POST' })
}

/** Il login è una navigazione completa: passa da Google e torna con il cookie. */
export const LOGIN_URL = '/auth/google/login'
