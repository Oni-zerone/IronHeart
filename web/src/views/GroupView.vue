<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import {
  deleteGroup,
  getGroup,
  inviteToGroup,
  leaveGroup,
  removeMember,
  renameGroup,
  setMemberRole,
  type GroupDetail,
  type GroupMember,
  type GroupRole,
} from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'
import AsciiDialog from '@/components/AsciiDialog.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { GROUP_ROLE_LABEL, GROUP_ROLE_OPTIONS, formatDateTime } from '@/format'
import { currentUser, errorText, loadSession, notify } from '@/session'

const props = defineProps<{ id: string }>()
const router = useRouter()

const group = ref<GroupDetail | null>(null)
const busy = ref(false)

const isInstanceAdmin = computed(() => currentUser.value?.role === 'admin')
/** Instance admin and admin of this group: unknown addresses become instance invites. */
const invitesNewPeople = computed(() => isInstanceAdmin.value && group.value?.role === 'admin')

const inviteEmail = ref('')
const inviteRole = ref<GroupRole>('member')
const inviting = ref(false)
const inviteError = ref('')
const inviteResult = ref('')

const renaming = ref(false)
const newName = ref('')
const renameError = ref('')
const confirm = ref<'leave' | 'delete' | null>(null)
const toRemove = ref<GroupMember | null>(null)

async function load() {
  try {
    group.value = await getGroup(props.id)
  } catch (err) {
    notify(errorText(err), 'error')
    router.replace({ name: 'groups' })
  }
}

async function run(action: () => Promise<GroupDetail>, done: string) {
  busy.value = true
  try {
    group.value = await action()
    notify(done)
    await loadSession()
  } catch (err) {
    notify(errorText(err), 'error')
    await load()
  } finally {
    busy.value = false
  }
}

async function invite() {
  inviting.value = true
  inviteError.value = ''
  inviteResult.value = ''
  const email = inviteEmail.value.trim().toLowerCase()
  try {
    const { result } = await inviteToGroup(props.id, email, inviteRole.value)
    inviteResult.value =
      result === 'instance_invite'
        ? `Invited to IronHeart and to ${group.value?.name}: ${email} joins at the first sign-in.`
        : 'Invitation sent.'
    inviteEmail.value = ''
  } catch (err) {
    inviteError.value = errorText(err)
  } finally {
    inviting.value = false
  }
}

function openRename() {
  newName.value = group.value?.name ?? ''
  renameError.value = ''
  renaming.value = true
}

async function submitRename() {
  busy.value = true
  renameError.value = ''
  try {
    group.value = await renameGroup(props.id, newName.value)
    renaming.value = false
    notify('Group renamed.')
    await loadSession()
  } catch (err) {
    renameError.value = errorText(err)
  } finally {
    busy.value = false
  }
}

function changeRole(member: GroupMember, role: string) {
  run(() => setMemberRole(props.id, member.user_id, role as GroupRole), 'Role updated.')
}

async function confirmRemove() {
  const member = toRemove.value
  if (!member) return
  await run(() => removeMember(props.id, member.user_id), `${member.email} removed from the group.`)
  toRemove.value = null
}

async function confirmAction() {
  busy.value = true
  try {
    if (confirm.value === 'leave') {
      await leaveGroup(props.id)
      notify(`You left ${group.value?.name}.`)
    } else if (confirm.value === 'delete') {
      await deleteGroup(props.id)
      notify('Group deleted.')
    }
    confirm.value = null
    await loadSession()
    router.replace({ name: 'groups' })
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
    <RouterLink class="back" :to="{ name: 'groups' }">&lt;- groups</RouterLink>
    <p v-if="!group" class="dim"><LoadingSpinner />loading</p>
    <template v-else>
      <div class="page-head">
        <h1>{{ group.name }}</h1>
        <span class="dim">
          you are
          <span :class="{ acc: group.role === 'admin' }">{{ group.role ? GROUP_ROLE_LABEL[group.role] : 'not a member' }}</span>
        </span>
        <button v-if="group.can_manage" class="btn plain" type="button" @click="openRename">rename</button>
        <button v-if="group.role" class="btn plain" type="button" @click="confirm = 'leave'">leave</button>
        <button v-if="isInstanceAdmin" class="btn danger" type="button" @click="confirm = 'delete'">delete group</button>
      </div>

      <AsciiBox v-if="group.can_manage" title="invite">
        <div class="form">
          <p v-if="invitesNewPeople">
            Invite an existing user, or a new person: as instance admin your invitation to an unknown address becomes
            an IronHeart invite with {{ group.name }} attached.
          </p>
          <p v-else>
            Invite an existing user of this instance. They get an email and accept or reject the invitation from
            their dashboard.
          </p>
          <div v-if="inviteError" class="notice">
            <span class="err">!!</span>
            <p>{{ inviteError }}</p>
          </div>
          <form class="row" @submit.prevent="invite">
            <input
              v-model="inviteEmail"
              class="input grow email"
              type="email"
              placeholder="email to invite"
              aria-label="Email"
              spellcheck="false"
            />
            <span class="select">
              <select v-model="inviteRole" aria-label="Role in the group">
                <option v-for="o in GROUP_ROLE_OPTIONS" :key="o.value" :value="o.value">{{ o.title }}</option>
              </select>
            </span>
            <button class="btn" type="submit" :disabled="inviting || !inviteEmail.trim()">
              <LoadingSpinner v-if="inviting" />invite
            </button>
          </form>
          <p v-if="inviteResult"><span class="acc">&gt;</span> {{ inviteResult }}</p>
          <p v-if="invitesNewPeople" class="help">
            Existing accounts still have to accept. A new person gets one email: the IronHeart invite, mentioning
            the group and the role.
          </p>
          <p v-else class="help">For privacy the answer is always the same. Members appear here only once they accept.</p>
        </div>
      </AsciiBox>

      <AsciiBox title="members" :right="`${group.members.length}`">
        <div class="scroll-x">
          <div class="tbl members" role="table" aria-label="Members">
            <span class="th" role="columnheader">user</span>
            <span class="th" role="columnheader">role</span>
            <span class="th" role="columnheader">since</span>
            <span class="th" role="columnheader"><span class="sr-only">actions</span></span>
            <span class="rule" aria-hidden="true">{{ '-'.repeat(300) }}</span>
            <template v-for="member in group.members" :key="member.user_id">
              <span role="cell">
                <span>{{ member.name ?? member.email }}</span>
                <span v-if="member.user_id === currentUser?.id" class="dim"> (you)</span>
                <template v-if="member.name"><br /><span class="dim">{{ member.email }}</span></template>
              </span>
              <span role="cell">
                <span v-if="group.can_manage" class="select">
                  <select
                    :value="member.role"
                    :disabled="busy"
                    :aria-label="`Role of ${member.email}`"
                    @change="changeRole(member, ($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="o in GROUP_ROLE_OPTIONS" :key="o.value" :value="o.value">{{ o.title }}</option>
                  </select>
                </span>
                <span v-else :class="{ acc: member.role === 'admin' }">{{ GROUP_ROLE_LABEL[member.role] }}</span>
              </span>
              <span class="dim" role="cell">{{ formatDateTime(member.created_at) }}</span>
              <span role="cell">
                <button
                  v-if="group.can_manage && member.user_id !== currentUser?.id"
                  class="link-btn danger"
                  type="button"
                  :aria-label="`Remove ${member.email}`"
                  @click="toRemove = member"
                >[remove]</button>
              </span>
            </template>
          </div>
        </div>
      </AsciiBox>

      <p class="help">
        {{
          group.can_manage
            ? 'Pending invitations are not listed. The last group admin cannot leave or be demoted.'
            : 'Only group admins invite and remove members.'
        }}
      </p>
    </template>

    <AsciiDialog v-if="renaming" title="rename group" @close="renaming = false">
      <form class="form" @submit.prevent="submitRename">
        <div v-if="renameError" class="notice">
          <span class="err">!!</span>
          <p>{{ renameError }}</p>
        </div>
        <div class="field">
          <label class="field-label" for="group-rename">name</label>
          <input id="group-rename" v-model="newName" class="input" maxlength="80" spellcheck="false" autocomplete="off" />
        </div>
        <div class="actions">
          <span class="grow" />
          <button class="btn plain" type="button" @click="renaming = false">cancel</button>
          <button class="btn" type="submit" :disabled="busy || !newName.trim()"><LoadingSpinner v-if="busy" />rename</button>
        </div>
      </form>
    </AsciiDialog>

    <ConfirmDialog
      v-if="confirm === 'leave' && group"
      :title="`leave ${group.name}?`"
      action="leave"
      danger
      :busy="busy"
      @confirm="confirmAction"
      @cancel="confirm = null"
    >
      You stop seeing its applications and receiving their alerts. A group admin can invite you again.
    </ConfirmDialog>
    <ConfirmDialog
      v-if="confirm === 'delete' && group"
      :title="`delete ${group.name}?`"
      action="delete"
      danger
      :busy="busy"
      @confirm="confirmAction"
      @cancel="confirm = null"
    >
      The members lose access to the group.
    </ConfirmDialog>
    <ConfirmDialog
      v-if="toRemove"
      :title="`remove ${toRemove.email}?`"
      action="remove"
      danger
      :busy="busy"
      @confirm="confirmRemove"
      @cancel="toRemove = null"
    >
      They stop seeing the group's applications and receiving their alerts.
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.back:hover,
.back:focus-visible {
  color: var(--accent);
  outline: none;
}
.email {
  flex-basis: 28ch;
}
.members {
  grid-template-columns: minmax(28ch, auto) auto auto auto;
}
</style>
