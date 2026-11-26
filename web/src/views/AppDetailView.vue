<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { mdiArrowLeft, mdiContentCopy, mdiDeleteOutline, mdiKeyChange } from '@mdi/js'

import {
  deleteApp,
  getApp,
  rotateKey,
  updateApp,
  type AppDetail,
  type AppInput,
  type Credentials,
} from '@/api'
import AppForm from '@/components/AppForm.vue'
import CredentialsDialog from '@/components/CredentialsDialog.vue'
import { STATE_COLOR, STATE_LABEL, formatDateTime, formatDuration, formatRelative } from '@/format'
import { currentUser, errorText, notify } from '@/session'
import { shellSnippet } from '@/snippet'

const props = defineProps<{ id: string }>()
const router = useRouter()

const app = ref<AppDetail | null>(null)
const form = ref<AppInput | null>(null)
const saving = ref(false)
const saveError = ref('')
const credentials = ref<Credentials | null>(null)
const confirm = ref<'rotate' | 'delete' | null>(null)
const busy = ref(false)

const snippet = computed(() => (app.value ? shellSnippet(window.location.origin, app.value.key_id) : ''))
const recipientsText = computed(() =>
  app.value?.recipients.length ? app.value.recipients.join(', ') : `${currentUser.value?.email} (account)`,
)

function setApp(detail: AppDetail) {
  app.value = detail
  form.value = {
    name: detail.name,
    interval_secs: detail.interval_secs,
    grace_secs: detail.grace_secs,
    recipients: [...detail.recipients],
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

async function save() {
  if (!form.value) return
  saving.value = true
  saveError.value = ''
  try {
    setApp(await updateApp(props.id, form.value))
    notify('Modifiche salvate.')
  } catch (err) {
    saveError.value = errorText(err)
  } finally {
    saving.value = false
  }
}

async function togglePause() {
  if (!app.value) return
  try {
    setApp(await updateApp(props.id, { paused: app.value.state !== 'paused' }))
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

async function confirmAction() {
  busy.value = true
  try {
    if (confirm.value === 'rotate') {
      const result = await rotateKey(props.id)
      setApp(result.app)
      credentials.value = result.credentials
    } else if (confirm.value === 'delete') {
      await deleteApp(props.id)
      notify('Applicazione eliminata.')
      router.replace({ name: 'apps' })
    }
    confirm.value = null
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    busy.value = false
  }
}

async function copy(text: string) {
  await navigator.clipboard.writeText(text)
  notify('Copiato negli appunti.')
}

function ipsText(ips: Record<string, number>, others: number): string {
  const list = Object.entries(ips)
    .sort((a, b) => b[1] - a[1])
    .map(([ip, n]) => `${ip} (${n})`)
  if (others > 0) list.push(`altri ${others}`)
  return list.join(', ')
}

onMounted(load)
</script>

<template>
  <v-container class="py-6" max-width="960">
    <v-btn :prepend-icon="mdiArrowLeft" :to="{ name: 'apps' }" variant="text" class="mb-2">
      Applicazioni
    </v-btn>
    <v-progress-linear v-if="!app || !form" indeterminate />
    <template v-else>
      <div class="d-flex align-center flex-wrap ga-2 mb-4">
        <h1 class="text-h5 me-auto">{{ app.name }}</h1>
        <v-chip :color="STATE_COLOR[app.state]" variant="flat">{{ STATE_LABEL[app.state] }}</v-chip>
      </div>

      <v-row>
        <v-col cols="12" md="5">
          <v-card title="Stato">
            <v-list density="compact">
              <v-list-item title="Ultimo segnale" :subtitle="`${formatRelative(app.last_seen_at)} · ${formatDateTime(app.last_seen_at)}`" />
              <v-list-item
                v-if="app.state === 'up'"
                title="Scadenza"
                :subtitle="`${formatRelative(app.next_deadline_at)} · ${formatDateTime(app.next_deadline_at)}`"
              />
              <v-list-item
                v-if="app.state === 'down'"
                title="Ferma da"
                :subtitle="formatDateTime(app.down_since)"
              />
              <v-list-item title="Controllo" :subtitle="`Ogni ${formatDuration(app.interval_secs)}, tolleranza ${formatDuration(app.grace_secs)}`" />
              <v-list-item title="Avvisi a" :subtitle="recipientsText" />
              <v-list-item title="Key ID" :subtitle="app.key_id">
                <template #append>
                  <v-btn :icon="mdiContentCopy" variant="text" size="small" @click="copy(app.key_id)" />
                </template>
              </v-list-item>
            </v-list>
            <v-card-actions class="flex-wrap">
              <v-btn variant="tonal" @click="togglePause">
                {{ app.state === 'paused' ? 'Riprendi il controllo' : 'Metti in pausa' }}
              </v-btn>
              <v-btn :prepend-icon="mdiKeyChange" @click="confirm = 'rotate'">Nuova chiave</v-btn>
            </v-card-actions>
          </v-card>
        </v-col>

        <v-col cols="12" md="7">
          <v-card title="Impostazioni">
            <v-card-text>
              <v-alert v-if="saveError" type="error" variant="tonal" class="mb-4">{{ saveError }}</v-alert>
              <AppForm v-model="form" />
            </v-card-text>
            <v-card-actions>
              <v-btn :prepend-icon="mdiDeleteOutline" color="error" @click="confirm = 'delete'">Elimina</v-btn>
              <v-spacer />
              <v-btn :loading="saving" color="primary" variant="flat" @click="save">Salva</v-btn>
            </v-card-actions>
          </v-card>
        </v-col>

        <v-col cols="12">
          <v-card title="Tentativi non validi" subtitle="Ultime 24 ore">
            <v-card-text v-if="app.failures.length === 0">Nessun tentativo non valido.</v-card-text>
            <v-table v-else density="compact">
              <thead>
                <tr>
                  <th>Periodo</th>
                  <th class="text-end">Firma errata</th>
                  <th class="text-end">Ripetuti</th>
                  <th class="text-end">Orologio</th>
                  <th>Provenienza</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="bucket in app.failures" :key="bucket.flushed_at">
                  <td>{{ formatDateTime(bucket.period_start) }}</td>
                  <td class="text-end">{{ bucket.bad_signature }}</td>
                  <td class="text-end">{{ bucket.replay }}</td>
                  <td class="text-end">{{ bucket.clock_skew }}</td>
                  <td>{{ ipsText(bucket.ips, bucket.other_ips) }}</td>
                </tr>
              </tbody>
            </v-table>
          </v-card>
        </v-col>

        <v-col cols="12">
          <v-card title="Integrazione">
            <v-card-text>
              <p class="mb-2">
                Ogni richiesta è firmata con HMAC-SHA256 e un timestamp sempre crescente; il segreto non
                viaggia mai. Il servizio risponde sempre 204: per verificare che funzioni, guarda qui
                l'ultimo segnale e i tentativi non validi.
              </p>
              <pre class="snippet">{{ snippet }}</pre>
            </v-card-text>
          </v-card>
        </v-col>
      </v-row>
    </template>

    <v-dialog :model-value="confirm !== null" max-width="480" @update:model-value="confirm = null">
      <v-card :title="confirm === 'rotate' ? 'Generare una nuova chiave?' : 'Eliminare l\'applicazione?'">
        <v-card-text>
          {{
            confirm === 'rotate'
              ? 'La chiave attuale smette subito di funzionare: dovrai aggiornare l\'applicazione con quella nuova.'
              : 'L\'applicazione, la sua chiave e lo storico dei tentativi vengono cancellati.'
          }}
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="confirm = null">Annulla</v-btn>
          <v-btn :loading="busy" :color="confirm === 'delete' ? 'error' : 'primary'" variant="flat" @click="confirmAction">
            {{ confirm === 'rotate' ? 'Genera' : 'Elimina' }}
          </v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>

    <CredentialsDialog :credentials="credentials" :app-name="app?.name ?? ''" @close="credentials = null" />
  </v-container>
</template>

<style scoped>
.snippet {
  font-size: 0.8rem;
  overflow-x: auto;
  padding: 12px;
  border-radius: 4px;
  background: rgba(var(--v-theme-on-surface), 0.06);
}
</style>
