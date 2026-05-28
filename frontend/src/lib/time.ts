// Compact relative time formatter — "12m", "3h", "2d", "now".

export function relTime(iso: string | null | undefined): string {
  if (!iso) return '—'
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const diff = (Date.now() - t) / 1000
  if (diff < 5) return 'now'
  if (diff < 60) return `${Math.floor(diff)}s`
  if (diff < 3600) return `${Math.floor(diff / 60)}m`
  if (diff < 86_400) return `${Math.floor(diff / 3600)}h`
  if (diff < 86_400 * 30) return `${Math.floor(diff / 86_400)}d`
  if (diff < 86_400 * 365) return `${Math.floor(diff / 86_400 / 30)}mo`
  return `${Math.floor(diff / 86_400 / 365)}y`
}

export function absTime(iso: string | null | undefined): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return d.toISOString().replace('T', ' ').replace(/\.\d{3}Z$/, 'Z')
}
