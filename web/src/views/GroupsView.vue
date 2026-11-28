<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { createGroup, leaveGroup, listGroups, listUsers, type GroupSummary, type UserSummary } from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { GROUP_ROLE_LABEL } from '@/format'
import { currentUser, errorText, loadSession, notify } from '@/session'

const router = useRouter()
const groups = ref<GroupSummary[] | null>(null)
const isAdmin = computed(() => currentUser.value?.role === 'admin')
const mine = computed(() => (groups.value ?? []).filter((g) => g.role !== null))
const others = computed(() => (groups.value ?? []).filter((g) => g.role === null))

const creating = ref(false)
const users = ref<UserSummary[]>([])
const newName = ref('')
const newAdmin = ref('')
const createError = ref('')
const saving = ref(false)
const toLeave = ref<GroupSummary | null>(null)
const busy = ref(false)

async function load() {
  try {
    groups.value = (await listGroups()).groups
  } catch (err) {
    notify(errorText(err), 'error')
  }
}

async function openCreate() {
  newName.value = ''
  newAdmin.value = currentUser.value?.id ?? ''
  createError.value = ''
  creating.value = true
  try {
    users.value = (await listUsers()).users
  } catch (err) {
    createError.value = errorText(err)
  }
}

async function submitCreate() {
  saving.value = true
  createError.value = ''
  try {
    const group = await createGroup(newName.value, newAdmin.value)
    creating.value = false
    await loadSession()
    router.push({ name: 'group', params: { id: group.id } })
  } catch (err) {
    createError.value = errorText(err)
  } finally {
    saving.value = false
  }
}

async function confirmLeave() {
  const group = toLeave.value
  if (!group) return
  busy.value = true
  try {
    await leaveGroup(group.id)
    notify(`You left ${group.name}.`)
    toLeave.value = null
    await Promise.all([load(), loadSession()])
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>groups</h1>
      <button v-if="isAdmin" class="btn" type="button" @click="openCreate">new group</button>
    </div>
    <p>
      Groups share applications: every member sees and manages them; group admins also delete them, move them out
      and manage the members.
    </p>

    <p v-if="!groups" class="dim"><LoadingSpinner />loading</p>
    <template v-else>
      <AsciiBox title="your groups" :right="`${mine.length}`">
        <p v-if="mine.length === 0">
          You are not in any group yet. Invitations show up on <RouterLink class="acc" :to="{ name: 'apps' }">/apps</RouterLink>.
        </p>
        <div v-else class="scroll-x">
          <div class="tbl groups" role="table" aria-label="Your groups">
            <span class="th" role="columnheader">group</span>
            <span class="th" role="columnheader">your role</span>
            <span class="th num" role="columnheader">members</span>
            <span class="th num" role="columnheader">apps</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="group in mine" :key="group.id">
              <span role="cell"><RouterLink class="name" :to="{ name: 'group', params: { id: group.id } }">{{ group.name }}</RouterLink></span>
              <span role="cell" :class="{ acc: group.role === 'admin' }">{{ GROUP_ROLE_LABEL[group.role!] }}</span>
              <span class="num" role="cell">{{ group.member_count }}</span>
              <span class="num" role="cell">{{ group.app_count }}</span>
              <span class="row" role="cell">
                <RouterLink class="link-btn" :to="{ name: 'group', params: { id: group.id } }">[open]</RouterLink>
                <button class="link-btn" type="button" :aria-label="`Leave ${group.name}`" @click="toLeave = group">[leave]</button>
              </span>
            </template>
          </div>
        </div>
      </AsciiBox>

      <AsciiBox v-if="isAdmin" title="other groups" :right="`${others.length}`">
        <p v-if="others.length === 0">No other groups.</p>
        <div v-else class="scroll-x">
          <div class="tbl groups" role="table" aria-label="Other groups">
            <span class="th" role="columnheader">group</span>
            <span class="th" role="columnheader">your role</span>
            <span class="th num" role="columnheader">members</span>
            <span class="th num" role="columnheader">apps</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="group in others" :key="group.id">
              <span role="cell"><RouterLink class="name" :to="{ name: 'group', params: { id: group.id } }">{{ group.name }}</RouterLink></span>
              <span class="dim" role="cell">-</span>
              <span class="num" role="cell">{{ group.member_count }}</span>
              <span class="num" role="cell">{{ group.app_count }}</span>
              <span role="cell"><RouterLink class="link-btn" :to="{ name: 'group', params: { id: group.id } }">[open]</RouterLink></span>
            </template>
          </div>
        </div>
      </AsciiBox>
      <p class="help">Invitations to a group show up on /apps, under pending. The last group admin cannot leave.</p>
    </template>

    <AsciiDialog v-if="creating" title="new group" @close="creating = false">
      <form class="form" @submit.prevent="submitCreate">
        <div v-if="createError" class="notice">
          <span class="err">!!</span>
          <p>{{ createError }}</p>
        </div>
        <div class="field">
          <label class="field-label" for="group-name">name</label>
          <input id="group-name" v-model="newName" class="input" maxlength="80" spellcheck="false" autocomplete="off" />
        </div>
        <div class="field">
          <label class="field-label" for="group-admin">first group admin</label>
          <span class="select">
            <select id="group-admin" v-model="newAdmin">
              <option v-for="user in users" :key="user.id" :value="user.id">
                {{ user.id === currentUser?.id ? `me · ${user.email}` : user.email }}
              </option>
            </select>
          </span>
          <p class="help">They manage the members; as instance admin you can manage any group.</p>
        </div>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="creating = false">cancel</button>
          <button class="btn" type="submit" :disabled="saving || !newName.trim() || !newAdmin">
            <LoadingSpinner v-if="saving" />create
          </button>
        </div>
      </form>
    </AsciiDialog>

    <ConfirmDialog
      v-if="toLeave"
      :title="`leave ${toLeave.name}?`"
      action="leave"
      danger
      :busy="busy"
      @confirm="confirmLeave"
      @cancel="toLeave = null"
    >
      You stop seeing its applications and receiving their alerts. A group admin can invite you again.
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.groups {
  grid-template-columns: minmax(24ch, auto) auto auto auto auto;
}
.name {
  color: var(--hi);
}
.name:hover,
.name:focus-visible {
  background: var(--hover);
  outline: none;
}
</style>
