<script setup lang="ts">
// Groups of a user in a table cell: "ops (admin), billing", "-" when none.
import type { GroupTag } from '@/api'

defineProps<{ tags: GroupTag[] }>()
</script>

<template>
  <span v-if="tags.length === 0" class="dim">-</span>
  <span v-else class="tags">
    <span v-for="(tag, i) in tags" :key="tag.name"
      >{{ tag.name }}<span v-if="tag.role === 'admin'" class="dim"> (admin)</span
      ><template v-if="i < tags.length - 1">, </template></span
    >
  </span>
</template>

<style scoped>
.tags {
  white-space: normal;
}
</style>
