<script setup lang="ts">
import { computed } from 'vue'

import type { AppState } from '@/api'
import { STATE_GLYPH, STATE_LABEL } from '@/format'

const props = defineProps<{ state: AppState; padded?: boolean }>()

const label = computed(() => STATE_LABEL[props.state].toUpperCase())
// Fixed width in lists, so that the names line up.
const pad = computed(() => (props.padded ? ' '.repeat(Math.max(0, 14 - label.value.length)) : ''))
</script>

<template>
  <span class="state" :class="`s-${state}`">
    <span class="glyph" :class="{ blink: state === 'down' }" aria-hidden="true">{{ STATE_GLYPH[state].padEnd(4) }}</span><span class="label">{{ label }}</span>{{ pad }}
  </span>
</template>

<style scoped>
.state {
  white-space: pre;
}
.s-up .glyph {
  color: var(--accent);
}
.s-down .glyph {
  color: var(--alert);
}
.s-down .label {
  background: var(--alert);
  color: var(--bg);
}
.s-new,
.s-paused {
  color: var(--dim);
}
</style>
