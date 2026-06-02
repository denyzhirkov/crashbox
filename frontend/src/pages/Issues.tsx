import { A, useNavigate, useParams, useSearchParams } from '@solidjs/router'
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js'
import { api } from '../api/client'
import type { Issue, IssueFilters } from '../api/types'
import { Breadcrumb, Page } from '../components/layout'
import { fmt, Icon, SevCue, Sparkline, Voice } from '../components/primitives'
import { useAuth } from '../lib/auth-context'
import { loadViews, removeView, saveView, type SavedView } from '../lib/saved-views'
import { relTime } from '../lib/time'

type StatusFilter = 'unresolved' | 'resolved' | 'snoozed' | 'all'
const STATUSES: StatusFilter[] = ['unresolved', 'resolved', 'snoozed', 'all']

// The unified search box: `level:error env:production release:1.4.2` + free text.
// Recognised filter keys map to dedicated backend params; everything else is free-text search.
const FILTER_KEYS: Record<string, keyof IssueFilters> = {
  level: 'level',
  env: 'environment',
  environment: 'environment',
  release: 'release',
}
type Chip = { key: string; param: keyof IssueFilters; value: string }

function parseQuery(str: string): { filters: Partial<IssueFilters>; freeText: string; chips: Chip[] } {
  const filters: Partial<IssueFilters> = {}
  const terms: string[] = []
  const chips: Chip[] = []
  for (const tok of str.trim().split(/\s+/).filter(Boolean)) {
    const m = tok.match(/^([a-z]+):(.+)$/i)
    const param = m ? FILTER_KEYS[m[1].toLowerCase()] : undefined
    if (m && param) {
      ;(filters as Record<string, string>)[param] = m[2]
      chips.push({ key: m[1].toLowerCase(), param, value: m[2] })
    } else {
      terms.push(tok)
    }
  }
  return { filters, freeText: terms.join(' '), chips }
}

export default function IssuesPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [searchParams, setSearchParams] = useSearchParams()
  const nav = useNavigate()
  const { user } = useAuth()
  const liveLogsEnabled = () => user()?.live_logs_enabled !== false

  const initialTagsFromUrl = (): Array<[string, string]> => {
    const raw = searchParams.tag
    const all = Array.isArray(raw) ? raw : raw ? [raw] : []
    return all.flatMap<[string, string]>((p) => {
      const idx = (p as string).indexOf('=')
      return idx > 0 ? [[(p as string).slice(0, idx), (p as string).slice(idx + 1)]] : []
    })
  }

  const [query, setQuery] = createSignal((searchParams.q as string) ?? '')
  const [status, setStatus] = createSignal<StatusFilter>('unresolved')
  const [tags, setTags] = createSignal<Array<[string, string]>>(initialTagsFromUrl())
  const [cursor, setCursor] = createSignal(0)
  let searchRef: HTMLInputElement | undefined

  const parsed = createMemo(() => parseQuery(query()))

  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const [issues] = createResource(
    () => ({ pid: projectId(), p: parsed(), s: status(), t: tags() }),
    ({ pid, p, s, t }) =>
      api.issues.list(pid, { status: s, query: p.freeText || undefined, ...p.filters }, t),
  )

  // Saved views (localStorage).
  const [views, setViews] = createSignal<SavedView[]>(loadViews(projectId()))
  createEffect(() => setViews(loadViews(projectId())))

  const applyView = (v: SavedView) => {
    setStatus((v.filters.status as StatusFilter) ?? 'unresolved')
    setQuery(v.filters.query ?? '')
    setTags(v.tags)
  }
  const saveCurrent = () => {
    const name = window.prompt('// name this view')
    if (!name) return
    saveView(projectId(), {
      id: crypto.randomUUID(),
      name,
      filters: { status: status(), query: query() || undefined },
      tags: tags(),
    })
    setViews(loadViews(projectId()))
  }
  const deleteView = (id: string) => {
    removeView(projectId(), id)
    setViews(loadViews(projectId()))
  }

  const removeChip = (idx: number) => {
    const toks = query().trim().split(/\s+/).filter(Boolean)
    let seen = -1
    const kept = toks.filter((tk) => {
      const m = tk.match(/^([a-z]+):/i)
      if (m && FILTER_KEYS[m[1].toLowerCase()]) {
        seen++
        return seen !== idx
      }
      return true
    })
    setQuery(kept.join(' '))
  }
  const removeTag = (idx: number) => setTags((t) => t.filter((_, i) => i !== idx))
  const clearAll = () => {
    setQuery('')
    setTags([])
  }

  // keyboard: j/k move cursor, enter opens, / focuses search
  const onKey = (e: KeyboardEvent) => {
    if (document.querySelector('.cb-palette')) return
    const typing = ['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName ?? '')
    if (e.key === '/' && !typing) {
      e.preventDefault()
      searchRef?.focus()
      return
    }
    if (typing) {
      if (e.key === 'Escape') (document.activeElement as HTMLElement)?.blur()
      return
    }
    const list = issues() ?? []
    if (e.key === 'j') {
      e.preventDefault()
      setCursor((c) => Math.min(list.length - 1, c + 1))
    } else if (e.key === 'k') {
      e.preventDefault()
      setCursor((c) => Math.max(0, c - 1))
    } else if (e.key === 'Enter' || e.key === 'o') {
      const target = list[cursor()]
      if (target) nav(`/issues/${target.id}`)
    }
  }
  window.addEventListener('keydown', onKey)
  onCleanup(() => window.removeEventListener('keydown', onKey))

  // reset cursor when the result set changes
  createEffect(() => {
    query()
    status()
    tags()
    setCursor(0)
  })

  // mirror filters to URL so links/back-button share state
  createEffect(() => {
    setSearchParams(
      { tag: tags().map(([k, v]) => `${k}=${v}`), q: query() || undefined },
      { replace: true },
    )
  })

  return (
    <Page>
      <div style={{ display: 'flex', 'align-items': 'flex-start', 'justify-content': 'space-between', 'margin-bottom': '16px' }}>
        <Breadcrumb items={[{ label: 'projects', href: '/projects' }, { label: project()?.name ?? '…' }]} />
        <div style={{ display: 'flex', gap: '8px' }}>
          <Show when={liveLogsEnabled()}>
            <A href={`/projects/${projectId()}/logs`} class="btn ghost sm">live logs</A>
          </Show>
          <A href={`/projects/${projectId()}/settings`} class="btn ghost sm">settings</A>
        </div>
      </div>

      {/* unified search / filter */}
      <span class="field cb-focusring" style={{ display: 'flex', 'align-items': 'center', gap: '8px', padding: '0 12px', height: '42px', 'margin-bottom': '12px' }}>
        <Icon name="search" size={15} style={{ color: 'var(--text-faint)' }} />
        <input
          ref={searchRef}
          id="issue-search"
          class="input mono"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          placeholder="search or filter:  level:error  env:production  release:1.4.2"
          style={{ height: '40px', border: 'none', background: 'transparent', padding: 0, 'font-size': '13px' }}
        />
        <span class="kbd">/</span>
      </span>

      {/* status chips + saved views */}
      <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'margin-bottom': '12px', 'flex-wrap': 'wrap' }}>
        <For each={STATUSES}>
          {(s) => (
            <button class={`chip ${status() === s ? 'on' : ''}`} onClick={() => setStatus(s)}>{s}</button>
          )}
        </For>
        <span style={{ width: '1px', height: '18px', background: 'var(--line)', margin: '0 4px' }} />
        <For each={views()}>
          {(v) => (
            <button
              class="chip star"
              onClick={() => applyView(v)}
              onContextMenu={(e) => {
                e.preventDefault()
                if (window.confirm(`// delete view "${v.name}"?`)) deleteView(v.id)
              }}
              title="click to apply · right-click to delete"
            >
              <span class="ic" style={{ display: 'flex' }}><Icon name="star" size={11} /></span>
              {v.name}
            </button>
          )}
        </For>
        <button class="chip" style={{ color: 'var(--text-faint)' }} onClick={saveCurrent}>
          <Icon name="plus" size={11} /> save view
        </button>
      </div>

      {/* active filters (typed + tag) */}
      <Show when={parsed().chips.length > 0 || tags().length > 0}>
        <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'margin-bottom': '14px', 'flex-wrap': 'wrap' }}>
          <For each={parsed().chips}>
            {(c, i) => (
              <button class="chip mono on" onClick={() => removeChip(i())} title="remove filter">
                <span style={{ color: 'var(--text-faint)' }}>{c.key}:</span>
                <span style={{ color: 'var(--text-hi)' }}>{c.value}</span>
                <span class="x"><Icon name="x" size={11} /></span>
              </button>
            )}
          </For>
          <For each={tags()}>
            {([k, v], i) => (
              <button class="chip mono on" onClick={() => removeTag(i())} title="remove filter">
                <span style={{ color: 'var(--text-faint)' }}>{k}:</span>
                <span style={{ color: 'var(--text-hi)' }}>{v}</span>
                <span class="x"><Icon name="x" size={11} /></span>
              </button>
            )}
          </For>
          <button class="chip" style={{ color: 'var(--text-faint)' }} onClick={clearAll}>clear all</button>
        </div>
      </Show>

      {/* the tape */}
      <div class="card" style={{ padding: '6px' }}>
        <Show
          when={!issues.loading}
          fallback={
            <div style={{ padding: '8px' }}>
              <For each={[0, 1, 2, 3]}>
                {() => (
                  <div style={{ display: 'flex', 'align-items': 'center', gap: '12px', padding: '12px 10px' }}>
                    <div class="skel" style={{ width: '3px', height: '24px' }} />
                    <div class="skel" style={{ width: '56px', height: '14px' }} />
                    <div class="skel" style={{ width: '72px', height: '20px' }} />
                    <div class="skel" style={{ flex: '1', height: '14px' }} />
                    <div class="skel" style={{ width: '48px', height: '12px' }} />
                  </div>
                )}
              </For>
            </div>
          }
        >
          <Show
            when={(issues() ?? []).length > 0}
            fallback={
              <div style={{ padding: '34px 18px' }}>
                <Voice>{query().trim() || tags().length ? '0 matches. broaden the query or check the filter.' : "nothing's on fire"}</Voice>
              </div>
            }
          >
            <For each={issues()}>
              {(issue, i) => (
                <IssueRow issue={issue} focused={i() === cursor()} onHover={() => setCursor(i())} />
              )}
            </For>
          </Show>
        </Show>
      </div>

      <Show when={!issues.loading && (issues() ?? []).length > 0}>
        <div style={{ display: 'flex', 'align-items': 'center', 'justify-content': 'space-between', 'margin-top': '12px' }}>
          <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)' }}>{(issues() ?? []).length} issues</span>
          <span class="mono" style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'font-size': '11.5px', color: 'var(--text-faint)' }}>
            <span class="kbd">j</span><span class="kbd">k</span> to move · <span class="kbd">↵</span> open
          </span>
        </div>
      </Show>
    </Page>
  )
}

function isSnoozed(issue: Issue): boolean {
  if (!issue.snoozed_until) return false
  if (issue.snoozed_until === 'forever') return true
  return Date.parse(issue.snoozed_until) > Date.now()
}

function IssueRow(props: { issue: Issue; focused: boolean; onHover: () => void }) {
  const nav = useNavigate()
  const resolved = () => props.issue.status === 'resolved'
  const density = '12px' // regular
  return (
    <div
      class="cb-issuerow"
      onMouseEnter={props.onHover}
      onClick={() => nav(`/issues/${props.issue.id}`)}
      style={{
        position: 'relative', display: 'flex', 'align-items': 'center', gap: '14px',
        padding: `${density} 16px`, 'border-radius': 'var(--r-md)', cursor: 'pointer',
        opacity: resolved() ? 0.5 : 1,
        background: props.focused ? 'oklch(1 0 0 / 0.035)' : 'transparent',
        'box-shadow': props.focused ? 'var(--glow-focus)' : 'none',
        transition: 'background 0.12s',
      }}
    >
      <Show when={props.focused}>
        <span style={{ position: 'absolute', left: 0, top: '8px', bottom: '8px', width: '3px', 'border-radius': '999px', background: 'var(--accent-grad-v)' }} />
      </Show>
      <SevCue level={props.issue.level} />
      <span class="mono tnum" style={{ 'font-size': '13px', color: resolved() ? 'var(--text-faint)' : 'var(--text-mid)', width: '64px', 'text-align': 'right', flex: 'none' }}>
        {fmt(props.issue.event_count)}×
      </span>
      <Sparkline buckets={props.issue.last_24h_buckets} w={72} h={22} dim={resolved()} />
      <div style={{ flex: '1', 'min-width': 0 }}>
        <div class="mono" style={{ overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap', 'font-size': '13.5px', color: resolved() ? 'var(--text-mid)' : 'var(--text-hi)', 'line-height': 1.4 }}>
          {props.issue.title}
        </div>
      </div>
      <Show when={isSnoozed(props.issue)}>
        <Icon name="snooze" size={13} style={{ color: 'var(--text-faint)' }} />
      </Show>
      <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)', flex: 'none', width: '64px', 'text-align': 'right' }}>
        {relTime(props.issue.last_seen)}
      </span>
    </div>
  )
}
