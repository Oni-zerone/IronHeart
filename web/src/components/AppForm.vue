<script setup lang="ts">
import { ref, useId } from 'vue'

import type { AppInput } from '@/api'
import { currentUser } from '@/session'
import DurationField from './DurationField.vue'

const model = defineModel<AppInput>({ required: true })
/** Who gets the alerts when the list is empty: "you", or "every member of ops". */
const props = defineProps<{ fallback?: string }>()
const id = useId()
const draft = ref('')

const MAX_RECIPIENTS = 10

function addRecipient() {
  const email = draft.value.trim().toLowerCase()
  if (email && !model.value.recipients.includes(email) && model.value.recipients.length < MAX_RECIPIENTS) {
    model.value.recipients.push(email)
  }
  draft.value = ''
}

function removeRecipient(index: number) {
  model.value.recipients.splice(index, 1)
}
</script>

<template>
  <div class="form">
    <div class="field">
      <label class="field-label" :for="`${id}-name`">name</label>
      <input
        :id="`${id}-name`"
        v-model="model.name"
        class="input"
        maxlength="80"
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    <div class="field">
      <label class="field-label" :for="`${id}-interval`">interval</label>
      <p class="help">Longest time between two heartbeats, from 30 seconds to 7 days.</p>
      <DurationField :id="`${id}-interval`" v-model="model.interval_secs" label="Interval" />
    </div>
    <div class="field">
      <label class="field-label" :for="`${id}-grace`">tolerance</label>
      <p class="help">Extra wait before the application is reported as stopped, up to 1 day.</p>
      <DurationField :id="`${id}-grace`" v-model="model.grace_secs" label="Tolerance" />
    </div>
    <div class="field">
      <label class="field-label" :for="`${id}-recipients`">
        alert recipients <span class="dim">({{ model.recipients.length }}/{{ MAX_RECIPIENTS }})</span>
      </label>
      <p class="help">
        Press Enter after each address. If the list is empty, alerts go to
        {{ props.fallback ?? currentUser?.email ?? 'you' }}.
        <template v-if="currentUser?.alerts.mode === 'telegram'">
          Email is off on this instance: only addresses of IronHeart accounts with a linked Telegram chat receive alerts.
        </template>
      </p>
      <div class="chips">
        <span v-for="(email, i) in model.recipients" :key="email" class="chip">
          &lt;{{ email }}
          <button class="x" type="button" :aria-label="`Remove ${email}`" @click="removeRecipient(i)">x</button>&gt;
        </span>
        <input
          :id="`${id}-recipients`"
          v-model="draft"
          class="input grow"
          type="email"
          placeholder="name@example.com"
          spellcheck="false"
          :disabled="model.recipients.length >= MAX_RECIPIENTS"
          @keydown.enter.prevent="addRecipient"
          @blur="addRecipient"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 0 1ch;
  align-items: baseline;
}
.chips .input {
  flex-basis: 20ch;
}
.chip {
  white-space: nowrap;
  color: var(--accent);
}
.x {
  background: none;
  border: 0;
  padding: 0;
  cursor: pointer;
  color: var(--dim);
}
.x:hover,
.x:focus-visible {
  color: var(--bg);
  background: var(--hi);
  outline: none;
}
</style>
