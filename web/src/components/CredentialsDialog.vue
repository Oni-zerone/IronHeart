<script setup lang="ts">
import { computed, ref } from 'vue'

import type { Credentials } from '@/api'
import { copy } from '@/session'
import { highlightSnippet, shellSnippet } from '@/snippet'
import AsciiDialog from './AsciiDialog.vue'

const props = defineProps<{ credentials: Credentials; appName: string }>()
const emit = defineEmits<{ close: [] }>()

const saved = ref(false)
const snippet = computed(() => shellSnippet(window.location.origin, props.credentials.key_id, props.credentials.secret))
</script>

<template>
  <AsciiDialog :title="`key for ${appName}`" :width="88" persistent>
    <div class="form">
      <div class="notice">
        <span class="err">[!]</span>
        <p>
          The secret is shown only now: save it in the application's configuration. If you lose it, you
          will have to generate a new key.
        </p>
      </div>
      <dl class="kv">
        <dt>key id</dt>
        <dd>
          <span class="acc">{{ credentials.key_id }}</span> <button class="link-btn" type="button" @click="copy(credentials.key_id)">[copy]</button>
        </dd>
        <dt>secret</dt>
        <dd>
          <span class="hi">{{ credentials.secret }}</span> <button class="link-btn" type="button" @click="copy(credentials.secret)">[copy]</button>
        </dd>
      </dl>
      <div class="field">
        <div class="row">
          <span class="field-label grow">example for a cron job</span>
          <button class="link-btn" type="button" @click="copy(snippet)">[copy]</button>
        </div>
        <!-- eslint-disable-next-line vue/no-v-html -- escaped in highlightSnippet -->
        <pre class="code" v-html="highlightSnippet(snippet)"></pre>
      </div>
      <div class="actions">
        <button class="check" type="button" :aria-pressed="saved" @click="saved = !saved">
          <span class="mark">[{{ saved ? 'x' : ' ' }}]</span> I have saved the secret
        </button>
        <span class="grow" />
        <button class="btn" type="button" :disabled="!saved" @click="(saved = false), emit('close')">close</button>
      </div>
    </div>
  </AsciiDialog>
</template>
