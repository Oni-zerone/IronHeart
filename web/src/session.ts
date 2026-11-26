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
