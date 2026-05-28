// 2px vertical severity bar. Per docs/ui-design.md this is the load-bearing visual cue —
// no colored backgrounds, no pill tags.

const COLORS: Record<string, string> = {
  fatal: 'bg-crash',
  error: 'bg-crash',
  warning: 'bg-warn',
  info: 'bg-info',
  debug: 'bg-ink-400',
}

export function EdgeBar(props: { level: string | null | undefined; resolved?: boolean }) {
  if (props.resolved) {
    return <div class="w-[2px] self-stretch bg-ink-600" aria-hidden />
  }
  const cls = COLORS[props.level ?? 'error'] ?? 'bg-ink-400'
  return <div class={`w-[2px] self-stretch ${cls}`} aria-hidden />
}
