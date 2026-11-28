<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'

import { LOGIN_URL } from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'

const route = useRoute()
const loginOutcome = computed(() => route.query.login)

// The heart with a pulse line of the old icon (mdiHeartPulse), in ASCII: a solid heart (edges
// shaded with " .:-=+*#") with the pulse line cut out of it. Outside the heart, streaks that thin
// out give a sense of speed: on the left one row below the line, on the right one row above.
const heart = [
  '                  .-=====-.       .-=====-.',
  '                :*#########*-   -*#########*:',
  '               =##########  #+.+#############=',
  '              :##########    #*###############:',
  '              :#########  #  #################:',
  '                         ###  ###############+',
  ' . - --- ----==.=###########   #############=.==---- --- - .',
  '                 .=##########  ##',
  '                   .=########      #####=.',
  '                     .-*######    ###*-.',
  '                        .=####   ##=.',
  '                          .=#####=.',
  '                            .+#+.',
  '                              -',
]
/** Columns taken by the streaks: before and after the heart on their row. */
const STREAKS: Record<number, (col: number) => boolean> = {
  6: (col) => col < 15 || col >= 46,
}

type Kind = 'heart' | 'streak-near' | 'streak' | 'streak-far'

function kindOf(row: number, col: number, char: string): Kind {
  if (!STREAKS[row]?.(col)) return 'heart'
  return char === '=' ? 'streak-near' : char === '.' ? 'streak-far' : 'streak'
}

/** Runs of characters of the same kind, rows joined by newlines. */
const logo = heart.flatMap((line, row) => {
  const runs: { text: string; kind: Kind }[] = []
  for (let col = 0; col < line.length; col++) {
    const char = line[col] ?? ' '
    const kind = char === ' ' ? (runs.at(-1)?.kind ?? 'heart') : kindOf(row, col, char)
    const last = runs.at(-1)
    if (last && last.kind === kind) last.text += char
    else runs.push({ text: char, kind })
  }
  if (row < heart.length - 1) runs.push({ text: '\n', kind: 'heart' })
  return runs
})
</script>

<template>
  <div class="login">
    <div class="logo-wrap">
      <pre class="logo" role="img" aria-label="IronHeart"><span v-for="(run, i) in logo" :key="i" :class="run.kind">{{ run.text }}</span></pre>
    </div>
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
          <span class="err">!!</span>
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
/* On narrow screens the drawing stays centered on the heart and loses the tips of the streaks. */
.logo-wrap {
  display: flex;
  justify-content: center;
  width: 100%;
  overflow: hidden;
}
.logo {
  flex: none;
  white-space: pre;
}
/* The heart glows twice, like a heartbeat. */
.heart {
  color: var(--accent);
  animation: heartbeat 1.4s ease-out infinite;
}
.streak-near {
  color: var(--hi);
}
.streak {
  color: var(--accent);
}
.streak-far {
  color: var(--dim);
}
@keyframes heartbeat {
  0%,
  30%,
  100% {
    text-shadow: 0 0 6px rgb(120 197 238 / 30%);
  }
  8%,
  38% {
    text-shadow:
      0 0 4px rgb(198 237 255 / 60%),
      0 0 16px var(--accent);
  }
}
@media (prefers-reduced-motion: reduce) {
  .heart {
    animation: none;
  }
}
.login-box {
  width: 100%;
  max-width: 60ch;
}
</style>
