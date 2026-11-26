<script setup lang="ts">
import { computed } from 'vue'

import { UNITS, splitDuration } from '@/format'

const seconds = defineModel<number>({ required: true })
defineProps<{ label: string; hint?: string; min?: number }>()

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
  <div class="d-flex ga-2">
    <v-text-field
      :model-value="parts.amount"
      :label="label"
      :hint="hint"
      persistent-hint
      type="number"
      min="0"
      @update:model-value="setAmount"
    />
    <v-select
      :model-value="parts.unit"
      :items="UNITS"
      max-width="140"
      @update:model-value="setUnit"
    />
  </div>
</template>
