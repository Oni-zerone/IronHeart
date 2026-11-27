<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'

import { LOGIN_URL } from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'

const route = useRoute()
const loginOutcome = computed(() => route.query.login)

// The heart with a pulse line of the old icon (mdiHeartPulse), drawn in ASCII.
// `pulse` marks with "x" the characters of the line, which get their own color.
const heart = [
  "    _.-'''-._   _.-'''-._",
  "  .'         '.'         '.",
  ' /                         \\',
  '|                           |',
  '|         /\\                |',
  '|________/  \\   /___________|',
  ' \\           \\ /           /',
  '  \\           V           /',
  "   '.                   .'",
  "     '.               .'",
  "       '.           .'",
  "         '.       .'",
  "           '.   .'",
  "             '.'",
]
const pulse = [
  '',
  '',
  '',
  '',
  '          xx',
  ' xxxxxxxxx  x   xxxxxxxxxxxx',
  '             x x',
  '              x',
]

/** Runs of characters with the same color, rows joined by newlines. */
const logo = heart.flatMap((line, row) => {
  const mask = pulse[row] ?? ''
  const runs: { text: string; pulse: boolean }[] = []
  for (let col = 0; col < line.length; col++) {
    const isPulse = mask[col] === 'x'
    const last = runs.at(-1)
    if (last && last.pulse === isPulse) last.text += line[col]
    else runs.push({ text: line[col] ?? '', pulse: isPulse })
  }
  if (row < heart.length - 1) runs.push({ text: '\n', pulse: false })
  return runs
})
</script>

<template>
  <div class="login">
    <pre class="logo glow" role="img" aria-label="IronHeart"><span v-for="(run, i) in logo" :key="i" :class="{ pulse: run.pulse }">{{ run.text }}</span></pre>
    <p class="dim">signed heartbeats · an alert when an app goes quiet</p>
    <AsciiBox class="login-box" title="sign in" right="google">
      <div class="form">
        <div v-if="loginOutcome === 'denied'" class="notice">
          <span class="hi">[!]</span>
          <p>
            This Google account is not enabled. Ask an administrator for an invite and sign in with the
            invited address.
          </p>
        </div>
        <div v-else-if="loginOutcome === 'failed'" class="notice">
          <span class="hi">!!</span>
          <p>Sign-in failed. Try again.</p>
        </div>
        <p>Access is by invitation only, with the Google account of the invited address.</p>
        <div class="actions">
          <a class="btn" :href="LOGIN_URL">sign in with google</a>
          <span class="acc blink" aria-hidden="true">_</span>
        </div>
      </div>
    </AsciiBox>
  </div>
</template>

<style scoped>
.login {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--lh);
  padding-block: calc(var(--lh) * 2) 0;
}
.logo {
  white-space: pre;
  color: var(--accent);
}
/* The pulse line flashes twice, like a heartbeat. */
.pulse {
  color: var(--hi);
  animation: heartbeat 1.4s ease-out infinite;
}
@keyframes heartbeat {
  0%,
  30%,
  100% {
    text-shadow: 0 0 6px rgb(198 237 255 / 35%);
  }
  8%,
  38% {
    text-shadow:
      0 0 4px var(--hi),
      0 0 14px var(--accent);
  }
}
@media (prefers-reduced-motion: reduce) {
  .pulse {
    animation: none;
  }
}
.login-box {
  width: 100%;
  max-width: 60ch;
}
</style>
