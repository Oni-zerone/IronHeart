<script setup lang="ts">
import { onMounted } from 'vue'
import { mdiHeartPulse, mdiLogout } from '@mdi/js'

import LoginView from '@/views/LoginView.vue'
import { currentUser, loadSession, logout, toast } from '@/session'

onMounted(loadSession)
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
