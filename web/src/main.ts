import { createApp } from 'vue'
import '@fontsource/jetbrains-mono/latin-400.css'

import App from './App.vue'
import router from './router'
import './styles.css'

createApp(App).use(router).mount('#app')
