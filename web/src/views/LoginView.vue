<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { mdiGoogle } from '@mdi/js'

import { LOGIN_URL } from '@/api'

const route = useRoute()
const loginOutcome = computed(() => route.query.login)
</script>

<template>
  <v-container class="py-8" max-width="480">
    <v-card title="Sign in" subtitle="Use your Google account">
      <v-card-text>
        <v-alert v-if="loginOutcome === 'denied'" type="warning" variant="tonal" class="mb-4">
          This Google account is not enabled. Ask an administrator for an invite and sign in with
          the invited address.
        </v-alert>
        <v-alert v-else-if="loginOutcome === 'failed'" type="error" variant="tonal" class="mb-4">
          Sign-in failed. Please try again.
        </v-alert>
        <v-btn :href="LOGIN_URL" :prepend-icon="mdiGoogle" color="primary" size="large" block>
          Sign in with Google
        </v-btn>
      </v-card-text>
    </v-card>
  </v-container>
</template>
