<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'

import {
  inviteUser,
  listGroups,
  listUsers,
  removeUser,
  revokeInvite,
  setUserRole,
  type GroupSummary,
  type Role,
  type UserSummary,
  type UsersOverview,
} from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import GroupTags from '@/components/GroupTags.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { ROLE_LABEL, ROLE_OPTIONS, formatDateTime } from '@/format'
import { currentUser, errorText, notify } from '@/session'

const overview = ref<UsersOverview | null>(null)
const inviteEmail = ref('')
const inviteRole = ref<Role>('user')
const inviting = ref(false)
const inviteError = ref('')
const toRemove = ref<UserSummary | null>(null)
const busy = ref(false)
/** Removing a user with personal apps: move them into a group, or delete them. */
const keepApps = ref(true)
const groups = ref<GroupSummary[]>([])
const moveTo = ref('')

watch(toRemove, async (user) => {
  if (!user?.app_count) return
  keepApps.value = true
  try {
    groups.value = (await listGroups()).groups
    moveTo.value = groups.value[0]?.id ?? ''
    if (!moveTo.value) keepApps.value = false
  } catch (err) {
    notify(errorText(err), 'error')
  }
})

async function run(action: () => Promise<UsersOverview>, done?: string) {
  busy.value = true
  try {
    overview.value = await action()
    if (done) notify(done)
  } catch (err) {
    notify(errorText(err), 'error')
  } finally {
    busy.value = false
  }
}

async function invite() {
  inviting.value = true
  inviteError.value = ''
  try {
    overview.value = await inviteUser(inviteEmail.value, inviteRole.value)
    notify(`Invite sent to ${inviteEmail.value.trim().toLowerCase()}.`)
    inviteEmail.value = ''
    inviteRole.value = 'user'
  } catch (err) {
    inviteError.value = errorText(err)
  } finally {
    inviting.value = false
  }
}

function locked(user: UserSummary) {
  return user.is_master || user.id === currentUser.value?.id
}

function changeRole(user: UserSummary, role: string) {
  run(() => setUserRole(user.id, role as Role), 'Role updated.')
}

async function confirmRemove() {
  const user = toRemove.value
  if (!user) return
  const target = user.app_count > 0 && keepApps.value ? moveTo.value : undefined
  await run(() => removeUser(user.id, target), `${user.email} removed.`)
  toRemove.value = null
}

onMounted(() => run(listUsers))
</script>

<template>
  <div class="page">
    <div class="page-head"><h1>users</h1></div>

    <AsciiBox title="invite">
      <div class="form">
        <p>
          Access is by invitation only. The person receives an email and, at the first sign-in with the Google
          account of that address, gets the chosen role. Users manage only their own applications;
          admins can also invite and remove users.
        </p>
        <div v-if="inviteError" class="notice">
          <span class="err">!!</span>
          <p>{{ inviteError }}</p>
        </div>
        <form class="row" @submit.prevent="invite">
          <input
            id="invite-email"
            v-model="inviteEmail"
            class="input grow email"
            type="email"
            placeholder="email to invite"
            aria-label="Email"
            spellcheck="false"
          />
          <span class="select">
            <select id="invite-role" v-model="inviteRole" aria-label="Role">
              <option v-for="o in ROLE_OPTIONS" :key="o.value" :value="o.value">{{ o.title }}</option>
            </select>
          </span>
          <button class="btn" type="submit" :disabled="inviting || !inviteEmail.trim()">
            <LoadingSpinner v-if="inviting" />invite
          </button>
        </form>
      </div>
    </AsciiBox>

    <p v-if="!overview" class="dim"><LoadingSpinner />loading</p>
    <template v-else>
      <AsciiBox title="accounts" :right="`${overview.users.length}`">
        <div class="scroll-x">
          <div class="tbl accounts" role="table" aria-label="Accounts">
            <span class="th" role="columnheader">user</span>
            <span class="th" role="columnheader">role</span>
            <span class="th" role="columnheader">groups</span>
            <span class="th num" role="columnheader">apps</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="user in overview.users" :key="user.id">
              <span role="cell">
                <span>{{ user.name ?? user.email }}</span>
                <template v-if="user.name"><br /><span class="dim">{{ user.email }}</span></template>
              </span>
              <span role="cell">
                <span v-if="user.is_master" class="acc">[master]</span>
                <span v-else-if="locked(user)">{{ ROLE_LABEL[user.role] }} <span class="dim">(you)</span></span>
                <span v-else class="select">
                  <select
                    :value="user.role"
                    :disabled="busy"
                    :aria-label="`Role of ${user.email}`"
                    @change="changeRole(user, ($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="o in ROLE_OPTIONS" :key="o.value" :value="o.value">{{ o.title }}</option>
                  </select>
                </span>
              </span>
              <span role="cell"><GroupTags :tags="user.groups" /></span>
              <span class="num" role="cell">{{ user.app_count }}</span>
              <span role="cell">
                <button
                  v-if="!locked(user)"
                  class="link-btn danger"
                  type="button"
                  :aria-label="`Remove ${user.email}`"
                  @click="toRemove = user"
                >[remove]</button>
              </span>
            </template>
          </div>
        </div>
      </AsciiBox>

      <AsciiBox title="pending invites" :right="`${overview.invites.length}`">
        <p v-if="overview.invites.length === 0">No pending invites.</p>
        <div v-else class="scroll-x">
          <div class="tbl invites" role="table" aria-label="Pending invites">
            <span class="th" role="columnheader">email</span>
            <span class="th" role="columnheader">role</span>
            <span class="th" role="columnheader">groups</span>
            <span class="th" role="columnheader">invited</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="invite in overview.invites" :key="invite.email">
              <span role="cell">{{ invite.email }}</span>
              <span role="cell">{{ ROLE_LABEL[invite.role] }}</span>
              <span role="cell"><GroupTags :tags="invite.groups" /></span>
              <span class="dim" role="cell">{{ formatDateTime(invite.created_at) }}</span>
              <span role="cell">
                <button
                  class="link-btn"
                  type="button"
                  :aria-label="`Revoke the invite for ${invite.email}`"
                  :disabled="busy"
                  @click="run(() => revokeInvite(invite.email), 'Invite revoked.')"
                >[revoke]</button>
              </span>
            </template>
          </div>
        </div>
      </AsciiBox>
    </template>

    <ConfirmDialog
      v-if="toRemove && toRemove.app_count === 0"
      :title="`remove ${toRemove.email}?`"
      action="remove"
      danger
      :busy="busy"
      @confirm="confirmRemove"
      @cancel="toRemove = null"
    >
      They lose access immediately and leave their groups. Their pending transfer requests are cancelled.
    </ConfirmDialog>
    <AsciiDialog v-else-if="toRemove" :title="`remove ${toRemove.email}?`" @close="toRemove = null">
      <div class="form">
        <p>
          They lose access immediately and leave their groups. Group apps stay with their groups; their pending
          transfer requests are cancelled.
        </p>
        <div class="field" role="radiogroup" :aria-label="`Their ${toRemove.app_count} personal applications`">
          <span class="field-label">their {{ toRemove.app_count }} personal applications</span>
          <button
            class="check pre"
            type="button"
            role="radio"
            :aria-checked="keepApps"
            :disabled="groups.length === 0"
            @click="keepApps = true"
          ><span class="mark">{{ keepApps ? '(*)' : '( )' }}</span> move them to a group</button>
          <span v-if="keepApps" class="row indent">
            <span class="select">
              <select v-model="moveTo" aria-label="Group">
                <option v-for="group in groups" :key="group.id" :value="group.id">
                  {{ group.name }} · {{ group.app_count }}/100
                </option>
              </select>
            </span>
            <span class="dim">same keys and history</span>
          </span>
          <button
            class="check pre"
            type="button"
            role="radio"
            :aria-checked="!keepApps"
            @click="keepApps = false"
          ><span class="mark">{{ keepApps ? '( )' : '(*)' }}</span> delete them, with keys and history</button>
          <p v-if="groups.length === 0" class="help">There are no groups to move them to.</p>
        </div>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="toRemove = null">cancel</button>
          <button class="btn danger" type="button" :disabled="busy || (keepApps && !moveTo)" @click="confirmRemove">
            <LoadingSpinner v-if="busy" />remove
          </button>
        </div>
      </div>
    </AsciiDialog>
  </div>
</template>

<style scoped>
.indent {
  padding-left: 4ch;
}
.email {
  flex-basis: 28ch;
}
.accounts,
.invites {
  grid-template-columns: minmax(24ch, auto) auto minmax(16ch, auto) auto auto;
}
</style>
