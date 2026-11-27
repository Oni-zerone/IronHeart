<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useDisplay } from 'vuetify'
import { mdiEmailFastOutline, mdiPlus, mdiShieldAlertOutline } from '@mdi/js'

import { createApp, listApps, sendTestEmail, type App, type AppInput, type Credentials } from '@/api'
import AppForm from '@/components/AppForm.vue'
import CredentialsDialog from '@/components/CredentialsDialog.vue'
import { STATE_COLOR, STATE_LABEL, formatDuration, formatRelative } from '@/format'
import { errorText, notify } from '@/session'

const router = useRouter()
const { smAndDown } = useDisplay()
const apps = ref<App[]>([])
const loading = ref(true)

const creating = ref(false)
const saving = ref(false)
const createError = ref('')
const form = ref<AppInput>(emptyForm())
const created = ref<{ id: string; name: string; credentials: Credentials } | null>(null)
const sendingTest = ref(false)

function emptyForm(): AppInput {
  return { name: '', interval_secs: 300, grace_secs: 60, recipients: [] }
}

async function load() {
  try {
    apps.value = await listApps()
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    loading.value = false
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

onMounted(load)
</script>

<template>
  <v-container class="py-6" max-width="960">
    <div class="d-flex align-center flex-wrap ga-2 mb-4">
      <h1 class="text-h5 me-auto">Applications</h1>
      <v-btn :prepend-icon="mdiEmailFastOutline" :loading="sendingTest" variant="text" @click="testEmail">
        Test email
      </v-btn>
      <v-btn :prepend-icon="mdiPlus" color="primary" variant="flat" @click="openCreate">
        New application
      </v-btn>
    </div>

    <v-progress-linear v-if="loading" indeterminate />
    <v-card v-else-if="apps.length === 0" variant="tonal" class="pa-6 text-center">
      No applications yet. Create one to get the key for the heartbeat.
    </v-card>
    <v-card v-else>
      <v-list lines="two">
        <v-list-item
          v-for="app in apps"
          :key="app.id"
          :to="{ name: 'app', params: { id: app.id } }"
          :title="app.name"
          :subtitle="`Every ${formatDuration(app.interval_secs)} · last signal ${formatRelative(app.last_seen_at)}`"
        >
          <template #append>
            <v-chip
              v-if="app.failures_24h > 0"
              :prepend-icon="mdiShieldAlertOutline"
              color="error"
              variant="text"
              size="small"
              class="me-2"
              title="Invalid attempts in the last 24 hours"
            >
              {{ app.failures_24h }}
            </v-chip>
            <v-chip :color="STATE_COLOR[app.state]" size="small" variant="flat">
              {{ STATE_LABEL[app.state] }}
            </v-chip>
          </template>
        </v-list-item>
      </v-list>
    </v-card>

    <v-dialog v-model="creating" max-width="640" :fullscreen="smAndDown" scrollable>
      <v-card title="New application">
        <v-card-text class="pt-2">
          <v-alert v-if="createError" type="error" variant="tonal" class="mb-4">{{ createError }}</v-alert>
          <AppForm v-model="form" />
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="creating = false">Cancel</v-btn>
          <v-btn :loading="saving" color="primary" variant="flat" @click="submitCreate">Create</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>

    <CredentialsDialog
      :credentials="created?.credentials ?? null"
      :app-name="created?.name ?? ''"
      @close="closeCredentials"
    />
  </v-container>
</template>
