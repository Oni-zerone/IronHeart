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
      notify('Collegamento creato: il primo ping parte subito.')
    } else if (editing.value) {
      replace(await updateEntanglement(editing.value.id, replaceKey.value ? { ...rest, key_id, secret } : rest))
      notify('Collegamento aggiornato.')
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
    notify(updated.last_error ? `Non consegnato: ${updated.last_error}` : 'Ping consegnato.', updated.last_error ? 'error' : 'success')
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
  notify('Copiato negli appunti.')
}

function status(item: Entanglement): { color: string; text: string } {
  if (!item.enabled) return { color: 'warning', text: 'In pausa' }
  if (item.last_attempt_at === null) return { color: 'grey', text: 'In attesa' }
  if (item.last_error) return { color: 'error', text: 'Non consegnato' }
  return { color: 'success', text: 'Consegnato' }
}

onMounted(load)
</script>

<template>
  <v-container class="py-6" max-width="960">
    <div class="d-flex align-center flex-wrap ga-2 mb-4">
      <h1 class="text-h5 me-auto">Entanglement</h1>
      <v-btn :prepend-icon="mdiPlus" color="primary" variant="flat" @click="openNew">
        Nuovo collegamento
      </v-btn>
    </div>

    <v-card variant="tonal" class="mb-6">
      <v-card-text class="help">
        <p class="mb-2">
          Questa istanza manda un heartbeat ad altre istanze di IronHeart, così si controllano a
          vicenda: se questa si ferma, l'altra manda l'avviso.
        </p>
        <ol class="ms-5 mb-2">
          <li>Sull'altra istanza crea un'applicazione che rappresenta questa, e copia key ID e segreto.</li>
          <li>Qui crea un collegamento con l'indirizzo dell'altra istanza, la chiave e un intervallo più breve di quello dell'applicazione (per esempio la metà).</li>
          <li>Per il controllo reciproco, ripeti al contrario con l'indirizzo di questa istanza.</li>
        </ol>
        <div class="d-flex align-center flex-wrap ga-1">
          <span>Indirizzo di questa istanza:</span>
          <code>{{ instanceUrl }}</code>
          <v-btn :icon="mdiContentCopy" variant="text" size="small" aria-label="Copia l'indirizzo" @click="copy(instanceUrl)" />
        </div>
        <p class="text-medium-emphasis mt-2 mb-0">
          L'altra istanza risponde sempre allo stesso modo: qui vedi solo se il ping è stato
          consegnato. Se la chiave è sbagliata, lo segnala l'altra istanza con i tentativi non validi.
        </p>
      </v-card-text>
    </v-card>

    <v-progress-linear v-if="!items" indeterminate />
    <v-card v-else-if="items.length === 0" class="pa-6 text-center">Nessun collegamento.</v-card>
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
          <div>Ping ogni {{ formatDuration(item.interval_secs) }}</div>
          <div>
            Ultima consegna: {{ formatRelative(item.last_success_at) }}
            <span v-if="item.last_success_at">({{ formatDateTime(item.last_success_at) }})</span>
          </div>
          <div v-if="item.last_error" class="text-error">
            Ultimo tentativo {{ formatRelative(item.last_attempt_at) }}: {{ item.last_error }}
          </div>
        </v-card-text>
        <v-card-actions class="flex-wrap">
          <v-btn :prepend-icon="mdiSend" :loading="pinging === item.id" @click="pingNow(item)">Invia ora</v-btn>
          <v-btn variant="tonal" @click="toggle(item)">{{ item.enabled ? 'Metti in pausa' : 'Riprendi' }}</v-btn>
          <v-spacer />
          <v-btn :icon="mdiPencilOutline" aria-label="Modifica" @click="openEdit(item)" />
          <v-btn :icon="mdiDeleteOutline" aria-label="Elimina" color="error" @click="toDelete = item" />
        </v-card-actions>
      </v-card>
    </div>

    <v-dialog :model-value="editing !== null" max-width="640" :fullscreen="smAndDown" scrollable @update:model-value="editing = null">
      <v-card :title="editing === 'new' ? 'Nuovo collegamento' : 'Modifica collegamento'">
        <v-card-text class="pt-2">
          <v-alert v-if="formError" type="error" variant="tonal" class="mb-4">{{ formError }}</v-alert>
          <form class="d-flex flex-column ga-6" @submit.prevent="save">
            <section>
              <label class="form-title" for="ent-name">Nome</label>
              <v-text-field id="ent-name" v-model="form.name" maxlength="80" hide-details autofocus />
            </section>
            <section>
              <label class="form-title" for="ent-url">Indirizzo dell'altra istanza</label>
              <p class="form-help">Solo l'indirizzo, senza percorso. HTTPS obbligatorio, HTTP solo verso localhost.</p>
              <v-text-field id="ent-url" v-model="form.base_url" type="url" hide-details />
            </section>
            <section>
              <div class="form-title">Chiave</div>
              <p class="form-help">Key ID e segreto dell'applicazione creata sull'altra istanza.</p>
              <v-checkbox v-if="editing !== 'new'" v-model="replaceKey" label="Sostituisci la chiave" hide-details class="mb-2" />
              <div v-if="replaceKey" class="d-flex flex-column ga-3">
                <v-text-field v-model="form.key_id" label="Key ID" hide-details />
                <v-text-field v-model="form.secret" label="Segreto" type="password" autocomplete="off" hide-details />
              </div>
            </section>
            <section>
              <div class="form-title">Intervallo dei ping</div>
              <p class="form-help">
                Da 10 secondi a 1 giorno. Tienilo più breve dell'intervallo dell'applicazione sull'altra
                istanza, così un ping perso non basta a far scattare l'avviso.
              </p>
              <DurationField v-model="form.interval_secs" label="Intervallo dei ping" />
            </section>
            <v-switch v-model="form.enabled" label="Attivo" color="primary" hide-details />
          </form>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="editing = null">Annulla</v-btn>
          <v-btn :loading="saving" color="primary" variant="flat" @click="save">Salva</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>

    <v-dialog :model-value="toDelete !== null" max-width="480" @update:model-value="toDelete = null">
      <v-card v-if="toDelete" :title="`Eliminare «${toDelete.name}»?`">
        <v-card-text>
          Questa istanza smette di mandare ping: se l'applicazione sull'altra istanza è attiva,
          tra poco risulterà ferma.
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="toDelete = null">Annulla</v-btn>
          <v-btn color="error" variant="flat" @click="confirmDelete">Elimina</v-btn>
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
