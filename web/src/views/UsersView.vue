<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { mdiAccountPlusOutline, mdiDeleteOutline, mdiEmailRemoveOutline } from '@mdi/js'

import {
  inviteUser,
  listUsers,
  removeUser,
  revokeInvite,
  setUserRole,
  type Role,
  type UserSummary,
  type UsersOverview,
} from '@/api'
import { ROLE_LABEL, ROLE_OPTIONS, formatDateTime } from '@/format'
import { currentUser, errorText, notify } from '@/session'

const overview = ref<UsersOverview | null>(null)
const inviteEmail = ref('')
const inviteRole = ref<Role>('user')
const inviting = ref(false)
const inviteError = ref('')
const toRemove = ref<UserSummary | null>(null)
const busy = ref(false)

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

async function confirmRemove() {
  const user = toRemove.value
  if (!user) return
  await run(() => removeUser(user.id), `${user.email} removed.`)
  toRemove.value = null
}

onMounted(() => run(listUsers))
</script>

<template>
  <v-container class="py-6" max-width="960">
    <h1 class="text-h5 mb-4">Users</h1>

    <v-card title="Invite" class="mb-6">
      <v-card-text>
        <p class="mb-4">
          Access is by invitation only. The person receives an email and, the first time they sign in
          with the Google account for that address, gets the chosen role. Users can manage only their
          own applications; administrators can also invite and remove users.
        </p>
        <v-alert v-if="inviteError" type="error" variant="tonal" class="mb-4">{{ inviteError }}</v-alert>
        <form class="d-flex flex-wrap ga-3" @submit.prevent="invite">
          <v-text-field
            v-model="inviteEmail"
            label="Email"
            type="email"
            hide-details
            class="invite-email"
          />
          <v-select
            v-model="inviteRole"
            :items="ROLE_OPTIONS"
            label="Role"
            hide-details
            class="invite-role"
          />
          <v-btn
            :prepend-icon="mdiAccountPlusOutline"
            :loading="inviting"
            :disabled="!inviteEmail.trim()"
            type="submit"
            color="primary"
            variant="flat"
            size="large"
          >
            Invite
          </v-btn>
        </form>
      </v-card-text>
    </v-card>

    <v-progress-linear v-if="!overview" indeterminate />
    <template v-else>
      <v-card title="Account" class="mb-6">
        <v-table>
          <thead>
            <tr>
              <th>User</th>
              <th>Role</th>
              <th class="text-end">Apps</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="user in overview.users" :key="user.id">
              <td>
                <div>{{ user.name ?? user.email }}</div>
                <div v-if="user.name" class="text-body-2 text-medium-emphasis">{{ user.email }}</div>
              </td>
              <td class="role-cell">
                <v-chip v-if="user.is_master" size="small" color="primary" variant="tonal">
                  Master
                </v-chip>
                <span v-else-if="locked(user)">{{ ROLE_LABEL[user.role] }} (you)</span>
                <v-select
                  v-else
                  :model-value="user.role"
                  :items="ROLE_OPTIONS"
                  :disabled="busy"
                  :aria-label="`Role of ${user.email}`"
                  density="compact"
                  variant="plain"
                  hide-details
                  @update:model-value="(role: Role) => run(() => setUserRole(user.id, role), 'Role updated.')"
                />
              </td>
              <td class="text-end">{{ user.app_count }}</td>
              <td class="text-end">
                <v-btn
                  v-if="!locked(user)"
                  :icon="mdiDeleteOutline"
                  :aria-label="`Remove ${user.email}`"
                  variant="text"
                  color="error"
                  size="small"
                  @click="toRemove = user"
                />
              </td>
            </tr>
          </tbody>
        </v-table>
      </v-card>

      <v-card title="Pending invites">
        <v-card-text v-if="overview.invites.length === 0">No pending invites.</v-card-text>
        <v-table v-else>
          <thead>
            <tr>
              <th>Email</th>
              <th>Role</th>
              <th>Invited on</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="invite in overview.invites" :key="invite.email">
              <td>{{ invite.email }}</td>
              <td>{{ ROLE_LABEL[invite.role] }}</td>
              <td>{{ formatDateTime(invite.created_at) }}</td>
              <td class="text-end">
                <v-btn
                  :icon="mdiEmailRemoveOutline"
                  :aria-label="`Revoke the invite for ${invite.email}`"
                  :disabled="busy"
                  variant="text"
                  size="small"
                  @click="run(() => revokeInvite(invite.email), 'Invite revoked.')"
                />
              </td>
            </tr>
          </tbody>
        </v-table>
      </v-card>
    </template>

    <v-dialog :model-value="toRemove !== null" max-width="480" @update:model-value="toRemove = null">
      <v-card v-if="toRemove" :title="`Remove ${toRemove.email}?`">
        <v-card-text>
          They lose access immediately.
          <strong v-if="toRemove.app_count > 0">
            Their {{ toRemove.app_count }} applications are deleted too, with keys and history.
          </strong>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn @click="toRemove = null">Cancel</v-btn>
          <v-btn :loading="busy" color="error" variant="flat" @click="confirmRemove">Remove</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>
  </v-container>
</template>

<style scoped>
.invite-email {
  flex: 1 1 260px;
}

.invite-role {
  flex: 0 0 200px;
}

.role-cell {
  min-width: 180px;
}
</style>
