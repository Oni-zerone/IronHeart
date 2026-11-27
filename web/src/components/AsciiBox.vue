<script setup lang="ts">
// ASCII frame sized in whole columns:
//   +-[ TITLE ]--------------[ right ]-+
//   | content                          |
//   +----------------------------------+
// The side borders are a long column of "|" clipped to the height of the content,
// so the content must be a whole number of lines tall.
import { computed, ref } from 'vue'

import { repeat, useColumns } from '@/grid'

const props = defineProps<{
  title?: string
  /** Short text at the right of the top border (a count, a state). */
  right?: string
  /** No empty line above and below the content. */
  tight?: boolean
  /** Highlighted border: something needs attention. */
  hot?: boolean
}>()

const wrap = ref<HTMLElement | null>(null)
const cols = useColumns(wrap, 14)

const rightLength = computed(() => (props.right ? props.right.length + 5 : 0))
const shownTitle = computed(() => {
  if (!props.title) return ''
  const room = cols.value - 8 - rightLength.value
  return props.title.length > room ? props.title.slice(0, Math.max(1, room - 1)) + '~' : props.title
})
const fill = computed(() => {
  const fixed = 3 + (shownTitle.value ? shownTitle.value.length + 4 : 0) + rightLength.value
  return repeat('-', cols.value - fixed)
})
const bottom = computed(() => `+${repeat('-', cols.value - 2)}+`)
const side = '|\n'.repeat(1000)
</script>

<template>
  <section ref="wrap" class="box-wrap">
    <h2 v-if="title" class="sr-only">{{ title }}</h2>
    <div class="box" :class="{ hot }" :style="{ width: `${cols}ch` }">
      <div class="edge" aria-hidden="true">
        <template v-if="shownTitle">+-[ <span class="box-title">{{ shownTitle }}</span> ]</template>
        <template v-else>+-</template>{{ fill }}<template v-if="right">[ <span class="box-right">{{ right }}</span> ]-+</template>
        <template v-else>+</template>
      </div>
      <div class="body" :class="{ tight }">
        <span class="side left" aria-hidden="true">{{ side }}</span>
        <div class="content"><slot /></div>
        <span class="side right" aria-hidden="true">{{ side }}</span>
      </div>
      <div class="edge" aria-hidden="true">{{ bottom }}</div>
    </div>
  </section>
</template>

<style scoped>
.box-wrap {
  min-width: 0;
  overflow: hidden;
}
.box {
  background: var(--bg);
}
.edge {
  white-space: pre;
  overflow: hidden;
  color: var(--dim);
}
.box.hot .edge,
.box.hot .side {
  color: var(--accent);
}
.box-title {
  color: var(--accent);
  text-transform: uppercase;
}
.box.hot .box-title {
  color: var(--hi);
}
.box-right {
  color: var(--fg);
}
.body {
  position: relative;
  padding: var(--lh) 2ch;
}
.body.tight {
  padding-block: 0;
}
.side {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1ch;
  overflow: hidden;
  white-space: pre;
  color: var(--dim);
}
.side.left {
  left: 0;
}
.side.right {
  right: 0;
}
.content {
  min-width: 0;
}
</style>
