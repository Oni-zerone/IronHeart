<script setup lang="ts">
import { computed, onMounted } from 'vue'
import {
  mdiAccountMultipleOutline,
  mdiAtom,
  mdiDatabaseAlertOutline,
  mdiDatabaseOutline,
  mdiHeartPulse,
  mdiLogout,
} from '@mdi/js'

import LoginView from '@/views/LoginView.vue'
import { currentUser, loadSession, logout, toast } from '@/session'

onMounted(loadSession)

/** Indicazione del database per il master: SQLite locale si perde a ogni deploy. */
const database = computed(() => {
  const info = currentUser.value?.database
  if (!info) return null
  return info.backend === 'postgres'
    ? {
        label: 'Postgres',
        color: 'success',
        icon: mdiDatabaseOutline,
        hint: `Database esterno: ${info.location}`,
      }
    : {
        label: 'SQLite locale',
        color: 'warning',
        icon: mdiDatabaseAlertOutline,
        hint: `File ${info.location}: senza un volume i dati si perdono a ogni deploy. Imposta DATABASE_URL per usare Postgres.`,
      }
})
</script>

<template>
  <v-app>
    <v-app-bar flat border="b">
      <template #prepend>
        <v-icon :icon="mdiHeartPulse" color="primary" class="ms-2" />
      </template>
      <v-app-bar-title>
        <router-link :to="{ name: 'apps' }" class="text-decoration-none text-high-emphasis">IronHeart</router-link>
      </v-app-bar-title>
      <template v-if="currentUser" #append>
        <v-tooltip v-if="database" :text="database.hint" location="bottom" max-width="320">
          <template #activator="{ props }">
            <v-chip
              v-bind="props"
              :color="database.color"
              :prepend-icon="database.icon"
              size="small"
              variant="tonal"
              class="me-2"
            >
              <span class="d-none d-sm-inline">{{ database.label }}</span>
            </v-chip>
          </template>
        </v-tooltip>
        <v-btn
          v-if="currentUser.is_master"
          :prepend-icon="mdiAtom"
          :to="{ name: 'entanglement' }"
          variant="text"
          class="me-1"
        >
          <span class="d-none d-md-inline">Entanglement</span>
        </v-btn>
        <v-btn
          v-if="currentUser.role === 'admin'"
          :prepend-icon="mdiAccountMultipleOutline"
          :to="{ name: 'users' }"
          variant="text"
          class="me-2"
        >
          <span class="d-none d-md-inline">Utenti</span>
        </v-btn>
        <span class="text-body-2 me-2 d-none d-sm-inline">{{ currentUser.email }}</span>
        <v-btn :icon="mdiLogout" title="Esci" @click="logout" />
      </template>
    </v-app-bar>

    <v-main>
      <v-container v-if="currentUser === undefined" class="d-flex justify-center pa-12">
        <v-progress-circular indeterminate />
      </v-container>
      <LoginView v-else-if="currentUser === null" />
      <router-view v-else />
    </v-main>

    <v-snackbar v-model="toast.show" :color="toast.color" timeout="3000">{{ toast.text }}</v-snackbar>
  </v-app>
</template>
