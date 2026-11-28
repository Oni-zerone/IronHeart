<script setup lang="ts">
// Time left before the app is reported as stopped:
//   [#######.........::::]  # interval elapsed  . interval left  ! tolerance elapsed  : tolerance left
import { computed } from 'vue'

import type { App } from '@/api'
import { repeat } from '@/grid'

const props = withDefaults(defineProps<{ app: App; now: number; width?: number }>(), { width: 24 })

const parts = computed<[string, string][]>(() => {
  const { app } = props
  const w = Math.max(6, props.width)
  if (app.state === 'down') return [['down', repeat('!', w)]]
  if (app.state === 'paused') return [['mute', ' -'.repeat(Math.ceil(w / 2)).slice(0, w)]]
  if (app.state === 'new' || app.last_seen_at === null || app.next_deadline_at === null) return [['off', repeat('.', w)]]
  const total = app.interval_secs + app.grace_secs
  const graceWidth = app.grace_secs ? Math.max(1, Math.round((w * app.grace_secs) / total)) : 0
  const intervalWidth = w - graceWidth
  // The deadline already counts the tolerance: start of the window = deadline - total.
  const start = app.next_deadline_at - total * 1000
  const filled = Math.min(w, Math.max(0, Math.round((w * (props.now - start)) / 1000 / total)))
  const inInterval = Math.min(filled, intervalWidth)
  const inGrace = Math.max(0, filled - intervalWidth)
  return [
    ['on', repeat('#', inInterval)],
    ['off', repeat('.', intervalWidth - inInterval)],
    ['grace', repeat('!', inGrace)],
    ['grace-off', repeat(':', graceWidth - inGrace)],
  ]
})
</script>

<template>
  <span class="bar" aria-hidden="true">[<span v-for="([kind, text], i) in parts" :key="i" :class="kind">{{ text }}</span>]</span>
</template>

<style scoped>
.bar {
  white-space: pre;
  color: var(--dim);
}
.on {
  color: var(--accent);
}
.off {
  color: var(--faint);
}
.grace,
.down {
  color: var(--alert);
}
.grace-off,
.mute {
  color: var(--dim);
}
</style>
