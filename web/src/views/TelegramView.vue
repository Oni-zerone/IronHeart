<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import {
  createTelegramLink,
  deleteTelegramLink,
  getTelegram,
  sendTelegramTest,
  type TelegramLink,
  type TelegramOverview,
} from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { formatDateTime } from '@/format'
import { useNow } from '@/grid'
import { copy, currentUser, errorText, loadSession, notify } from '@/session'

/** While a link is pending, the page checks every few seconds whether the chat arrived. */
const POLL_MS = 3000

const now = useNow()
const overview = ref<TelegramOverview | null>(null)
const invite = ref<{ url: string; expires_at: number } | null>(null)
const creating = ref(false)
const testing = ref(false)
const toUnlink = ref<TelegramLink | null>(null)
let pollTimer: ReturnType<typeof setInterval> | undefined

const commands = [
  ['/apps', 'all applications and their state'],
  ['/down', 'applications that are down'],
  ['/status', 'details of an application'],
  ['/attempts', 'invalid heartbeat attempts'],
  ['/pause', 'pause an application (asks to confirm)'],
  ['/resume', 'resume an application (asks to confirm)'],
  ['/test', 'test message'],
  ['/stop', 'unlink the chat'],
]

const activeLinks = computed(() => (overview.value?.links ?? []).filter((l) => l.disabled_at === null))

/** The token is set but Telegram does not accept it (yet): the page only explains why. */
const active = computed(() => Boolean(currentUser.value?.telegram))
const problem = computed(() => {
  switch (currentUser.value?.alerts.telegram) {
    case 'checking':
      return 'Checking the bot token with Telegram. Reload the page in a moment.'
    case 'unreachable':
      return 'Telegram cannot be reached at the moment: the bot starts as soon as it answers.'
    default:
      return 'Telegram refused the bot token, or has been unreachable for over an hour: alerts on Telegram are not sent. The administrator must check TELEGRAM_BOT_TOKEN and restart IronHeart.'
  }
})

const expiresIn = computed(() => {
  if (!invite.value) return ''
  const secs = Math.max(0, Math.ceil((invite.value.expires_at - now.value) / 1000))
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`
})

async function load() {
  try {
    overview.value = await getTelegram()
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

function stopPolling() {
  clearInterval(pollTimer)
  pollTimer = undefined
}

async function poll() {
  if (!invite.value || Date.now() > invite.value.expires_at) {
    invite.value = null
    stopPolling()
    return
  }
  const known = new Set((overview.value?.links ?? []).map((l) => `${l.id}:${l.created_at}`))
  try {
    const fresh = await getTelegram()
    overview.value = fresh
    if (fresh.links.some((l) => !known.has(`${l.id}:${l.created_at}`))) {
      invite.value = null
      stopPolling()
      notify('Chat linked.')
      await loadSession()
    }
  } catch {
    // Retried at the next tick.
  }
}

async function createLink() {
  creating.value = true
  try {
    invite.value = await createTelegramLink()
    stopPolling()
    pollTimer = setInterval(poll, POLL_MS)
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    creating.value = false
  }
}

async function test() {
  testing.value = true
  try {
    await sendTelegramTest()
    notify('Test message sent.')
  } catch (err) {
    notify(errorText(err), 'error')
    await Promise.all([load(), loadSession()])
  } finally {
    testing.value = false
  }
}

async function confirmUnlink() {
  if (!toUnlink.value) return
  try {
    await deleteTelegramLink(toUnlink.value.id)
    toUnlink.value = null
    await Promise.all([load(), loadSession()])
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

onMounted(() => {
  if (active.value) load()
})
onBeforeUnmount(stopPolling)
</script>

<template>
  <div v-if="!active" class="page">
    <div class="page-head">
      <h1>telegram</h1>
    </div>
    <AsciiBox title="not working" hot>
      <div class="notice">
        <span class="hi">!!</span>
        <p>{{ problem }}</p>
      </div>
    </AsciiBox>
  </div>
  <div v-else class="page">
    <div class="page-head">
      <h1>telegram</h1>
      <button class="btn" type="button" :disabled="creating" @click="createLink">
        <LoadingSpinner v-if="creating" />link telegram
      </button>
    </div>

    <AsciiBox title="how it works">
      <div class="form">
        <p>
          The alerts that reach <span class="acc">{{ currentUser?.email }}</span> also arrive in your Telegram
          chats with <span class="acc">@{{ overview?.bot_username ?? currentUser?.telegram?.bot_username }}</span>, and the bot answers a few commands
          about your applications.
        </p>
        <ol class="steps">
          <li>
            <span class="acc">1.</span>
            <span>Press "link telegram": a link valid for 10 minutes appears.</span>
          </li>
          <li>
            <span class="acc">2.</span>
            <span>Open it on the device with Telegram and press Start in the chat with the bot.</span>
          </li>
          <li>
            <span class="acc">3.</span>
            <span>The chat shows up here. Repeat for other chats or devices.</span>
          </li>
        </ol>
        <p class="help">
          Telegram follows the email recipients: an application with its own recipient list sends here only if
          your address is in that list.
        </p>
      </div>
    </AsciiBox>

    <AsciiBox v-if="invite" title="link" :right="`expires ${expiresIn}`">
      <div class="form">
        <div class="row">
          <span class="acc url">{{ invite.url }}</span>
          <button class="link-btn" type="button" aria-label="Copy the link" @click="copy(invite.url)">[copy]</button>
        </div>
        <div class="actions">
          <a class="btn" :href="invite.url" target="_blank" rel="noopener">open telegram</a>
          <span class="dim"><LoadingSpinner />waiting for the chat</span>
        </div>
        <p class="help">The link works once. Do not share it: whoever opens it receives your alerts.</p>
      </div>
    </AsciiBox>

    <p v-if="!overview" class="dim"><LoadingSpinner />loading</p>
    <AsciiBox v-else title="chats" :right="String(overview.links.length)">
      <div class="form">
        <p v-if="overview.links.length === 0">No linked chats.</p>
        <ul v-else class="chats">
          <li v-for="link in overview.links" :key="link.id" class="row">
            <span :class="link.disabled_at ? 'hi' : 'acc'">{{ link.label }}</span>
            <span class="dim">linked {{ formatDateTime(link.created_at) }}</span>
            <span v-if="link.disabled_at" class="hi">blocked: write to the bot to resume</span>
            <span class="grow" />
            <button class="link-btn danger" type="button" @click="toUnlink = link">[unlink]</button>
          </li>
        </ul>
        <div class="actions">
          <button class="btn" type="button" :disabled="testing || activeLinks.length === 0" @click="test">
            <LoadingSpinner v-if="testing" />send test
          </button>
          <a class="btn plain" :href="overview.bot_url" target="_blank" rel="noopener">open the bot</a>
        </div>
      </div>
    </AsciiBox>

    <AsciiBox title="commands">
      <dl class="kv">
        <template v-for="[command, description] in commands" :key="command">
          <dt class="acc">{{ command }}</dt>
          <dd>{{ description }}</dd>
        </template>
      </dl>
    </AsciiBox>

    <ConfirmDialog
      v-if="toUnlink"
      :title="`unlink ${toUnlink.label}?`"
      action="unlink"
      danger
      @confirm="confirmUnlink"
      @cancel="toUnlink = null"
    >
      Alerts stop arriving in this chat. You can link it again at any time.
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.steps li {
  display: flex;
  gap: 1ch;
}
.steps li > :first-child {
  flex: none;
}
.url {
  overflow-wrap: anywhere;
  min-width: 0;
}
.chats {
  display: flex;
  flex-direction: column;
}
</style>
