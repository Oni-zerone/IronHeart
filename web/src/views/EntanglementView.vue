<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useDisplay } from 'vuetify'
import {
  mdiContentCopy,
  mdiDeleteOutline,
  mdiPencilOutline,
  mdiPlus,
  mdiSend,
} from '@mdi/js'

import {
  createEntanglement,
  deleteEntanglement,
  listEntanglements,
  pingEntanglement,
  updateEntanglement,
  type Entanglement,
  type EntanglementInput,
} from '@/api'
import DurationField from '@/components/DurationField.vue'
import { formatDateTime, formatDuration, formatRelative } from '@/format'
import { errorText, notify } from '@/session'

const { smAndDown } = useDisplay()
const items = ref<Entanglement[] | null>(null)
const instanceUrl = ref('')

const editing = ref<Entanglement | 'new' | null>(null)
const form = ref<EntanglementInput & { key_id: string; secret: string }>(emptyForm())
const replaceKey = ref(false)
const saving = ref(false)
const formError = ref('')
const toDelete = ref<Entanglement | null>(null)
const pinging = ref<string | null>(null)

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
      notify('Link created: the first ping is sent right away.')
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
    notify(updated.last_error ? `Not delivered: ${updated.last_error}` : 'Ping delivered.', updated.last_error ? 'error' : 'success')
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

async function copy(text: string) {
  await navigator.clipboard.writeText(text)
  notify('Copied to clipboard.')
}

function status(item: Entanglement): { color: string; text: string } {
  if (!item.enabled) return { color: 'warning', text: 'Paused' }
  if (item.last_attempt_at === null) return { color: 'grey', text: 'Waiting' }
  if (item.last_error) return { color: 'error', text: 'Not delivered' }
  return { color: 'success', text: 'Delivered' }
}

onMounted(load)
</script>

<template>
  <v-container class="py-6" max-width="960">
    <div class="d-flex align-center flex-wrap ga-2 mb-4">
      <h1 class="text-h5 me-auto">Entanglement</h1>
      <v-btn :prepend-icon="mdiPlus" color="primary" variant="flat" @click="openNew">
        New link
      </v-btn>
    </div>

    <v-card variant="tonal" class="mb-6">
      <v-card-text class="help">
        <p class="mb-2">
          This instance sends a heartbeat to other IronHeart instances, so they watch each other:
          if this one stops, the other one sends the alert.
        </p>
        <ol class="ms-5 mb-2">
          <li>On the other instance, create an application that represents this one, and copy its key ID and secret.</li>
          <li>Here, create a link with the other instance's address, the key and an interval shorter than the application's (for example half of it).</li>
          <li>For mutual monitoring, repeat the other way round with this instance's address.</li>
        </ol>
        <div class="d-flex align-center flex-wrap ga-1">
          <span>This instance's address:</span>
          <code>{{ instanceUrl }}</code>
          <v-btn :icon="mdiContentCopy" variant="text" size="small" aria-label="Copy the address" @click="copy(instanceUrl)" />
        </div>
        <p class="text-medium-emphasis mt-2 mb-0">
          The other instance always answers the same way: here you only see whether the ping was
          delivered. If the key is wrong, the other instance reports it as invalid attempts.
        </p>
      </v-card-text>
    </v-card>

    <v-progress-linear v-if="!items" indeterminate />
    <v-card v-else-if="items.length === 0" class="pa-6 text-center">No links.</v-card>
    <div v-else class="d-flex flex-column ga-4">
      <v-card v-for="item in items" :key="item.id">
        <v-card-item>
          <v-card-title class="title">{{ item.name }}</v-card-title>
          <v-card-subtitle class="subtitle">{{ item.base_url }} · key ID {{ item.key_id }}</v-card-subtitle>
          <template #append>
            <v-chip :color="status(item).color" variant="flat" size="small">{{ status(item).text }}</v-chip>
          </template>
        </v-card-item>
        <v-card-text>
          <div>Ping every {{ formatDuration(item.interval_secs) }}</div>
          <div>
            Last delivery: {{ formatRelative(item.last_success_at) }}
            <span v-if="item.last_success_at">({{ formatDateTime(item.last_success_at) }})</span>
          </div>
          <div v-if="item.last_error" class="text-error">
            Last attempt {{ formatRelative(item.last_attempt_at) }}: {{ item.last_error }}
          </div>
        </v-card-text>
        <v-card-actions class="flex-wrap">
          <v-btn :prepend-icon="mdiSend" :loading="pinging === item.id" @click="pingNow(item)">Send now</v-btn>
          <v-btn variant="tonal" @click="toggle(item)">{{ item.enabled ? 'Pause' : 'Resume' }}</v-btn>
          <v-spacer />
          <v-btn :icon="mdiPencilOutline" aria-label="Edit" @click="openEdit(item)" />
          <v-btn :icon="mdiDeleteOutline" aria-label="Delete" color="error" @click="toDelete = item" />
        </v-card-actions>
      </v-card>
    </div>

    <v-dialog :model-value="editing !== null" max-width="640" :fullscreen="smAndDown" scrollable @update:model-value="editing = null">
      <v-card :title="editing === 'new' ? 'New link' : 'Edit link'">
        <v-card-text class="pt-2">
          <v-alert v-if="formError" type="error" variant="tonal" class="mb-4">{{ formError }}</v-alert>
          <form class="d-flex flex-column ga-6" @submit.prevent="save">
            <section>
              <label class="form-title" for="ent-name">Name</label>
              <v-text-field id="ent-name" v-model="form.name" maxlength="80" hide-details autofocus />
            </section>
            <section>
              <label class="form-title" for="ent-url">Address of the other instance</label>
              <p class="form-help">Only the address, without a path. HTTPS is required, HTTP only for localhost.</p>
              <v-text-field id="ent-url" v-model="form.base_url" type="url" hide-details />
            </section>
            <section>
              <div class="form-title">Key</div>
              <p class="form-help">Key ID and secret of the application created on the other instance.</p>
              <v-checkbox v-if="editing !== 'new'" v-model="replaceKey" label="Replace the key" hide-details class="mb-2" />
              <div v-if="replaceKey" class="d-flex flex-column ga-3">
                <v-text-field v-model="form.key_id" label="Key ID" hide-details />
                <v-text-field v-model="form.secret" label="Secret" type="password" autocomplete="off" hide-details />
              </div>
            </section>
            <section>
              <div class="form-title">Ping interval</div>
              <p class="form-help">
                From 10 seconds to 1 day. Keep it shorter than the application's interval on the other
                instance, so a single lost ping is not enough to trigger an alert.
              </p>
              <DurationField v-model="form.interval_secs" label="Ping interval" />
            </section>
            <v-switch v-model="form.enabled" label="Active" color="primary" hide-details />
          </form>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="editing = null">Cancel</v-btn>
          <v-btn :loading="saving" color="primary" variant="flat" @click="save">Save</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>

    <v-dialog :model-value="toDelete !== null" max-width="480" @update:model-value="toDelete = null">
      <v-card v-if="toDelete" :title="`Delete “${toDelete.name}”?`">
        <v-card-text>
          This instance stops sending pings: if the application on the other instance is up,
          it will soon be reported as down.
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="toDelete = null">Cancel</v-btn>
          <v-btn color="error" variant="flat" @click="confirmDelete">Delete</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>
  </v-container>
</template>

<style scoped>
.title,
.subtitle {
  white-space: normal;
  overflow-wrap: anywhere;
}

.help {
  hyphens: manual;
}

.form-title {
  display: block;
  font-size: 1rem;
  font-weight: 500;
  margin-bottom: 4px;
}

.form-help {
  font-size: 0.875rem;
  opacity: var(--v-medium-emphasis-opacity);
  margin-bottom: 8px;
  hyphens: manual;
  overflow-wrap: break-word;
}
</style>
