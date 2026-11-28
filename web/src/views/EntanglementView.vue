<script setup lang="ts">
import { onMounted, ref } from 'vue'

import {
  createEntanglement,
  deleteEntanglement,
  listEntanglements,
  pingEntanglement,
  updateEntanglement,
  type Entanglement,
  type EntanglementInput,
} from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import DurationField from '@/components/DurationField.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { formatDateTime, formatDuration, formatRelative } from '@/format'
import { useNow } from '@/grid'
import { copy, errorText, notify } from '@/session'

const now = useNow()
const items = ref<Entanglement[] | null>(null)
const instanceUrl = ref('')

const editing = ref<Entanglement | 'new' | null>(null)
const form = ref<EntanglementInput & { key_id: string; secret: string }>(emptyForm())
const replaceKey = ref(false)
const saving = ref(false)
const formError = ref('')
const toDelete = ref<Entanglement | null>(null)
const pinging = ref<string | null>(null)

const diagram = [
  '  this instance ---ping---> other instance',
  '        ^                          |',
  '        +----------ping------------+',
].join('\n')

function emptyForm() {
  return { name: '', base_url: 'https://', interval_secs: 60, enabled: true, key_id: '', secret: '' }
}

async function load() {
  try {
    const result = await listEntanglements()
    items.value = result.items
    instanceUrl.value = result.instance_url
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

function replace(updated: Entanglement) {
  items.value = (items.value ?? []).map((e) => (e.id === updated.id ? updated : e))
}

function openNew() {
  form.value = emptyForm()
  replaceKey.value = true
  formError.value = ''
  editing.value = 'new'
}

function openEdit(item: Entanglement) {
  form.value = { ...item, key_id: item.key_id, secret: '' }
  replaceKey.value = false
  formError.value = ''
  editing.value = item
}

async function save() {
  saving.value = true
  formError.value = ''
  const { key_id, secret, ...rest } = form.value
  try {
    if (editing.value === 'new') {
      await createEntanglement({ ...rest, key_id, secret })
      notify('Link created: the first ping goes out now.')
    } else if (editing.value) {
      replace(await updateEntanglement(editing.value.id, replaceKey.value ? { ...rest, key_id, secret } : rest))
      notify('Link updated.')
    }
    editing.value = null
    await load()
  } catch (err) {
    formError.value = errorText(err)
  } finally {
    saving.value = false
  }
}

async function toggle(item: Entanglement) {
  try {
    replace(await updateEntanglement(item.id, { enabled: !item.enabled }))
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

async function pingNow(item: Entanglement) {
  pinging.value = item.id
  try {
    const updated = await pingEntanglement(item.id)
    replace(updated)
    if (updated.last_error) notify(`Not delivered: ${updated.last_error}`, 'error')
    else notify('Ping delivered.')
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    pinging.value = null
  }
}

async function confirmDelete() {
  if (!toDelete.value) return
  try {
    await deleteEntanglement(toDelete.value.id)
    toDelete.value = null
    await load()
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

function status(item: Entanglement): { text: string; bad: boolean } {
  if (!item.enabled) return { text: 'paused', bad: false }
  if (item.last_attempt_at === null) return { text: 'waiting', bad: false }
  if (item.last_error) return { text: 'not delivered', bad: true }
  return { text: 'delivered', bad: false }
}

onMounted(load)
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>entanglement</h1>
      <button class="btn" type="button" @click="openNew">new link</button>
    </div>

    <AsciiBox title="how it works">
      <div class="form">
        <p>
          This instance sends a heartbeat to other IronHeart instances, so they watch each other: if this one
          stops, the other one sends the alert.
        </p>
        <pre class="dim scroll-x" aria-hidden="true">{{ diagram }}</pre>
        <ol class="steps">
          <li>
            <span class="acc">1.</span>
            <span>On the other instance, create an application that stands for this one, and copy key ID and secret.</span>
          </li>
          <li>
            <span class="acc">2.</span>
            <span>
              Here, create a link with the other instance's address, the key and an interval shorter than the
              application's (half, for example).
            </span>
          </li>
          <li>
            <span class="acc">3.</span>
            <span>For mutual monitoring, repeat the other way round with this instance's address.</span>
          </li>
        </ol>
        <div class="row">
          <span class="dim">this instance:</span>
          <span class="acc url">{{ instanceUrl }}</span>
          <button class="link-btn" type="button" aria-label="Copy the address" @click="copy(instanceUrl)">[copy]</button>
        </div>
        <p class="help">
          The other instance always answers the same way: here you only see whether the ping was delivered. If
          the key is wrong, the other instance reports it with the invalid attempts.
        </p>
      </div>
    </AsciiBox>

    <p v-if="!items" class="dim"><LoadingSpinner />loading</p>
    <AsciiBox v-else-if="items.length === 0" title="links"><p>No links.</p></AsciiBox>
    <div v-else class="list">
      <AsciiBox v-for="item in items" :key="item.id" :title="item.name" :right="status(item).text" :hot="status(item).bad">
        <div class="form">
          <dl class="kv">
            <dt>address</dt>
            <dd>{{ item.base_url }}</dd>
            <dt>key id</dt>
            <dd class="acc">{{ item.key_id }}</dd>
            <dt>ping</dt>
            <dd>every {{ formatDuration(item.interval_secs) }}</dd>
            <dt>delivered</dt>
            <dd>
              {{ formatRelative(item.last_success_at, now) }}
              <span v-if="item.last_success_at" class="dim nowrap">· {{ formatDateTime(item.last_success_at) }}</span>
            </dd>
            <template v-if="item.last_error">
              <dt>error</dt>
              <dd class="err">{{ formatRelative(item.last_attempt_at, now) }}: {{ item.last_error }}</dd>
            </template>
          </dl>
          <div class="actions">
            <button class="btn" type="button" :disabled="pinging === item.id" @click="pingNow(item)">
              <LoadingSpinner v-if="pinging === item.id" />send now
            </button>
            <button class="btn plain" type="button" @click="toggle(item)">{{ item.enabled ? 'pause' : 'resume' }}</button>
            <span class="grow" />
            <button class="link-btn" type="button" @click="openEdit(item)">[edit]</button>
            <button class="link-btn danger" type="button" @click="toDelete = item">[delete]</button>
          </div>
        </div>
      </AsciiBox>
    </div>

    <AsciiDialog
      v-if="editing"
      :title="editing === 'new' ? 'new link' : 'edit link'"
      :width="72"
      @close="editing = null"
    >
      <form class="form" @submit.prevent="save">
        <div v-if="formError" class="notice">
          <span class="err">!!</span>
          <p>{{ formError }}</p>
        </div>
        <div class="field">
          <label class="field-label" for="ent-name">name</label>
          <input id="ent-name" v-model="form.name" class="input" maxlength="80" spellcheck="false" />
        </div>
        <div class="field">
          <label class="field-label" for="ent-url">other instance address</label>
          <p class="help">Address only, no path. HTTPS required, HTTP only towards localhost.</p>
          <input id="ent-url" v-model="form.base_url" class="input" type="url" spellcheck="false" />
        </div>
        <div class="field">
          <span class="field-label">key</span>
          <p class="help">Key ID and secret of the application created on the other instance.</p>
          <button
            v-if="editing !== 'new'"
            class="check"
            type="button"
            :aria-pressed="replaceKey"
            @click="replaceKey = !replaceKey"
          >
            <span class="mark">[{{ replaceKey ? 'x' : ' ' }}]</span> replace the key
          </button>
          <template v-if="replaceKey">
            <div class="row">
              <label class="key-label dim" for="ent-key">key id</label>
              <input id="ent-key" v-model="form.key_id" class="input grow" spellcheck="false" autocomplete="off" />
            </div>
            <div class="row">
              <label class="key-label dim" for="ent-secret">secret</label>
              <input id="ent-secret" v-model="form.secret" class="input grow" type="password" autocomplete="off" />
            </div>
          </template>
        </div>
        <div class="field">
          <label class="field-label" for="ent-interval">ping interval</label>
          <p class="help">
            From 10 seconds to 1 day. Keep it shorter than the application's interval on the other instance, so
            a single lost ping is not enough to trigger the alert.
          </p>
          <DurationField id="ent-interval" v-model="form.interval_secs" label="Ping interval" />
        </div>
        <button class="check" type="button" :aria-pressed="form.enabled" @click="form.enabled = !form.enabled">
          <span class="mark">[{{ form.enabled ? 'x' : ' ' }}]</span> active
        </button>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="editing = null">cancel</button>
          <button class="btn" type="submit" :disabled="saving"><LoadingSpinner v-if="saving" />save</button>
        </div>
      </form>
    </AsciiDialog>

    <ConfirmDialog
      v-if="toDelete"
      :title="`delete ${toDelete.name}?`"
      action="delete"
      danger
      @confirm="confirmDelete"
      @cancel="toDelete = null"
    >
      This instance stops sending pings: if the application on the other instance is active, it will soon be
      reported as stopped.
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
}
.list {
  display: flex;
  flex-direction: column;
  gap: var(--lh);
}
.key-label {
  width: 8ch;
}
</style>
