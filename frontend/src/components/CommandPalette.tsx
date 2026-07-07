// Command palette — primary navigation surface. ⌘K / Ctrl+K from anywhere, or the
// top-bar button / project chip (via the shared `paletteOpen` signal).
//
// Context-aware: on an issue you get resolve/snooze/copy-event-id; on a project you get
// copy-DSN/settings/filter + any DOWN heartbeat monitors surfaced first as alerts;
// everywhere you get project switching + session controls.
// Keyboard: arrows nav, Enter run, Esc / click-outside close.

import { useLocation, useNavigate } from '@solidjs/router'
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js'
import { api } from '../api/client'
import { useAuth } from '../lib/auth-context'
import { paletteOpen, setPaletteOpen } from '../lib/palette'
import { Icon, Voice } from './primitives'

type Category = 'alert' | 'issue' | 'project' | 'nav' | 'session'
type Command = { id: string; category: Category; icon: string; label: string; hint?: string; accent?: boolean; danger?: boolean; run: () => void | Promise<void> }

const CAT_LABEL: Record<Category, string> = { alert: 'alerts', nav: 'navigate', issue: 'this issue', project: 'this project', session: 'session' }
const CAT_ORDER: Category[] = ['alert', 'issue', 'project', 'nav', 'session']

export function CommandPalette() {
  const [query, setQuery] = createSignal('')
  const [cursor, setCursor] = createSignal(0)
  const nav = useNavigate()
  const auth = useAuth()
  const location = useLocation()
  let inputRef: HTMLInputElement | undefined

  const close = () => {
    setPaletteOpen(false)
    setQuery('')
    setCursor(0)
  }
  const open = () => {
    setPaletteOpen(true)
    setQuery('')
    setCursor(0)
    queueMicrotask(() => inputRef?.focus())
  }

  const issueId = createMemo(() => {
    const m = location.pathname.match(/^\/issues\/(\d+)$/)
    return m ? Number(m[1]) : null
  })
  const projectId = createMemo(() => {
    const m = location.pathname.match(/^\/projects\/(\d+)(\/|$)/)
    return m ? Number(m[1]) : null
  })

  // Fetch context only while open, to keep the palette cheap when idle.
  const [issue] = createResource(() => (paletteOpen() ? issueId() : null), (id) => api.issues.get(id))
  const [events] = createResource(() => (paletteOpen() ? issueId() : null), (id) => api.issues.events(id, 1).then((page) => page.items))
  const [projects] = createResource(() => (paletteOpen() ? 'load' : null), () => api.projects.overview())
  const [monitors] = createResource(() => (paletteOpen() ? projectId() : null), (id) => api.heartbeats.list(id))

  const copy = async (text?: string) => {
    if (!text) return
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      /* clipboard blocked */
    }
  }

  const commands = createMemo<Command[]>(() => {
    const out: Command[] = []

    // context: alerts — DOWN heartbeat monitors for the current project, surfaced first
    const pidForAlerts = projectId()
    if (pidForAlerts != null) {
      for (const m of monitors() ?? []) {
        if (m.status !== 'down') continue
        out.push({
          id: `hb-${m.id}`, category: 'alert', icon: 'pulse',
          label: `${m.name} is down`, hint: 'view heartbeats', accent: true, danger: true,
          run: () => nav(`/projects/${pidForAlerts}/heartbeats`),
        })
      }
    }

    // context: issue
    const iid = issueId()
    if (iid != null) {
      const resolved = issue()?.status === 'resolved'
      out.push({
        id: 'toggle', category: 'issue', icon: 'check',
        label: resolved ? 'reopen issue' : 'mark issue fixed', hint: `#${iid}`,
        run: async () => { await api.issues.setStatus(iid, resolved ? 'unresolved' : 'resolved'); window.location.reload() },
      })
      out.push({ id: 'snooze1d', category: 'issue', icon: 'snooze', label: 'snooze · 1 day', run: () => api.issues.snooze(iid, '1d').then(() => window.location.reload()) })
      out.push({ id: 'snoozeforever', category: 'issue', icon: 'snooze', label: 'snooze · until next crash', run: () => api.issues.snooze(iid, 'forever').then(() => window.location.reload()) })
      const evId = events()?.[0]?.event_id
      if (evId) out.push({ id: 'copyev', category: 'issue', icon: 'copy', label: 'copy event id', hint: evId, run: () => copy(evId) })
    }

    // context: project
    const pid = projectId()
    if (pid != null) {
      out.push({ id: 'copydsn', category: 'project', icon: 'copy', label: 'copy DSN', run: async () => { const d = await api.projects.dsn(pid); await copy(d.dsn) } })
      out.push({ id: 'pissues', category: 'project', icon: 'bolt', label: 'view issues', run: () => nav(`/projects/${pid}/issues`) })
      if (auth.user()?.live_logs_enabled !== false)
        out.push({ id: 'plogs', category: 'project', icon: 'clock', label: 'view live logs', run: () => nav(`/projects/${pid}/logs`) })
      const downN = (monitors() ?? []).filter((m) => m.status === 'down').length
      out.push({ id: 'pheartbeats', category: 'project', icon: 'pulse', label: 'view heartbeats', hint: downN > 0 ? `${downN} down` : undefined, accent: downN > 0, run: () => nav(`/projects/${pid}/heartbeats`) })
      out.push({ id: 'psettings', category: 'project', icon: 'ext', label: 'project settings', run: () => nav(`/projects/${pid}/settings`) })
      out.push({ id: 'pfatals', category: 'project', icon: 'search', label: 'filter · unresolved fatals', run: () => nav(`/projects/${pid}/issues?q=level:fatal`) })
    }

    // nav: switch project
    for (const p of projects() ?? []) {
      out.push({
        id: `go-${p.id}`, category: 'nav', icon: 'chevronr', label: `go to ${p.name}`,
        hint: p.unresolved_count > 0 ? `${p.unresolved_count} unresolved` : 'clear', accent: p.unresolved_count > 0,
        run: () => nav(`/projects/${p.id}/issues`),
      })
    }
    out.push({ id: 'go-projects', category: 'nav', icon: 'arrow', label: 'go to projects', run: () => nav('/projects') })

    // session
    out.push({ id: 'tokens', category: 'session', icon: 'cmd', label: 'manage api tokens', run: () => nav('/tokens') })
    out.push({ id: 'logout', category: 'session', icon: 'x', label: 'logout', run: async () => { await auth.logout(); nav('/login', { replace: true }) } })
    return out
  })

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase()
    const all = commands()
    if (!q) return all
    return all.filter((c) => (c.label + ' ' + (c.hint ?? '')).toLowerCase().includes(q))
  })
  const groups = createMemo(() =>
    CAT_ORDER.map((cat) => ({ cat, items: filtered().filter((c) => c.category === cat) })).filter((g) => g.items.length),
  )
  const flat = createMemo(() => groups().flatMap((g) => g.items))

  createEffect(() => {
    const n = flat().length
    if (cursor() >= n) setCursor(Math.max(0, n - 1))
  })

  const run = (c: Command | undefined) => {
    if (!c) return
    close()
    setTimeout(() => void c.run(), 0)
  }

  const onKey = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault()
      if (paletteOpen()) close()
      else open()
      return
    }
    if (!paletteOpen()) return
    if (e.key === 'Escape') { e.preventDefault(); close() }
    else if (e.key === 'ArrowDown') { e.preventDefault(); setCursor((c) => Math.min(flat().length - 1, c + 1)) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setCursor((c) => Math.max(0, c - 1)) }
    else if (e.key === 'Enter') { e.preventDefault(); run(flat()[cursor()]) }
  }
  window.addEventListener('keydown', onKey)
  onCleanup(() => window.removeEventListener('keydown', onKey))

  // focus input whenever opened externally (top-bar button / project chip)
  createEffect(() => {
    if (paletteOpen()) queueMicrotask(() => inputRef?.focus())
  })

  return (
    <Show when={paletteOpen()}>
      <div
        class="cb-palette"
        onMouseDown={close}
        style={{
          position: 'fixed', inset: 0, 'z-index': 200, background: 'oklch(0.1 0.005 73 / 0.62)',
          'backdrop-filter': 'blur(3px)', '-webkit-backdrop-filter': 'blur(3px)',
          display: 'flex', 'justify-content': 'center', 'align-items': 'flex-start', 'padding-top': '12vh',
        }}
      >
        <div
          onMouseDown={(e) => e.stopPropagation()}
          class="card"
          style={{ width: 'min(92vw, 560px)', 'max-height': '70vh', display: 'flex', 'flex-direction': 'column', overflow: 'hidden', background: 'var(--bg-float)', 'box-shadow': 'var(--shadow-pop)' }}
        >
          <div style={{ display: 'flex', 'align-items': 'center', gap: '12px', padding: '14px 18px', 'border-bottom': '1px solid var(--line-soft)' }}>
            <Icon name="cmd" size={15} style={{ color: 'var(--text-faint)' }} />
            <input
              ref={inputRef}
              value={query()}
              onInput={(e) => { setQuery(e.currentTarget.value); setCursor(0) }}
              placeholder="// type a command…"
              class="mono"
              style={{ flex: '1', background: 'none', border: 'none', color: 'var(--text-hi)', 'font-size': '14px', outline: 'none' }}
            />
            <span class="kbd">esc</span>
          </div>

          <div style={{ 'overflow-y': 'auto', padding: '6px' }}>
            <Show when={flat().length > 0} fallback={<Voice style={{ padding: '18px 12px' }}>no command matches "{query()}"</Voice>}>
              <For each={groups()}>
                {(g) => (
                  <div style={{ 'margin-bottom': '4px' }}>
                    <div class="mono" style={{ 'font-size': '10px', 'letter-spacing': '0.08em', 'text-transform': 'uppercase', color: 'var(--text-faint)', padding: '8px 12px 4px' }}>{CAT_LABEL[g.cat]}</div>
                    <For each={g.items}>
                      {(c) => {
                        const idx = () => flat().indexOf(c)
                        const active = () => idx() === cursor()
                        return (
                          <div
                            onMouseEnter={() => setCursor(idx())}
                            onClick={() => run(c)}
                            style={{ position: 'relative', display: 'flex', 'align-items': 'center', gap: '12px', padding: '9px 12px', 'border-radius': '7px', cursor: 'pointer', background: active() ? 'oklch(1 0 0 / 0.05)' : 'transparent' }}
                          >
                            <Show when={active()}>
                              <span style={{ position: 'absolute', left: 0, top: '7px', bottom: '7px', width: '2.5px', 'border-radius': '999px', background: 'var(--accent)' }} />
                            </Show>
                            <Icon name={c.icon} size={14} style={{ color: c.danger ? 'var(--sev-error)' : active() ? 'var(--accent-ink)' : 'var(--text-faint)' }} />
                            <span class="mono" style={{ flex: '1', 'min-width': 0, 'font-size': '13px', color: active() ? 'var(--text-hi)' : 'var(--text)' }}>{c.label}</span>
                            <Show when={c.hint}>
                              <span class="mono" style={{ 'font-size': '11.5px', color: c.danger ? 'var(--sev-error)' : c.accent ? 'var(--accent-ink)' : 'var(--text-faint)' }}>{c.hint}</span>
                            </Show>
                            <Show when={active()}>
                              <span class="kbd"><Icon name="enter" size={11} /></span>
                            </Show>
                          </div>
                        )
                      }}
                    </For>
                  </div>
                )}
              </For>
            </Show>
          </div>

          <div style={{ display: 'flex', 'align-items': 'center', gap: '16px', padding: '9px 16px', 'border-top': '1px solid var(--line-soft)', 'font-size': '11.5px', color: 'var(--text-faint)' }}>
            <span style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}><span class="kbd">↑</span><span class="kbd">↓</span> nav</span>
            <span style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}><span class="kbd">↵</span> run</span>
            <span style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}><span class="kbd">esc</span> close</span>
            <div style={{ flex: '1' }} />
            <span class="mono">{flat().length} commands</span>
          </div>
        </div>
      </div>
    </Show>
  )
}
