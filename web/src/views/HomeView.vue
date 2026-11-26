<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute } from 'vue-router'
import { mdiEmailFastOutline, mdiGoogle } from '@mdi/js'

import { LOGIN_URL, sendTestEmail } from '@/api'
import { currentUser } from '@/session'

const route = useRoute()
const loginFailed = computed(() => route.query.login === 'failed')

const sending = ref(false)
const snackbar = ref({ show: false, text: '', color: 'success' })

async function onTestEmail() {
  sending.value = true
  try {
    await sendTestEmail()
    snackbar.value = { show: true, text: 'Email di prova inviata.', color: 'success' }
  } catch {
    snackbar.value = { show: true, text: 'Invio non riuscito.', color: 'error' }
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <v-container class="py-8" max-width="640">
    <v-card v-if="!currentUser" title="Accedi" subtitle="Usa il tuo account Google">
      <v-card-text>
        <v-alert v-if="loginFailed" type="error" variant="tonal" class="mb-4">
          Accesso non riuscito. Riprova.
        </v-alert>
        <v-btn :href="LOGIN_URL" :prepend-icon="mdiGoogle" color="primary" size="large" block>
          Accedi con Google
        </v-btn>
      </v-card-text>
    </v-card>

    <v-card v-else :title="`Ciao ${currentUser.name ?? currentUser.email}`">
      <v-card-text>
        Qui compariranno le tue applicazioni. Intanto puoi verificare che gli avvisi ti arrivino.
      </v-card-text>
      <v-card-actions>
        <v-btn :prepend-icon="mdiEmailFastOutline" :loading="sending" @click="onTestEmail">
          Mandami una mail di prova
        </v-btn>
      </v-card-actions>
    </v-card>

    <v-snackbar v-model="snackbar.show" :color="snackbar.color" timeout="3000">
      {{ snackbar.text }}
    </v-snackbar>
  </v-container>
</template>
