import { ref } from 'vue'

import { getMe, logout as apiLogout, type User } from './api'

/** Session state shared by the app. `undefined` = not loaded yet. */
export const currentUser = ref<User | null | undefined>(undefined)

export async function loadSession(): Promise<void> {
  currentUser.value = await getMe()
}

export async function logout(): Promise<void> {
  await apiLogout()
  currentUser.value = null
}

/** Short message at the bottom of the screen, shared by all pages. */
export const toast = ref<{ text: string; error: boolean } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | undefined

export function notify(text: string, kind: 'success' | 'error' = 'success') {
  toast.value = { text, error: kind === 'error' }
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => (toast.value = null), 3000)
}

/** Copies to the clipboard and says how it went. */
export async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    notify('Copied to the clipboard.')
  } catch {
    notify('Copy failed: select the text by hand.', 'error')
  }
}

export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : 'Operation failed.'
}
