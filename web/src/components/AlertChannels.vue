<script setup lang="ts">
// Alert channels of the instance and their state. `row`, under the top bar of
// every page:
//   alerts  mail [ok]  telegram [ok]
// `notices`, on the main page: a warning when the user receives nothing, or
// (master) the pings are held.
import { computed } from 'vue'

import type { ChannelHealth } from '@/api'
import { currentUser } from '@/session'

defineProps<{ part: 'row' | 'notices' }>()

const alerts = computed(() => currentUser.value?.alerts)

const labels: Record<ChannelHealth, { text: string; hot: boolean; hint: string }> = {
  checking: { text: 'checking', hot: false, hint: 'First check in progress.' },
  ok: { text: 'ok', hot: false, hint: 'Working.' },
  unreachable: {
    text: 'unreachable',
    hot: true,
    hint: 'The last check failed for a network error: after an hour it counts as broken.',
  },
  unverified: {
    text: 'unchecked',
    hot: false,
    hint: 'SMTP without login: the server cannot be checked, alerts are sent anyway.',
  },
  broken: { text: 'broken', hot: true, hint: 'The key was refused, or the provider has been unreachable for over an hour.' },
}

const telegramLabel = computed(() => {
  const a = alerts.value
  if (!a?.telegram) return null
  if (a.telegram === 'ok' && !a.telegram_linked) {
    return { text: 'not linked', hot: a.mode === 'telegram', hint: 'Link a Telegram chat to receive alerts there.' }
  }
  return labels[a.telegram]
})

/** Telegram-only instance and no linked chat: this user receives nothing. */
const unreached = computed(() => alerts.value?.mode === 'telegram' && !alerts.value.telegram_linked)
</script>

<template>
  <p v-if="alerts && part === 'row'" class="row">
      <span class="dim">alerts</span>
      <span v-if="alerts.mail" :title="labels[alerts.mail].hint">
        mail <span :class="labels[alerts.mail].hot ? 'err' : 'acc'">[{{ labels[alerts.mail].text }}]</span>
      </span>
      <span v-if="telegramLabel" :title="telegramLabel.hint">
        telegram <RouterLink :class="telegramLabel.hot ? 'err' : 'acc'" :to="{ name: 'telegram' }">[{{ telegramLabel.text }}]</RouterLink>
      </span>
  </p>
  <div v-else-if="alerts && (unreached || alerts.held)" class="stack">
    <div v-if="unreached" class="notice">
      <span class="err">!!</span>
      <p>
        You receive no alerts: this instance sends them only on Telegram.
        <RouterLink class="acc" :to="{ name: 'telegram' }">Link a chat</RouterLink> to receive them and to add
        applications.
      </p>
    </div>
    <div v-if="alerts.held" class="notice">
      <span class="err">!!</span>
      <p>
        No alert channel reaches you: the entanglement pings are held, so the linked instances report this one as
        down.
      </p>
    </div>
  </div>
</template>
