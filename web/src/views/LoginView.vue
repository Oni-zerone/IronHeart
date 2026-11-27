<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'

import { LOGIN_URL } from '@/api'
import AsciiBox from '@/components/AsciiBox.vue'

const route = useRoute()
const loginOutcome = computed(() => route.query.login)

// "IronHeart" in figlet's standard font.
const logo = [
  ' ___                 _   _                 _',
  '|_ _|_ __ ___  _ __ | | | | ___  __ _ _ __| |_',
  " | || '__/ _ \\| '_ \\| |_| |/ _ \\/ _` | '__| __|",
  ' | || | | (_) | | | |  _  |  __/ (_| | |  | |_',
  '|___|_|  \\___/|_| |_|_| |_|\\___|\\__,_|_|   \\__|',
].join('\n')
</script>

<template>
  <div class="login">
    <pre class="logo glow" role="img" aria-label="IronHeart">{{ logo }}</pre>
    <p class="dim">signed heartbeats · an alert when an app goes quiet</p>
    <AsciiBox class="login-box" title="sign in" right="google">
      <div class="form">
        <div v-if="loginOutcome === 'denied'" class="notice">
          <span class="hi">[!]</span>
          <p>
            This Google account is not enabled. Ask an administrator for an invite and sign in with the
            invited address.
          </p>
        </div>
        <div v-else-if="loginOutcome === 'failed'" class="notice">
          <span class="hi">!!</span>
          <p>Sign-in failed. Try again.</p>
        </div>
        <p>Access is by invitation only, with the Google account of the invited address.</p>
        <div class="actions">
          <a class="btn" :href="LOGIN_URL">sign in with google</a>
          <span class="acc blink" aria-hidden="true">_</span>
        </div>
      </div>
    </AsciiBox>
  </div>
</template>

<style scoped>
.login {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--lh);
  padding-block: calc(var(--lh) * 2) 0;
}
.logo {
  white-space: pre;
  color: var(--accent);
}
.login-box {
  width: 100%;
  max-width: 60ch;
}
@media (max-width: 440px) {
  .logo {
    font-size: 10.5px;
    line-height: 15px;
  }
}
</style>
