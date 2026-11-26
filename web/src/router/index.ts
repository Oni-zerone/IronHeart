import { createRouter, createWebHistory } from 'vue-router'

import AppDetailView from '@/views/AppDetailView.vue'
import AppsView from '@/views/AppsView.vue'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/', redirect: { name: 'apps' } },
    { path: '/apps', name: 'apps', component: AppsView },
    { path: '/apps/:id', name: 'app', component: AppDetailView, props: true },
  ],
})

export default router
