<script setup lang="ts">
// "|/-\" spinner, for buttons that are waiting for the server.
import { onBeforeUnmount, ref } from 'vue'

const frames = '|/-\\'
const frame = ref(0)
const timer = setInterval(() => (frame.value = (frame.value + 1) % frames.length), 100)
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <span class="spinner" role="img" aria-label="Loading">{{ frames[frame] }}</span>
</template>

<style scoped>
/* The space after the spinner belongs to it, so a hidden spinner leaves no gap. */
.spinner {
  margin-right: 1ch;
}
</style>
