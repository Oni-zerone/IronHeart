// Shared state of the character grid: glyph width, clock and spinner.
import { computed, onBeforeUnmount, onMounted, readonly, ref, type Ref } from 'vue'

const charWidth = ref(8.4)
let measured = false

/** Width in pixels of one monospace character, measured again once the web font has loaded. */
export function useCharWidth(): Readonly<Ref<number>> {
  if (!measured && typeof document !== 'undefined') {
    measured = true
    const measure = () => {
      const probe = document.createElement('span')
      probe.textContent = '0'.repeat(100)
      probe.setAttribute('aria-hidden', 'true')
      probe.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;font:var(--fs)/var(--lh) var(--mono)'
      document.body.appendChild(probe)
      const width = probe.getBoundingClientRect().width / 100
      probe.remove()
      if (width > 0) charWidth.value = width
    }
    measure()
    document.fonts?.ready.then(measure)
  }
  return readonly(charWidth)
}

/** Number of whole columns that fit in an element, updated on resize. */
export function useColumns(target: Ref<HTMLElement | null>, min = 10) {
  const char = useCharWidth()
  const width = ref(0)
  let observer: ResizeObserver | undefined
  onMounted(() => {
    if (!target.value) return
    width.value = target.value.getBoundingClientRect().width
    observer = new ResizeObserver(([entry]) => {
      if (entry) width.value = entry.contentRect.width
    })
    observer.observe(target.value)
  })
  onBeforeUnmount(() => observer?.disconnect())
  return computed(() => Math.max(min, Math.floor((width.value + 0.5) / char.value)))
}

export const reducedMotion =
  typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches

const now = ref(Date.now())
let clockUsers = 0
let clockTimer: ReturnType<typeof setInterval> | undefined

/** Current time, refreshed every second while at least one component uses it. */
export function useNow(): Readonly<Ref<number>> {
  onMounted(() => {
    if (clockUsers++ === 0) {
      now.value = Date.now()
      clockTimer = setInterval(() => (now.value = Date.now()), 1000)
    }
  })
  onBeforeUnmount(() => {
    if (--clockUsers === 0) clearInterval(clockTimer)
  })
  return readonly(now)
}

/** Repeats `fn` every `ms` while the page is visible (and once when it becomes visible again). */
export function usePolling(fn: () => void, ms: number) {
  let timer: ReturnType<typeof setInterval> | undefined
  const onVisibility = () => {
    if (document.visibilityState === 'visible') fn()
  }
  onMounted(() => {
    timer = setInterval(() => {
      if (document.visibilityState === 'visible') fn()
    }, ms)
    document.addEventListener('visibilitychange', onVisibility)
  })
  onBeforeUnmount(() => {
    clearInterval(timer)
    document.removeEventListener('visibilitychange', onVisibility)
  })
}

/** `n` repeated characters, never negative. */
export const repeat = (char: string, n: number) => char.repeat(Math.max(0, Math.floor(n)))
