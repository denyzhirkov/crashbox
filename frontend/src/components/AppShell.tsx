import { useLocation, useNavigate } from '@solidjs/router'
import { createMemo, createResource, type JSX, Show } from 'solid-js'
import { api } from '../api/client'
import { useAuth } from '../lib/auth-context'
import { setPaletteOpen } from '../lib/palette'
import { CommandPalette } from './CommandPalette'
import { FooterHints, Wordmark } from './layout'
import { Icon } from './primitives'

export function AppShell(props: { children: JSX.Element }) {
  const { user, logout } = useAuth()
  const nav = useNavigate()
  const location = useLocation()

  const onLogout = async () => {
    await logout()
    nav('/login', { replace: true })
  }

  // Current project (for the top-bar chip) derived from the path on project-scoped routes.
  const projectId = createMemo(() => {
    const m = location.pathname.match(/^\/projects\/(\d+)(\/|$)/)
    return m ? Number(m[1]) : null
  })
  const [project] = createResource(projectId, (id) => (id ? api.projects.get(id) : Promise.resolve(null)))

  return (
    <div style={{ 'min-height': '100%', display: 'flex', 'flex-direction': 'column' }}>
      <header
        style={{
          position: 'sticky', top: 0, 'z-index': 40,
          'border-bottom': '1px solid var(--line)', background: 'oklch(0.166 0.006 73 / 0.72)',
          'backdrop-filter': 'blur(16px) saturate(140%)', '-webkit-backdrop-filter': 'blur(16px) saturate(140%)',
        }}
      >
        <div style={{ 'max-width': 'var(--maxw)', margin: '0 auto', height: '52px', padding: '0 24px', display: 'flex', 'align-items': 'center', gap: '16px' }}>
          <Wordmark href="/projects" />
          <Show when={project()}>
            {(p) => (
              <>
                <span style={{ color: 'var(--text-faint)', opacity: 0.5 }}>/</span>
                <button
                  onClick={() => setPaletteOpen(true)}
                  class="cb-projbtn"
                  style={{
                    display: 'flex', 'align-items': 'center', gap: '8px', background: 'none',
                    border: '1px solid transparent', 'border-radius': '7px', padding: '4px 8px',
                    cursor: 'pointer', color: 'var(--text-hi)', 'margin-left': '-2px',
                  }}
                  title="switch project / commands"
                >
                  <span class="mono" style={{ 'font-size': '13px', 'font-weight': 500, 'white-space': 'nowrap' }}>{p().name}</span>
                  <Icon name="chevron" size={12} style={{ color: 'var(--text-faint)' }} />
                </button>
              </>
            )}
          </Show>

          <div style={{ flex: '1' }} />

          <button
            onClick={() => setPaletteOpen(true)}
            class="cb-cmdhint"
            style={{
              display: 'flex', 'align-items': 'center', gap: '8px', background: 'var(--bg-sunken)',
              border: '1px solid var(--line)', 'border-radius': '7px', height: '30px',
              padding: '0 8px 0 10px', cursor: 'pointer', color: 'var(--text-faint)',
            }}
          >
            <Icon name="search" size={13} />
            <span style={{ 'font-size': '12.5px' }}>search or jump…</span>
            <span class="kbd" style={{ 'margin-left': '6px' }}>⌘K</span>
          </button>

          <Show when={user()}>
            {(u) => (
              <>
                <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)', 'max-width': '220px', overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}>
                  {u().email}
                </span>
                <button class="btn ghost sm" onClick={onLogout}>logout</button>
              </>
            )}
          </Show>
        </div>
      </header>

      <div style={{ flex: '1' }}>{props.children}</div>

      <FooterHints />
      <CommandPalette />
    </div>
  )
}
