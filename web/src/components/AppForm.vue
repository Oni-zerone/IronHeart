<script setup lang="ts">
import type { AppInput } from '@/api'
import { currentUser } from '@/session'

import DurationField from './DurationField.vue'

const model = defineModel<AppInput>({ required: true })
</script>

<template>
  <div class="d-flex flex-column ga-6">
    <section>
      <label class="form-title" for="app-name">Name</label>
      <v-text-field id="app-name" v-model="model.name" maxlength="80" hide-details autofocus />
    </section>

    <section>
      <div class="form-title">Interval</div>
      <p class="form-help">Maximum time between two heartbeats, from 30 seconds to 7 days.</p>
      <DurationField v-model="model.interval_secs" label="Interval" />
    </section>

    <section>
      <div class="form-title">Grace period</div>
      <p class="form-help">
        Extra time to wait before reporting the application as down, up to 1 day.
      </p>
      <DurationField v-model="model.grace_secs" label="Grace period" />
    </section>

    <section>
      <label class="form-title" for="app-recipients">Alert recipients</label>
      <p class="form-help">
        Press Enter after each address. If the list is empty, alerts go to
        {{ currentUser?.email ?? 'you' }}.
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
