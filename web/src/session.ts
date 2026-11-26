import { ref } from 'vue'

import { getMe, logout as apiLogout, type User } from './api'

/** Stato della sessione condiviso dall'app. `undefined` = non ancora caricato. */
export const currentUser = ref<User | null | undefined>(undefined)

export async function loadSession(): Promise<void> {
  currentUser.value = await getMe()
}

export async function logout(): Promise<void> {
  await apiLogout()
  currentUser.value = null
}

/** Messaggio breve in basso, condiviso dalle pagine. */
export const toast = ref({ show: false, text: '', color: 'success' })

export function notify(text: string, color: 'success' | 'error' = 'success') {
  toast.value = { show: true, text, color }
}

export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : 'Operazione non riuscita.'
}
