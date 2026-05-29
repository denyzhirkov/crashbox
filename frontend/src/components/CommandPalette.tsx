// Hand-rolled command palette. ~180 lines, no kbar (React-only) and no other dep.
//
// Triggered by Cmd+K / Ctrl+K from anywhere. The command list is computed from the current
// route via `useLocation()`, so when you're on /issues/:id you get resolve/snooze actions;
// elsewhere you get just navigation + session controls.
//
// Keyboard: arrows nav, Enter execute, Esc / click-outside close. Mouse hover also moves the
// cursor.

import { useLocation, useNavigate } from '@solidjs/router'
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
} from 'solid-js'
import { api } from '../api/client'
import { useAuth } from '../lib/auth-context'

type Command = {
  id: string
  category: 'nav' | 'issue' | 'project' | 'session'
  label: string
  hint?: string
  run: () => void | Promise<void>
}

export function CommandPalette() {
  const [open, setOpen] = createSignal(false)
  const [query, setQuery] = createSignal('')
  const [cursor, setCursor] = createSignal(0)
  const nav = useNavigate()
  const auth = useAuth()
  const location = useLocation()

  let inputRef: HTMLInputElement | undefined

  const close = () => {
    setOpen(false)
    setQuery('')
    setCursor(0)
  }

  const openMe = () => {
    setOpen(true)
    setQuery('')
    setCursor(0)
    // Focus the input once Solid renders it.
    queueMicrotask(() => inputRef?.focus())
  }

  // Commands depend on the current route. Keep this tiny — discoverability via search beats
  // a long list of options.
  const commands = createMemo<Command[]>(() => {
    const out: Command[] = []

    out.push({
      id: 'nav-projects',
      category: 'nav',
      label: 'go to projects',
      hint: 'p',
      run: () => nav('/projects'),
    })

    const issueMatch = location.pathname.match(/^\/issues\/(\d+)$/)
    if (issueMatch) {
      const id = Number(issueMatch[1])
      out.push({
        id: 'resolve',
        category: 'issue',
        label: 'mark fixed',
        run: async () => {
          await api.issues.setStatus(id, 'resolved')
          window.location.reload()
        },
      })
      out.push({
        id: 'reopen',
        category: 'issue',
        label: 'reopen',
        run: async () => {
          await api.issues.setStatus(id, 'unresolved')
          window.location.reload()
        },
      })
      out.push({
        id: 'snooze1h',
        category: 'issue',
        label: 'snooze · 1 hour',
        run: () => api.issues.snooze(id, '1h').then(() => window.location.reload()),
      })
      out.push({
        id: 'snooze1d',
        category: 'issue',
        label: 'snooze · 1 day',
        run: () => api.issues.snooze(id, '1d').then(() => window.location.reload()),
      })
      out.push({
        id: 'snoozeforever',
        category: 'issue',
        label: 'snooze · until next crash',
        run: () => api.issues.snooze(id, 'forever').then(() => window.location.reload()),
      })
      out.push({
        id: 'wake',
        category: 'issue',
        label: 'wake (clear snooze)',
        run: () => api.issues.snooze(id, 'wake').then(() => window.location.reload()),
      })
    }

    const projectMatch = location.pathname.match(/^\/projects\/(\d+)(\/|$)/)
    if (projectMatch) {
      const id = Number(projectMatch[1])
      out.push({
        id: 'project-issues',
        category: 'nav',
        label: 'issues for this project',
        run: () => nav(`/projects/${id}/issues`),
      })
      out.push({
        id: 'project-settings',
        category: 'nav',
        label: 'project settings',
        run: () => nav(`/projects/${id}/settings`),
      })
      out.push({
        id: 'copy-dsn',
        category: 'project',
        label: 'copy DSN',
        run: async () => {
          const dsn = await api.projects.dsn(id)
          try {
            await navigator.clipboard.writeText(dsn.dsn)
          } catch {
            /* clipboard blocked */
          }
        },
      })
    }

    out.push({
      id: 'logout',
      category: 'session',
      label: 'logout',
      run: async () => {
        await auth.logout()
        nav('/login', { replace: true })
      },
    })
    return out
  })

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase()
    const all = commands()
    if (!q) return all
    return all.filter(
      (c) => c.label.toLowerCase().includes(q) || c.category.includes(q),
    )
  })

  // Keep cursor in bounds when filter shrinks
  createEffect(() => {
    const n = filtered().length
    if (cursor() >= n) setCursor(Math.max(0, n - 1))
  })

  const onKey = (e: KeyboardEvent) => {
    const isOpenShortcut = (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k'
    if (isOpenShortcut) {
      e.preventDefault()
      if (open()) close()
      else openMe()
      return
    }
    if (!open()) return

    if (e.key === 'Escape') {
      e.preventDefault()
      close()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(c + 1, filtered().length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(c - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const cmd = filtered()[cursor()]
      if (cmd) {
        close()
        void cmd.run()
      }
    }
  }

  window.addEventListener('keydown', onKey)
  onCleanup(() => window.removeEventListener('keydown', onKey))

  return (
    <Show when={open()}>
      <div
        class="fixed inset-0 bg-ink-900/70 z-50 flex items-start justify-center pt-32"
        onClick={close}
      >
        <div
          onClick={(e) => e.stopPropagation()}
          class="w-[520px] max-w-[90vw] bg-ink-800 border border-ink-600 shadow-2xl"
        >
          <input
            ref={inputRef}
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value)
              setCursor(0)
            }}
            placeholder="// type a command…  esc to close"
            class="w-full bg-transparent border-b border-ink-600 px-4 py-3 text-ink-100 focus:outline-none font-mono text-[13px]"
          />
          <ul class="max-h-[320px] overflow-y-auto py-1">
            <For
              each={filtered()}
              fallback={
                <li class="px-4 py-3 text-[12px] text-ink-400">// no matches</li>
              }
            >
              {(cmd, i) => (
                <li
                  onMouseEnter={() => setCursor(i())}
                  onClick={() => {
                    close()
                    void cmd.run()
                  }}
                  class={`px-4 py-2 flex items-baseline gap-3 text-[12px] cursor-pointer ${
                    i() === cursor()
                      ? 'bg-ink-700/50 border-l-2 border-crash'
                      : 'border-l-2 border-transparent'
                  }`}
                >
                  <span class="text-ink-500 text-[10px] uppercase w-16 shrink-0">
                    {cmd.category}
                  </span>
                  <span class="text-ink-100 flex-1 truncate">{cmd.label}</span>
                  <Show when={cmd.hint}>
                    <kbd class="text-ink-500 text-[10px]">{cmd.hint}</kbd>
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <div class="border-t border-ink-600 px-4 py-1.5 text-[10px] text-ink-500 flex gap-4">
            <span><kbd class="text-ink-300">↑↓</kbd> nav</span>
            <span><kbd class="text-ink-300">↵</kbd> run</span>
            <span><kbd class="text-ink-300">esc</kbd> close</span>
            <span class="ml-auto opacity-70">cmd+k anywhere</span>
          </div>
        </div>
      </div>
    </Show>
  )
}
