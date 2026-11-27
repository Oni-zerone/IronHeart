<script setup lang="ts">
// Flat line under the top bar with a pulse that runs from left to right.
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import { reducedMotion, repeat, useColumns } from '@/grid'

const wrap = ref<HTMLElement | null>(null)
const cols = useColumns(wrap, 20)
const tick = ref(0)
const blip = '_/\\/\\_'
const trail = 10

let timer: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  if (reducedMotion) return
  timer = setInterval(() => {
    if (document.visibilityState === 'visible') tick.value++
  }, 90)
})
onBeforeUnmount(() => clearInterval(timer))

const parts = computed(() => {
  const w = cols.value
  const period = w + 24
  const at = reducedMotion ? Math.floor(w * 0.6) : ((tick.value * 2) % period) - 12
  const clamp = (n: number) => Math.max(0, Math.min(w, n))
  const start = clamp(at)
  const end = clamp(at + blip.length)
  const trailStart = clamp(at - trail)
  return {
    before: repeat('_', trailStart),
    trail: repeat('.', start - trailStart),
    blip: blip.slice(start - at, end - at),
    after: repeat('_', w - end),
  }
})
</script>

<template>
  <div ref="wrap" class="ecg-wrap">
    <div class="ecg" aria-hidden="true">{{ parts.before }}<span class="trail">{{ parts.trail }}</span><span class="blip">{{ parts.blip }}</span>{{ parts.after }}</div>
  </div>
</template>

<style scoped>
.ecg-wrap {
  min-width: 0;
}
.ecg {
  white-space: pre;
  overflow: hidden;
  color: var(--faint);
}
.trail {
  color: var(--dim);
}
.blip {
  color: var(--accent);
  text-shadow: 0 0 8px rgb(120 197 238 / 70%);
}
</style>
