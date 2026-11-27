<script setup lang="ts">
// Modal window drawn as an ASCII box with an offset shadow, like a TUI dialog.
// The parent shows it with v-if; Esc and a click outside close it unless it is persistent.
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

import AsciiBox from './AsciiBox.vue'

const props = withDefaults(defineProps<{ title: string; width?: number; persistent?: boolean }>(), {
  width: 64,
})
const emit = defineEmits<{ close: [] }>()

const panel = ref<HTMLElement | null>(null)
let previousFocus: Element | null = null

function close() {
  if (!props.persistent) emit('close')
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.stopPropagation()
    close()
  }
}

onMounted(async () => {
  previousFocus = document.activeElement
  document.addEventListener('keydown', onKey)
  document.body.style.overflow = 'hidden'
  await nextTick()
  const target =
    panel.value?.querySelector<HTMLElement>('[autofocus]') ??
    panel.value?.querySelector<HTMLElement>('input, select, button:not(:disabled)')
  target?.focus()
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKey)
  document.body.style.overflow = ''
  if (previousFocus instanceof HTMLElement) previousFocus.focus()
})
</script>

<template>
  <Teleport to="body">
    <div class="overlay" @click.self="close">
      <div
        ref="panel"
        class="panel"
        role="dialog"
        aria-modal="true"
        :aria-label="title"
        :style="{ maxWidth: `${width}ch` }"
      >
        <AsciiBox :title="title" :right="persistent ? undefined : 'esc'" hot>
          <slot />
        </AsciiBox>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 20;
  overflow-y: auto;
  background-color: rgb(0 0 0 / 82%);
  background-image: radial-gradient(var(--faint) 1px, transparent 1px);
  background-size: 4px 4px;
  padding: calc(var(--lh) * 3 + env(safe-area-inset-top, 0px)) max(16px, env(safe-area-inset-right, 0px))
    calc(var(--lh) * 3 + env(safe-area-inset-bottom, 0px)) max(16px, env(safe-area-inset-left, 0px));
}
.panel {
  margin: 0 auto;
}
.panel :deep(.box) {
  box-shadow: 2ch var(--lh) 0 0 var(--faint);
}
</style>
