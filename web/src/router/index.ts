import { createRouter, createWebHistory } from 'vue-router'

import AppDetailView from '@/views/AppDetailView.vue'
import AppsView from '@/views/AppsView.vue'
import EntanglementView from '@/views/EntanglementView.vue'
import TelegramView from '@/views/TelegramView.vue'
import UsersView from '@/views/UsersView.vue'
import { currentUser } from '@/session'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/', redirect: { name: 'apps' } },
    { path: '/apps', name: 'apps', component: AppsView },
    { path: '/apps/:id', name: 'app', component: AppDetailView, props: true },
    { path: '/users', name: 'users', component: UsersView, meta: { admin: true } },
    { path: '/entanglement', name: 'entanglement', component: EntanglementView, meta: { master: true } },
    { path: '/telegram', name: 'telegram', component: TelegramView, meta: { telegram: true } },
  ],
})

// Restricted pages: the backend answers 403 (404 for Telegram without an active bot) anyway.
router.beforeEach((to) => {
  const user = currentUser.value
  if (!user) return
  if (
    (to.meta.admin && user.role !== 'admin') ||
    (to.meta.master && !user.is_master) ||
    (to.meta.telegram && !user.telegram)
  ) {
    return { name: 'apps' }
  }
})

export default router
