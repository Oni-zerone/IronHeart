<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import {
  cancelTransfer,
  deleteApp,
  getApp,
  moveApp,
  requestTransfer,
  rotateKey,
  updateApp,
  type AppDetail,
  type AppInput,
  type Credentials,
  type FailureBucket,
} from '@/api'
import AppForm from '@/components/AppForm.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import AsciiBox from '@/components/AsciiBox.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import CredentialsDialog from '@/components/CredentialsDialog.vue'
import DeadlineBar from '@/components/DeadlineBar.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import StateTag from '@/components/StateTag.vue'
import {
  GROUP_ROLE_LABEL,
  formatDateTime,
  formatDuration,
  formatElapsed,
  formatRelative,
  formatTime,
} from '@/format'
import { useNow, usePolling } from '@/grid'
import { copy, currentUser, errorText, notify } from '@/session'
import { highlightSnippet, shellSnippet } from '@/snippet'

const props = defineProps<{ id: string }>()
const router = useRouter()
const now = useNow()

const HOUR = 3_600_000

const app = ref<AppDetail | null>(null)
const form = ref<AppInput | null>(null)
const saving = ref(false)
const saveError = ref('')
const credentials = ref<Credentials | null>(null)
const confirm = ref<'rotate' | 'delete' | null>(null)
const busy = ref(false)

const snippet = computed(() => (app.value ? shellSnippet(window.location.origin, app.value.key_id) : ''))
const recipientsText = computed(() => {
  if (app.value?.recipients.length) return app.value.recipients.join(', ')
  return app.value?.owner.kind === 'group'
    ? `members of ${app.value.owner.name}`
    : `${currentUser.value?.email} (account)`
})
/** Fallback recipients, for the help of the settings form. */
const fallback = computed(() =>
  app.value?.owner.kind === 'group' ? `every member of ${app.value.owner.name}` : undefined,
)

// ---- Transfer to another user (a request they accept) and move (immediate) ----

const transferring = ref(false)
const transferEmail = ref('')
const transferError = ref('')
/** Address the request was sent to: the dialog then shows the confirmation. */
const transferSent = ref('')

interface MoveTarget {
  /** 'me', or a group id. */
  value: string
  label: string
  note: string
}

const moving = ref(false)
const moveTarget = ref('')
const moveError = ref('')

/** Where the app can go right away: the user (group apps), or their other groups. */
const moveTargets = computed<MoveTarget[]>(() => {
  const a = app.value
  if (!a?.can_manage) return []
  const out: MoveTarget[] = []
  if (a.owner.kind === 'group') out.push({ value: 'me', label: 'me', note: 'personal' })
  for (const g of currentUser.value?.groups ?? []) {
    if (a.owner.kind === 'group' && a.owner.id === g.id) continue
    out.push({ value: g.id, label: `group: ${g.name}`, note: GROUP_ROLE_LABEL[g.role] })
  }
  return out
})

/** With no recipients configured, who gets the alerts after the move. */
const moveRecipients = computed(() => {
  if (app.value?.recipients.length) return ''
  const target = moveTargets.value.find((t) => t.value === moveTarget.value)
  if (!target) return ''
  return target.value === 'me'
    ? 'No alert recipients are configured: after the move alerts go to you only.'
    : `No alert recipients are configured: after the move alerts go to every member of ${target.label.slice(7)}.`
})

function openTransfer() {
  transferEmail.value = ''
  transferError.value = ''
  transferSent.value = ''
  transferring.value = true
}

async function submitTransfer() {
  busy.value = true
  transferError.value = ''
  try {
    const sent = await requestTransfer(props.id, transferEmail.value)
    transferSent.value = sent.to_email
    await refresh()
  } catch (err) {
    transferError.value = errorText(err)
  } finally {
    busy.value = false
  }
}

async function cancelRequest() {
  try {
    await cancelTransfer(props.id)
    notify('Transfer request cancelled.')
    await refresh()
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

function openMove() {
  moveTarget.value = moveTargets.value[0]?.value ?? ''
  moveError.value = ''
  moving.value = true
}

async function submitMove() {
  const target = moveTargets.value.find((t) => t.value === moveTarget.value)
  if (!target) return
  busy.value = true
  moveError.value = ''
  try {
    await moveApp(props.id, target.value === 'me' ? { kind: 'user' } : { kind: 'group', id: target.value })
    moving.value = false
    notify(`Moved to ${target.label}.`)
    setApp(await getApp(props.id))
  } catch (err) {
    moveError.value = errorText(err)
  } finally {
    busy.value = false
  }
}

/** Invalid attempts per hour over the last 24 hours, oldest first. */
const hourly = computed(() => {
  const hours = Array.from({ length: 24 }, () => 0)
  for (const b of app.value?.failures ?? []) {
    const index = 23 - Math.floor((now.value - b.period_start) / HOUR)
    if (index >= 0 && index < 24) hours[index] = (hours[index] ?? 0) + b.bad_signature + b.replay + b.clock_skew
  }
  return hours
})
const hourlyMax = computed(() => Math.max(1, ...hourly.value))
/** Two characters per hour, density from the ramp " .:-=+*#%@"; "_" is an hour without attempts. */
const spark = computed(() => {
  const ramp = ' .:-=+*#%@'
  return hourly.value
    .map((n) => {
      const c = n ? ramp[Math.max(1, Math.round((n / hourlyMax.value) * (ramp.length - 1)))] : '_'
      return `${c}${c}`
    })
    .join('')
})
const sparkAxis = `-24h${' '.repeat(18)}-12h${' '.repeat(19)}now`

function setApp(detail: AppDetail, resetForm = true) {
  app.value = detail
  if (resetForm) {
    form.value = {
      name: detail.name,
      interval_secs: detail.interval_secs,
      grace_secs: detail.grace_secs,
      recipients: [...detail.recipients],
    }
  }
}

async function load() {
  try {
    setApp(await getApp(props.id))
  } catch (err) {
    notify(errorText(err), 'error')
    router.replace({ name: 'apps' })
  }
}

// Fresh state and attempts, without touching the settings being edited.
async function refresh() {
  if (!app.value) return
  try {
    setApp(await getApp(props.id), false)
  } catch {
    // the next round will try again
  }
}

async function save() {
  if (!form.value) return
  saving.value = true
  saveError.value = ''
  try {
    setApp(await updateApp(props.id, form.value))
    notify('Changes saved.')
  } catch (err) {
    saveError.value = errorText(err)
  } finally {
    saving.value = false
  }
}

async function togglePause() {
  if (!app.value) return
  const pausing = app.value.state !== 'paused'
  try {
    setApp(await updateApp(props.id, { paused: pausing }), false)
    notify(pausing ? 'Monitoring paused.' : 'Monitoring resumed: waiting for a full interval.')
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

async function confirmAction() {
  busy.value = true
  try {
    if (confirm.value === 'rotate') {
      const result = await rotateKey(props.id)
      setApp(result.app, false)
      credentials.value = result.credentials
    } else if (confirm.value === 'delete') {
      await deleteApp(props.id)
      notify('Application deleted.')
      router.replace({ name: 'apps' })
    }
    confirm.value = null
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    busy.value = false
  }
}

function ipsText(bucket: FailureBucket): string {
  const list = Object.entries(bucket.ips)
    .sort((a, b) => b[1] - a[1])
    .map(([ip, n]) => `${ip} (${n})`)
  if (bucket.other_ips > 0) list.push(`${bucket.other_ips} more`)
  return list.join(', ')
}

onMounted(load)
usePolling(refresh, 15_000)
</script>

<template>
  <div class="page">
    <RouterLink class="back" :to="{ name: 'apps' }">&lt;- applications</RouterLink>
    <p v-if="!app || !form" class="dim"><LoadingSpinner />loading</p>
    <template v-else>
      <div class="page-head">
        <h1>{{ app.name }}</h1>
        <StateTag :state="app.state" />
        <span v-if="app.owner.kind === 'group'" class="nowrap">
          <span class="dim">owner</span>
          <RouterLink class="acc" :to="{ name: 'group', params: { id: app.owner.id } }">[group: {{ app.owner.name }}]</RouterLink>
        </span>
      </div>

      <div v-if="app.state === 'down'" class="notice">
        <span class="err blink">!!</span>
        <p class="err">
          No valid heartbeat for
          {{ app.last_seen_at === null ? 'a while' : formatElapsed(app.last_seen_at, now) }}. Alert sent to
          {{ recipientsText }} at {{ formatTime(app.down_since) }}.
        </p>
      </div>

      <div v-if="app.pending_transfer" class="notice">
        <span class="acc">&gt;</span>
        <p>
          Transfer requested to <span class="hi">{{ app.pending_transfer.to_email }}</span> ·
          {{ formatDateTime(app.pending_transfer.created_at) }} · expires
          {{ formatRelative(app.pending_transfer.expires_at, now) }}. The app keeps working here until it is accepted.
          <button class="link-btn" type="button" @click="cancelRequest">[cancel request]</button>
        </p>
      </div>

      <div class="columns">
        <AsciiBox title="status" :hot="app.state === 'down'">
          <div class="form">
            <dl class="kv">
              <dt>owner</dt>
              <dd v-if="app.owner.kind === 'group'">
                group <span class="hi">{{ app.owner.name }}</span>
                <span class="dim">· you are {{ app.group_role === 'admin' ? 'a group admin' : 'a member' }}</span>
              </dd>
              <dd v-else>you</dd>
              <dt>last heartbeat</dt>
              <dd>
                {{ formatRelative(app.last_seen_at, now) }}
                <span v-if="app.last_seen_at" class="dim nowrap">· {{ formatDateTime(app.last_seen_at) }}</span>
              </dd>
              <template v-if="app.state === 'up'">
                <dt>deadline</dt>
                <dd>
                  {{ formatRelative(app.next_deadline_at, now) }}
                  <span class="dim nowrap">· {{ formatDateTime(app.next_deadline_at) }}</span>
                </dd>
              </template>
              <template v-if="app.state === 'down'">
                <dt>down since</dt>
                <dd class="err nowrap">{{ formatDateTime(app.down_since) }}</dd>
              </template>
              <dt>check</dt>
              <dd>every {{ formatDuration(app.interval_secs) }}, tolerance {{ formatDuration(app.grace_secs) }}</dd>
              <dt>alerts to</dt>
              <dd>{{ recipientsText }}</dd>
              <dt>key id</dt>
              <dd>
                <span class="acc">{{ app.key_id }}</span> <button class="link-btn" type="button" @click="copy(app.key_id)">[copy]</button>
              </dd>
            </dl>
            <div class="scroll-x"><DeadlineBar :app="app" :now="now" :width="30" /></div>
            <div class="actions">
              <button class="btn plain" type="button" @click="togglePause">
                {{ app.state === 'paused' ? 'resume monitoring' : 'pause' }}
              </button>
              <button class="btn plain" type="button" @click="confirm = 'rotate'">new key</button>
            </div>
          </div>
        </AsciiBox>

        <AsciiBox title="settings">
          <form class="form" @submit.prevent="save">
            <div v-if="saveError" class="notice">
              <span class="err">!!</span>
              <p>{{ saveError }}</p>
            </div>
            <AppForm v-model="form" :fallback="fallback" />
            <p v-if="!app.can_manage" class="help">
              Only admins of {{ app.owner.name }} can delete it or move it out of the group.
            </p>
            <div class="actions">
              <button v-if="app.can_manage" class="btn danger" type="button" @click="confirm = 'delete'">delete</button>
              <span class="grow" />
              <button v-if="moveTargets.length" class="btn plain" type="button" @click="openMove">move</button>
              <button v-if="app.can_manage" class="btn plain" type="button" @click="openTransfer">transfer</button>
              <button class="btn" type="submit" :disabled="saving"><LoadingSpinner v-if="saving" />save</button>
            </div>
          </form>
        </AsciiBox>
      </div>

      <AsciiBox title="invalid attempts" right="last 24 hours">
        <p v-if="app.failures.length === 0">No invalid attempts.</p>
        <div v-else class="form">
          <div class="scroll-x" aria-hidden="true">
            <div class="pre"><span class="dim">|</span><span class="acc">{{ spark }}</span><span class="dim">|  max {{ hourlyMax }}/h</span></div>
            <div class="pre dim">{{ sparkAxis }}</div>
          </div>
          <div class="scroll-x">
            <div class="tbl failures" role="table" aria-label="Invalid attempts">
              <span class="th" role="columnheader">period</span>
              <span class="th num" role="columnheader">bad signature</span>
              <span class="th num" role="columnheader">replayed</span>
              <span class="th num" role="columnheader">clock</span>
              <span class="th" role="columnheader">source</span>
              <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
              <template v-for="bucket in app.failures" :key="bucket.flushed_at">
                <span role="cell">{{ formatDateTime(bucket.period_start) }}</span>
                <span class="num" :class="{ err: bucket.bad_signature }" role="cell">{{ bucket.bad_signature }}</span>
                <span class="num" :class="{ err: bucket.replay }" role="cell">{{ bucket.replay }}</span>
                <span class="num" :class="{ err: bucket.clock_skew }" role="cell">{{ bucket.clock_skew }}</span>
                <span role="cell">{{ ipsText(bucket) }}</span>
              </template>
            </div>
          </div>
        </div>
      </AsciiBox>

      <AsciiBox title="integration" right="hmac-sha256">
        <div class="form">
          <p>
            Every request is signed with HMAC-SHA256 and an ever-increasing timestamp; the secret never
            travels. The service always answers <span class="acc">204</span>: to check that it works, look
            here at the last heartbeat and the invalid attempts.
          </p>
          <div class="field">
            <div class="row">
              <span class="field-label grow">example cron job</span>
              <button class="link-btn" type="button" @click="copy(snippet)">[copy]</button>
            </div>
            <!-- eslint-disable-next-line vue/no-v-html -- escaped in highlightSnippet -->
            <pre class="code" v-html="highlightSnippet(snippet)"></pre>
          </div>
        </div>
      </AsciiBox>
    </template>

    <ConfirmDialog
      v-if="confirm === 'rotate'"
      title="generate a new key?"
      action="generate"
      :busy="busy"
      @confirm="confirmAction"
      @cancel="confirm = null"
    >
      The current key stops working immediately: you will have to update the application with the new one.
    </ConfirmDialog>
    <ConfirmDialog
      v-if="confirm === 'delete'"
      title="delete the application?"
      action="delete"
      danger
      :busy="busy"
      @confirm="confirmAction"
      @cancel="confirm = null"
    >
      The application, its key and the history of attempts are deleted.
    </ConfirmDialog>

    <AsciiDialog v-if="transferring && app" :title="`transfer ${app.name}`" @close="transferring = false">
      <form v-if="!transferSent" class="form" @submit.prevent="submitTransfer">
        <p>
          Ask another user to take this application over. It moves to them only when they accept, with the same key,
          state and history.
        </p>
        <div v-if="transferError" class="notice">
          <span class="err">!!</span>
          <p>{{ transferError }}</p>
        </div>
        <div class="field">
          <label class="field-label" for="transfer-email">new owner's email</label>
          <input
            id="transfer-email"
            v-model="transferEmail"
            class="input"
            type="email"
            placeholder="name@example.com"
            spellcheck="false"
            autocomplete="off"
          />
          <p class="help">
            They get an email (or a Telegram message if email is off) and find the request on their dashboard. It
            expires in 7 days.
          </p>
        </div>
        <div v-if="!app.recipients.length" class="notice">
          <span class="err">!!</span>
          <p>
            No alert recipients are configured: after the transfer alerts go to the new owner<template
              v-if="app.owner.kind === 'group'"
            >, no longer to the members of {{ app.owner.name }}</template>.
          </p>
        </div>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="transferring = false">cancel</button>
          <button class="btn" type="submit" :disabled="busy || !transferEmail.trim()">
            <LoadingSpinner v-if="busy" />send request
          </button>
        </div>
      </form>
      <div v-else class="form">
        <p><span class="acc">&gt;</span> Transfer request sent to <span class="hi">{{ transferSent }}</span>.</p>
        <p class="help">
          They can accept or reject it from their dashboard. Until then the app stays where it is and keeps working;
          you can cancel the request from this page.
        </p>
        <div class="actions">
          <span class="grow" />
          <button class="btn" type="button" @click="transferring = false">close</button>
        </div>
      </div>
    </AsciiDialog>

    <AsciiDialog v-if="moving && app" :title="`move ${app.name}`" @close="moving = false">
      <form class="form" @submit.prevent="submitMove">
        <p>
          Now owned by <span class="hi">{{ app.owner.kind === 'group' ? `group ${app.owner.name}` : 'you' }}</span>.
          Moving is immediate: same key, state, deadline and history.
        </p>
        <div v-if="moveError" class="notice">
          <span class="err">!!</span>
          <p>{{ moveError }}</p>
        </div>
        <div class="field" role="radiogroup" aria-label="New owner">
          <span class="field-label">new owner</span>
          <button
            v-for="target in moveTargets"
            :key="target.value"
            class="check pre"
            type="button"
            role="radio"
            :aria-checked="moveTarget === target.value"
            @click="moveTarget = target.value"
          ><span class="mark">{{ moveTarget === target.value ? '(*)' : '( )' }}</span> {{ target.label }}  <span class="dim">{{ target.note }}</span></button>
        </div>
        <div v-if="moveRecipients" class="notice">
          <span class="err">!!</span>
          <p>{{ moveRecipients }}</p>
        </div>
        <p class="help">
          {{
            app.owner.kind === 'group'
              ? 'Only your groups are listed. To give the app to another person, use transfer: they have to accept it.'
              : 'Group admins can move it out of the group again; plain members cannot.'
          }}
        </p>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="moving = false">cancel</button>
          <button class="btn" type="submit" :disabled="busy || !moveTarget"><LoadingSpinner v-if="busy" />move</button>
        </div>
      </form>
    </AsciiDialog>

    <CredentialsDialog
      v-if="credentials && app"
      :credentials="credentials"
      :app-name="app.name"
      @close="credentials = null"
    />
  </div>
</template>

<style scoped>
.back {
  color: var(--dim);
  align-self: flex-start;
}
.back:hover,
.back:focus-visible {
  color: var(--accent);
  outline: none;
}
.columns {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: var(--lh) 2ch;
  align-items: start;
}
@media (max-width: 880px) {
  .columns {
    grid-template-columns: minmax(0, 1fr);
  }
}
.failures {
  grid-template-columns: repeat(5, auto);
}
</style>
