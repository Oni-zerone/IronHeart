<script setup lang="ts">
import type { AppInput } from '@/api'
import { currentUser } from '@/session'

import DurationField from './DurationField.vue'

const model = defineModel<AppInput>({ required: true })
</script>

<template>
  <div class="d-flex flex-column ga-4">
    <v-text-field v-model="model.name" label="Nome" maxlength="80" counter autofocus />
    <DurationField
      v-model="model.interval_secs"
      label="Intervallo"
      hint="Ogni quanto l'applicazione deve inviare l'heartbeat (da 30 secondi a 7 giorni)."
    />
    <DurationField
      v-model="model.grace_secs"
      label="Tolleranza"
      hint="Attesa in più prima di considerarla ferma (fino a 1 giorno)."
    />
    <v-combobox
      v-model="model.recipients"
      label="Destinatari degli avvisi"
      :hint="`Premi Invio dopo ogni indirizzo. Se vuoto, gli avvisi vanno a ${currentUser?.email ?? 'te'}.`"
      persistent-hint
      multiple
      chips
      closable-chips
    />
  </div>
</template>
