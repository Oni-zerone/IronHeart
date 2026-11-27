<script setup lang="ts">
import type { AppInput } from '@/api'
import { currentUser } from '@/session'

import DurationField from './DurationField.vue'

const model = defineModel<AppInput>({ required: true })
</script>

<template>
  <div class="d-flex flex-column ga-6">
    <section>
      <label class="form-title" for="app-name">Nome</label>
      <v-text-field id="app-name" v-model="model.name" maxlength="80" hide-details autofocus />
    </section>

    <section>
      <div class="form-title">Intervallo</div>
      <p class="form-help">Tempo massimo tra due heartbeat, da 30 secondi a 7 giorni.</p>
      <DurationField v-model="model.interval_secs" label="Intervallo" />
    </section>

    <section>
      <div class="form-title">Tolleranza</div>
      <p class="form-help">
        Attesa in più prima di segnalare l'applicazione come ferma, fino a 1 giorno.
      </p>
      <DurationField v-model="model.grace_secs" label="Tolleranza" />
    </section>

    <section>
      <label class="form-title" for="app-recipients">Destinatari degli avvisi</label>
      <p class="form-help">
        Premi Invio dopo ogni indirizzo. Se l'elenco è vuoto, gli avvisi vanno a
        {{ currentUser?.email ?? 'te' }}.
      </p>
      <v-combobox
        id="app-recipients"
        v-model="model.recipients"
        multiple
        chips
        closable-chips
        hide-details
      />
    </section>
  </div>
</template>

<style scoped>
.form-title {
  display: block;
  font-size: 1rem;
  font-weight: 500;
  margin-bottom: 4px;
}

.form-help {
  font-size: 0.875rem;
  opacity: var(--v-medium-emphasis-opacity);
  margin-bottom: 8px;
  hyphens: manual;
  overflow-wrap: break-word;
}
</style>
