<script setup lang="ts">
import { computed, ref } from 'vue'
import { mdiContentCopy } from '@mdi/js'

import type { Credentials } from '@/api'
import { notify } from '@/session'
import { shellSnippet } from '@/snippet'

const props = defineProps<{ credentials: Credentials | null; appName: string }>()
const emit = defineEmits<{ close: [] }>()

const saved = ref(false)
const snippet = computed(() =>
  props.credentials
    ? shellSnippet(window.location.origin, props.credentials.key_id, props.credentials.secret)
    : '',
)

async function copy(text: string) {
  await navigator.clipboard.writeText(text)
  notify('Copiato negli appunti.')
}

function close() {
  saved.value = false
  emit('close')
}
</script>

<template>
  <v-dialog :model-value="credentials !== null" max-width="720" persistent>
    <v-card v-if="credentials" :title="`Chiave di «${appName}»`">
      <v-card-text class="d-flex flex-column ga-4">
        <v-alert type="warning" variant="tonal">
          Il segreto si vede solo adesso: salvalo nella configurazione dell'applicazione. Se lo
          perdi, dovrai generare una nuova chiave.
        </v-alert>
        <v-text-field :model-value="credentials.key_id" label="Key ID" readonly>
          <template #append-inner>
            <v-btn :icon="mdiContentCopy" variant="text" size="small" @click="copy(credentials.key_id)" />
          </template>
        </v-text-field>
        <v-text-field :model-value="credentials.secret" label="Segreto" readonly class="font-mono">
          <template #append-inner>
            <v-btn :icon="mdiContentCopy" variant="text" size="small" @click="copy(credentials.secret)" />
          </template>
        </v-text-field>
        <div>
          <div class="d-flex align-center justify-space-between mb-1">
            <span class="text-subtitle-2">Esempio per un cron</span>
            <v-btn :prepend-icon="mdiContentCopy" variant="text" size="small" @click="copy(snippet)">
              Copia
            </v-btn>
          </div>
          <pre class="snippet">{{ snippet }}</pre>
        </div>
        <v-checkbox v-model="saved" label="Ho salvato il segreto" hide-details />
      </v-card-text>
      <v-card-actions>
        <v-spacer />
        <v-btn :disabled="!saved" color="primary" variant="flat" @click="close">Chiudi</v-btn>
      </v-card-actions>
    </v-card>
  </v-dialog>
</template>

<style scoped>
.snippet {
  font-size: 0.8rem;
  overflow-x: auto;
  padding: 12px;
  border-radius: 4px;
  background: rgba(var(--v-theme-on-surface), 0.06);
}
</style>
