<script setup lang="ts">
import { computed, onMounted } from 'vue'

import AsciiBox from '@/components/AsciiBox.vue'
import EcgLine from '@/components/EcgLine.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import LoginView from '@/views/LoginView.vue'
import { currentUser, loadSession, logout, toast } from '@/session'

const version = __APP_VERSION__

onMounted(loadSession)

/** Database indicator for the master: a local SQLite file is lost at every deploy. */
const database = computed(() => {
  const info = currentUser.value?.database
  if (!info) return null
  return info.backend === 'postgres'
    ? { label: 'postgres', short: '[db]', warn: false, hint: `External database: ${info.location}` }
    : {
        label: 'sqlite',
        short: '[!]',
        warn: true,
        hint: `File ${info.location}: without a volume the data is lost at every deploy. Set DATABASE_URL to use Postgres.`,
      }
})

/** Brevo API key indicator for the master: shown only when the key has a problem. */
const mailKey = computed(() => {
  const status = currentUser.value?.mail_key
  if (!status || status.state === 'ok') return null
  const when = new Date(status.checked_at).toLocaleString()
  return status.state === 'rejected'
    ? {
        label: 'mail key',
        hint: `Brevo refused the API key (${when}): alert emails cannot be sent and entanglement pings are held, so the other instances report this one as down. Checked again every hour. Set a new BREVO_API_KEY and restart. ${status.error ?? ''}`,
      }
    : {
        label: 'mail?',
        hint: `Could not check the Brevo API key (${when}), retrying every hour. ${status.error ?? ''}`,
      }
})
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <RouterLink class="brand" :to="{ name: 'apps' }">
        <span class="acc glow">&lt;3</span> <span class="hi">IRONHEART</span>
        <span class="dim version"> v{{ version }}</span>
      </RouterLink>
      <template v-if="currentUser">
        <nav class="links" aria-label="Main menu">
          <RouterLink :to="{ name: 'apps' }">/apps</RouterLink>
          <RouterLink v-if="currentUser.role === 'admin'" :to="{ name: 'users' }">/users</RouterLink>
          <RouterLink v-if="currentUser.is_master" :to="{ name: 'entanglement' }">/entanglement</RouterLink>
        </nav>
        <div class="tools">
          <span
            v-if="database"
            class="db"
            :class="{ warn: database.warn }"
            :title="database.hint"
            :aria-label="`Database: ${database.label}. ${database.hint}`"
          >
            <span class="db-long">{{ database.short }} {{ database.label }}</span>
            <span class="db-short">{{ database.short }}</span>
          </span>
          <span
            v-if="mailKey"
            class="db warn"
            :title="mailKey.hint"
            :aria-label="`Email: ${mailKey.hint}`"
          >
            <span class="db-long">!! {{ mailKey.label }}</span>
            <span class="db-short">!!</span>
          </span>
          <span class="dim email">{{ currentUser.email }}</span>
          <button class="btn plain" type="button" @click="logout">sign out</button>
        </div>
      </template>
    </header>
    <EcgLine class="ecg" />

    <main>
      <p v-if="currentUser === undefined" class="dim"><LoadingSpinner />loading</p>
      <LoginView v-else-if="currentUser === null" />
      <RouterView v-else />
    </main>

    <div v-if="toast" class="toast" role="status">
      <AsciiBox tight :hot="toast.error">
        <p>
          <span :class="toast.error ? 'hi' : 'acc'">{{ toast.error ? '!!' : '>' }}</span> {{ toast.text }}
        </p>
      </AsciiBox>
    </div>
  </div>
</template>

<style scoped>
.shell {
  max-width: 112ch;
  margin: 0 auto;
}

/* One row when it fits; on narrow screens the links get their own row instead of wrapping. */
.topbar {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  grid-template-areas: 'brand links tools';
  align-items: baseline;
  gap: 0 2ch;
  padding-block: var(--lh) 0;
  white-space: nowrap;
}
.brand {
  grid-area: brand;
}
.links {
  grid-area: links;
  justify-self: end;
  display: flex;
  gap: 0 2ch;
}
.tools {
  grid-area: tools;
  display: flex;
  gap: 0 2ch;
  align-items: baseline;
}
.links a:hover,
.links a:focus-visible {
  color: var(--accent);
  outline: none;
}
.links a.router-link-active {
  color: var(--bg);
  background: var(--accent);
}
.db {
  cursor: help;
  color: var(--fg);
}
.db.warn {
  color: var(--hi);
}
.db-short {
  display: none;
}
.email {
  display: none;
}
@media (min-width: 1100px) {
  .email {
    display: inline;
  }
}
@media (max-width: 760px) {
  .topbar {
    grid-template-columns: auto minmax(0, 1fr) auto;
    grid-template-areas:
      'brand . tools'
      'links links links';
  }
  .links {
    justify-self: start;
  }
}
@media (max-width: 600px) {
  .version,
  .db-long {
    display: none;
  }
  .db-short {
    display: inline;
  }
}

.ecg {
  margin-bottom: var(--lh);
}

.toast {
  position: fixed;
  z-index: 30;
  width: min(48ch, calc(100vw - 32px));
  right: max(16px, env(safe-area-inset-right, 0px));
  bottom: calc(var(--lh) + env(safe-area-inset-bottom, 0px));
}
.toast :deep(.box) {
  box-shadow: 1ch calc(var(--lh) / 2) 0 0 var(--faint);
}
</style>
