// App-shell layout pieces: wordmark, centered page column, breadcrumb, project nav, footer hints.

import { A } from '@solidjs/router'
import { For, type JSX, Show } from 'solid-js'
import { useAuth } from '../lib/auth-context'

export function Wordmark(props: { small?: boolean; onClick?: () => void; href?: string }) {
  const mark = (
    <>
      <span style={{ position: 'relative', width: '16px', height: '16px', flex: 'none' }}>
        <span style={{ position: 'absolute', inset: 0, 'border-radius': '5px', background: 'var(--accent-grad)', opacity: 0.92 }} />
        <span style={{ position: 'absolute', inset: '3px', 'border-radius': '2px', background: 'var(--bg-base)' }} />
        <span style={{ position: 'absolute', inset: '6px', 'border-radius': '1px', background: 'var(--accent-grad)' }} />
      </span>
      <span class="mono" style={{ 'font-size': props.small ? '13px' : '14px', 'font-weight': 600, 'letter-spacing': '-0.02em', color: 'var(--text-hi)' }}>
        crashbox
      </span>
    </>
  )
  const style: JSX.CSSProperties = { display: 'flex', 'align-items': 'center', gap: '8px', background: 'none', border: 'none', cursor: 'pointer', padding: 0 }
  return props.href ? (
    <A href={props.href} style={style}>{mark}</A>
  ) : (
    <button onClick={props.onClick} style={style}>{mark}</button>
  )
}

export function Page(props: { children: JSX.Element; pad?: boolean }) {
  return (
    <main style={{ 'max-width': 'var(--maxw)', margin: '0 auto', padding: props.pad === false ? '0 24px 80px' : '36px 24px 80px' }}>
      {props.children}
    </main>
  )
}

export type Crumb = { label: string; href?: string }
export function Breadcrumb(props: { items: Crumb[] }) {
  return (
    <nav class="mono" style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'font-size': '12.5px', 'margin-bottom': '18px', 'flex-wrap': 'wrap' }}>
      <For each={props.items}>
        {(it, i) => {
          const lastColor = () => (i() === props.items.length - 1 ? 'var(--text-hi)' : 'var(--text-lo)')
          return (
            <span style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}>
              <Show when={i() > 0}>
                <span style={{ color: 'var(--text-faint)', opacity: 0.5 }}>/</span>
              </Show>
              <Show
                when={it.href}
                fallback={<span style={{ color: lastColor(), 'white-space': 'nowrap' }}>{it.label}</span>}
              >
                <A href={it.href!} class="cb-crumb" style={{ color: lastColor(), 'white-space': 'nowrap' }}>{it.label}</A>
              </Show>
            </span>
          )
        }}
      </For>
    </nav>
  )
}

/* ---- project section nav ------------------------------------------------
   Every project page shows the SAME set of sections with the current one lit —
   pages must not hand-roll "links to everyone but me" (the lists drift and the
   menu appears to change shape as you navigate; that shipped as a bug once). */
export type ProjectSection = 'issues' | 'logs' | 'heartbeats' | 'settings'

export function ProjectNav(props: { projectId: number; current: ProjectSection }) {
  const { user } = useAuth()
  const sections = () => {
    const all: Array<{ key: ProjectSection; label: string; path: string }> = [
      { key: 'issues', label: 'issues', path: 'issues' },
      { key: 'logs', label: 'live logs', path: 'logs' },
      { key: 'heartbeats', label: 'heartbeats', path: 'heartbeats' },
      { key: 'settings', label: 'settings', path: 'settings' },
    ]
    return all.filter((s) => s.key !== 'logs' || user()?.live_logs_enabled !== false)
  }
  return (
    <nav style={{ display: 'flex', gap: '4px' }} aria-label="project sections">
      <For each={sections()}>
        {(s) => {
          const active = () => s.key === props.current
          return (
            <A
              href={`/projects/${props.projectId}/${s.path}`}
              class="btn ghost sm"
              aria-current={active() ? 'page' : undefined}
              style={
                active()
                  ? {
                      color: 'var(--text-hi)',
                      background: 'oklch(1 0 0 / 0.05)',
                      'border-color': 'var(--line-strong)',
                      'pointer-events': 'none',
                    }
                  : undefined
              }
            >
              {s.label}
            </A>
          )
        }}
      </For>
    </nav>
  )
}

type Hint = { keys: string[]; label: string }
const FOOTER_HINTS: Hint[] = [
  { keys: ['j', 'k'], label: 'nav' },
  { keys: ['/'], label: 'search' },
  { keys: ['⌘K'], label: 'commands' },
  { keys: ['↵'], label: 'open' },
]

export function FooterHints() {
  return (
    <footer
      style={{
        position: 'fixed', bottom: 0, left: 0, right: 0, 'z-index': 30,
        'border-top': '1px solid var(--line)', background: 'oklch(0.139 0.006 73 / 0.82)',
        'backdrop-filter': 'blur(14px)', '-webkit-backdrop-filter': 'blur(14px)',
      }}
    >
      <div style={{ 'max-width': 'var(--maxw)', margin: '0 auto', height: '32px', padding: '0 24px', display: 'flex', 'align-items': 'center', gap: '16px' }}>
        <For each={FOOTER_HINTS}>
          {(h, i) => (
            <span style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'font-size': '11.5px', color: 'var(--text-faint)' }}>
              <span style={{ display: 'flex', gap: '3px' }}>
                <For each={h.keys}>{(k) => <span class="kbd">{k}</span>}</For>
              </span>
              <span style={{ color: 'var(--text-lo)' }}>{h.label}</span>
              <Show when={i() < FOOTER_HINTS.length - 1}>
                <span style={{ opacity: 0.4, 'margin-left': '8px' }}>·</span>
              </Show>
            </span>
          )}
        </For>
        <div style={{ flex: '1' }} />
        <span style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'font-size': '11.5px', color: 'var(--text-faint)' }}>
          <span class="livedot" /> live
        </span>
      </div>
    </footer>
  )
}
