<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { mdiGoogle } from '@mdi/js'

import { LOGIN_URL } from '@/api'

const route = useRoute()
const loginFailed = computed(() => route.query.login === 'failed')
</script>

<template>
  <v-container class="py-8" max-width="480">
    <v-card title="Accedi" subtitle="Usa il tuo account Google">
      <v-card-text>
        <v-alert v-if="loginFailed" type="error" variant="tonal" class="mb-4">
          Accesso non riuscito. Riprova.
        </v-alert>
        <v-btn :href="LOGIN_URL" :prepend-icon="mdiGoogle" color="primary" size="large" block>
          Accedi con Google
        </v-btn>
      </v-card-text>
    </v-card>
  </v-container>
</template>
