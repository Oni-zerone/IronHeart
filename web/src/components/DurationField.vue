<script setup lang="ts">
import { computed } from 'vue'

import { UNITS, splitDuration } from '@/format'

const seconds = defineModel<number>({ required: true })
/** `label` is for screen readers: the visible title is outside the field. */
defineProps<{ label: string; id?: string }>()

const parts = computed(() => splitDuration(seconds.value))

function setAmount(value: string) {
  const amount = Math.max(0, Math.round(Number(value) || 0))
  seconds.value = amount * parts.value.unit
}

function setUnit(unit: number) {
  seconds.value = parts.value.amount * unit
}
</script>

<template>
  <div class="row">
    <input
      :id="id"
      class="input amount"
      type="number"
      min="0"
      :aria-label="label"
      :value="parts.amount"
      @input="setAmount(($event.target as HTMLInputElement).value)"
    />
    <span class="select">
      <select
        :aria-label="`Unit of ${label.toLowerCase()}`"
        :value="parts.unit"
        @change="setUnit(Number(($event.target as HTMLSelectElement).value))"
      >
        <option v-for="u in UNITS" :key="u.value" :value="u.value">{{ u.title }}</option>
      </select>
    </span>
  </div>
</template>

<style scoped>
.amount {
  width: 9ch;
}
</style>
