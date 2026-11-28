<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import {
  acceptGroupInvite,
  createApp,
  listApps,
  listGroupInvites,
  rejectGroupInvite,
  sendTelegramTest,
  sendTestEmail,
  type App,
  type AppInput,
  type Credentials,
  type GroupInvite,
} from '@/api'
import AlertChannels from '@/components/AlertChannels.vue'
import AppForm from '@/components/AppForm.vue'
import AsciiBox from '@/components/AsciiBox.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import CredentialsDialog from '@/components/CredentialsDialog.vue'
import DeadlineBar from '@/components/DeadlineBar.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import StateTag from '@/components/StateTag.vue'
import { GROUP_ROLE_LABEL, formatDuration, formatElapsed, formatRelative } from '@/format'
import { useNow, usePolling } from '@/grid'
import { currentUser, errorText, loadSession, notify } from '@/session'

const MAX_APPS = 100

const router = useRouter()
const now = useNow()
const apps = ref<App[] | null>(null)
const groupInvites = ref<GroupInvite[]>([])
const answering = ref(false)

const creating = ref(false)
const saving = ref(false)
const createError = ref('')
const form = ref<AppInput>(emptyForm())
const created = ref<{ id: string; name: string; credentials: Credentials } | null>(null)
const sendingTest = ref(false)
const sendingTelegram = ref(false)

const summary = computed(() => {
  const list = apps.value ?? []
  const count = (state: App['state']) => list.filter((a) => a.state === state).length
  return `${count('up')} up · ${count('down')} down`
})

function emptyForm(): AppInput {
  return { name: '', interval_secs: 300, grace_secs: 60, recipients: [] }
}

async function load(quiet = false) {
  try {
    const [list, invites] = await Promise.all([listApps(), listGroupInvites()])
    apps.value = list
    groupInvites.value = invites
  } catch (err) {
    if (!quiet) notify(errorText(err), 'error')
  }
}

async function answerInvite(invite: GroupInvite, accept: boolean) {
  answering.value = true
  try {
    if (accept) {
      await acceptGroupInvite(invite.group_id)
      notify(`You joined ${invite.group_name}.`)
      await loadSession()
    } else {
      await rejectGroupInvite(invite.group_id)
      notify('Invitation rejected.')
    }
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    answering.value = false
    await load(true)
  }
}

function caption(app: App): string {
  switch (app.state) {
    case 'up':
      return `deadline ${formatRelative(app.next_deadline_at, now.value)}`
    case 'down':
      return app.down_since === null ? 'stopped' : `stopped for ${formatElapsed(app.down_since, now.value)}`
    case 'paused':
      return 'no deadline'
    default:
      return 'waiting for the first heartbeat'
  }
}

function openCreate() {
  form.value = emptyForm()
  createError.value = ''
  creating.value = true
}

async function submitCreate() {
  saving.value = true
  createError.value = ''
  try {
    const result = await createApp(form.value)
    creating.value = false
    created.value = { id: result.app.id, name: result.app.name, credentials: result.credentials }
    await load()
  } catch (err) {
    createError.value = errorText(err)
  } finally {
    saving.value = false
  }
}

function closeCredentials() {
  const id = created.value?.id
  created.value = null
  if (id) router.push({ name: 'app', params: { id } })
}

async function testEmail() {
  if (sendingTest.value) return
  sendingTest.value = true
  try {
    await sendTestEmail()
    notify('Test email sent.')
  } catch {
    notify('Sending failed.', 'error')
  } finally {
    sendingTest.value = false
  }
}

async function testTelegram() {
  if (sendingTelegram.value) return
  sendingTelegram.value = true
  try {
    await sendTelegramTest()
    notify('Test message sent on Telegram.')
  } catch (err) {
    notify(errorText(err), 'error')
    // A chat that refused the message is now disabled: refresh the state.
    await loadSession()
  } finally {
    sendingTelegram.value = false
  }
}

// Shortcuts: n = new application, t = test email (not while typing or with a dialog open).
function onKey(event: KeyboardEvent) {
  if (event.metaKey || event.ctrlKey || event.altKey) return
  const target = event.target as HTMLElement | null
  if (target?.closest('input, select, textarea, [contenteditable]')) return
  if (document.querySelector('[role="dialog"]')) return
  if (event.key === 'n') {
    event.preventDefault()
    openCreate()
  } else if (event.key === 't' && currentUser.value?.alerts.mail) {
    event.preventDefault()
    testEmail()
  }
}

onMounted(() => {
  load()
  document.addEventListener('keydown', onKey)
})
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
// The monitor runs every 30 seconds: refresh so that a stopped app shows up without reloading.
usePolling(() => load(true), 15_000)
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>applications <span v-if="apps" class="dim">({{ apps.length }}/{{ MAX_APPS }})</span></h1>
      <button v-if="currentUser?.alerts.mail" class="btn plain" type="button" aria-keyshortcuts="t" @click="testEmail">
        <LoadingSpinner v-if="sendingTest" /><kbd>t</kbd>est email
      </button>
      <button
        v-if="currentUser?.telegram && currentUser.alerts.telegram_linked"
        class="btn plain"
        type="button"
        @click="testTelegram"
      >
        <LoadingSpinner v-if="sendingTelegram" />test telegram
      </button>
      <button class="btn" type="button" aria-keyshortcuts="n" @click="openCreate"><kbd>n</kbd>ew application</button>
    </div>

    <AlertChannels part="notices" />

    <AsciiBox v-if="groupInvites.length" title="pending" :right="`${groupInvites.length} for you`" hot>
      <div class="form">
        <div class="scroll-x">
          <div class="tbl pending" role="table" aria-label="Pending requests">
            <span class="th" role="columnheader">request</span>
            <span class="th" role="columnheader">name</span>
            <span class="th" role="columnheader">detail</span>
            <span class="th" role="columnheader">from</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="invite in groupInvites" :key="invite.group_id">
              <span class="acc" role="cell">GROUP</span>
              <span class="name" role="cell">{{ invite.group_name }}</span>
              <span role="cell">as {{ GROUP_ROLE_LABEL[invite.role] }}</span>
              <span class="dim" role="cell">{{ invite.invited_by ?? '-' }}</span>
              <span class="row" role="cell">
                <button class="btn" type="button" :disabled="answering" @click="answerInvite(invite, true)">accept</button>
                <button class="btn plain" type="button" :disabled="answering" @click="answerInvite(invite, false)">reject</button>
              </span>
            </template>
          </div>
        </div>
        <p class="help">Group members see and manage the group's applications and receive their alerts. Expires in 7 days.</p>
      </div>
    </AsciiBox>

    <p v-if="!apps" class="dim"><LoadingSpinner />loading</p>

    <AsciiBox v-else-if="apps.length === 0" title="empty">
      <div class="form">
        <p>No applications yet. Create one to get the key to use in the heartbeat.</p>
        <pre class="dim scroll-x">  app --(signed POST every N s)--> ironheart --(silence)--> alert</pre>
      </div>
    </AsciiBox>

    <AsciiBox v-else title="status" :right="summary">
      <ul class="apps">
        <li v-for="app in apps" :key="app.id">
          <RouterLink class="app-row" :to="{ name: 'app', params: { id: app.id } }">
            <div class="line">
              <span class="pre"><span class="cursor" aria-hidden="true">&gt; </span><StateTag :state="app.state" padded /></span>
              <span class="name">{{ app.name }}</span>
              <span v-if="app.failures_24h > 0" class="fails" title="Invalid attempts in the last 24 hours">
                !{{ app.failures_24h }} invalid
              </span>
            </div>
            <div class="line sub">
              <DeadlineBar :app="app" :now="now" />
              <span>{{ caption(app) }}</span>
              <span class="hide-sm">· every {{ formatDuration(app.interval_secs) }}</span>
            </div>
          </RouterLink>
        </li>
      </ul>
    </AsciiBox>

    <p v-if="apps?.length" class="help">
      [####....::] interval elapsed · left · tolerance. A stopped app blinks until a valid heartbeat arrives.
    </p>

    <AsciiDialog v-if="creating" title="new application" :width="72" @close="creating = false">
      <form class="form" @submit.prevent="submitCreate">
        <div v-if="createError" class="notice">
          <span class="err">!!</span>
          <p>{{ createError }}</p>
        </div>
        <AppForm v-model="form" />
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="creating = false">cancel</button>
          <button class="btn" type="submit" :disabled="saving"><LoadingSpinner v-if="saving" />create</button>
        </div>
      </form>
    </AsciiDialog>

    <CredentialsDialog
      v-if="created"
      :credentials="created.credentials"
      :app-name="created.name"
      @close="closeCredentials"
    />
  </div>
</template>

<style scoped>
.apps {
  display: flex;
  flex-direction: column;
  gap: var(--lh);
}
.app-row {
  display: block;
  margin: 0 -1ch;
  padding: 0 1ch;
}
.app-row:hover,
.app-row:focus-visible {
  background: var(--hover);
  outline: none;
}
.cursor {
  color: transparent;
}
.app-row:hover .cursor,
.app-row:focus-visible .cursor {
  color: var(--accent);
}
.line {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 2ch;
  min-width: 0;
}
.name {
  color: var(--hi);
  overflow-wrap: anywhere;
}
.pending {
  grid-template-columns: auto minmax(12ch, auto) auto minmax(0, 1fr) auto;
}
.sub {
  padding-left: 2ch;
  color: var(--dim);
}
/* Pushed to the right, also when it wraps onto its own line. */
.fails {
  margin-left: auto;
  color: var(--alert);
  white-space: nowrap;
}
</style>
