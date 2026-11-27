<script setup lang="ts">
import AsciiDialog from './AsciiDialog.vue'
import LoadingSpinner from './LoadingSpinner.vue'

defineProps<{ title: string; action: string; danger?: boolean; busy?: boolean }>()
defineEmits<{ confirm: []; cancel: [] }>()
</script>

<template>
  <AsciiDialog :title="title" :width="60" @close="$emit('cancel')">
    <div class="form">
      <p><slot /></p>
      <div class="actions">
        <span class="grow" />
        <button class="btn plain" type="button" @click="$emit('cancel')">cancel</button>
        <button class="btn" :class="{ danger }" type="button" :disabled="busy" @click="$emit('confirm')">
          <LoadingSpinner v-if="busy" />{{ action }}
        </button>
      </div>
    </div>
  </AsciiDialog>
</template>
