<script setup lang="ts">
import { computed } from 'vue'

import { UNITS, splitDuration } from '@/format'

const seconds = defineModel<number>({ required: true })
/** `label` is for screen readers: the visible title sits outside the field. */
defineProps<{ label: string }>()

const parts = computed(() => splitDuration(seconds.value))

function setAmount(value: string | number) {
  const amount = Math.max(0, Math.round(Number(value) || 0))
  seconds.value = amount * parts.value.unit
}

function setUnit(unit: number) {
  seconds.value = parts.value.amount * unit
}
</script>

<template>
  <div class="d-flex ga-3">
    <v-text-field
      :model-value="parts.amount"
      :aria-label="label"
      type="number"
      min="0"
      hide-details
      class="flex-grow-1"
      @update:model-value="setAmount"
    />
    <v-select
      :model-value="parts.unit"
      :items="UNITS"
      :aria-label="`${label} unit`"
      hide-details
      class="unit"
      @update:model-value="setUnit"
    />
  </div>
</template>

<style scoped>
.unit {
  flex: 0 0 160px;
}
</style>
