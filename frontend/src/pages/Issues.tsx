import { A, useNavigate, useParams, useSearchParams } from '@solidjs/router'
import {
  createEffect,
  createResource,
  createSignal,
  For,
  onCleanup,
  Show,
} from 'solid-js'
import { api } from '../api/client'
import type { Issue } from '../api/types'
import { EdgeBar } from '../components/EdgeBar'
import { Sparkline } from '../components/Sparkline'
import { loadViews, removeView, saveView, type SavedView } from '../lib/saved-views'
import { relTime } from '../lib/time'

type StatusFilter = 'unresolved' | 'resolved' | 'snoozed' | 'all'

export default function IssuesPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [searchParams, setSearchParams] = useSearchParams()

  const initialTagsFromUrl = (): Array<[string, string]> => {
    const raw = searchParams.tag
    const all = Array.isArray(raw) ? raw : raw ? [raw] : []
    return all.flatMap<[string, string]>((p) => {
      const idx = (p as string).indexOf('=')
      return idx > 0 ? [[(p as string).slice(0, idx), (p as string).slice(idx + 1)]] : []
    })
  }

  const [query, setQuery] = createSignal('')
  const [status, setStatus] = createSignal<StatusFilter>('unresolved')
  const [tags, setTags] = createSignal<Array<[string, string]>>(initialTagsFromUrl())
  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const [issues, { refetch }] = createResource(
    () => ({ pid: projectId(), q: query(), s: status(), t: tags() }),
    ({ pid, q, s, t }) => api.issues.list(pid, { query: q || undefined, status: s }, t),
  )

  // Saved views (localStorage). Mutates trigger re-read.
  const [views, setViews] = createSignal<SavedView[]>(loadViews(projectId()))
  createEffect(() => setViews(loadViews(projectId())))

  const removeTag = (idx: number) => setTags((t) => t.filter((_, i) => i !== idx))

  const applyView = (v: SavedView) => {
    setStatus((v.filters.status as StatusFilter) ?? 'unresolved')
    setQuery(v.filters.query ?? '')
    setTags(v.tags)
  }
  const persistViews = (next: SavedView[]) => setViews(next)
  const saveCurrent = () => {
    const name = window.prompt('// name this view')
    if (!name) return
    const view: SavedView = {
      id: crypto.randomUUID(),
      name,
      filters: { status: status(), query: query() || undefined },
      tags: tags(),
    }
    saveView(projectId(), view)
    persistViews(loadViews(projectId()))
  }
  const deleteView = (id: string) => {
    removeView(projectId(), id)
    persistViews(loadViews(projectId()))
  }

  // j/k keyboard nav
  const [cursor, setCursor] = createSignal(0)
  const nav = useNavigate()
  const onKey = (e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement) return
    const list = issues() ?? []
    if (e.key === 'j') {
      setCursor((c) => Math.min(c + 1, Math.max(list.length - 1, 0)))
    } else if (e.key === 'k') {
      setCursor((c) => Math.max(c - 1, 0))
    } else if (e.key === 'o' || e.key === 'Enter') {
      const target = list[cursor()]
      if (target) nav(`/issues/${target.id}`)
    } else if (e.key === '/') {
      e.preventDefault()
      ;(document.getElementById('issue-search') as HTMLInputElement | null)?.focus()
    }
  }
  window.addEventListener('keydown', onKey)
  onCleanup(() => window.removeEventListener('keydown', onKey))

  // Reset cursor when filters change
  createEffect(() => {
    query()
    status()
    tags()
    setCursor(0)
  })

  // Mirror tags to URL so links/back-button share filters.
  createEffect(() => {
    const t = tags()
    setSearchParams(
      { tag: t.map(([k, v]) => `${k}=${v}`) },
      { replace: true },
    )
  })

  return (
    <section class="flex flex-col gap-5">
      <header class="flex items-baseline gap-3">
        <A href="/projects" class="text-ink-400 hover:text-ink-100 text-[12px]">
          projects
        </A>
        <span class="text-ink-500">/</span>
        <h1 class="font-serif text-[24px] text-ink-50 leading-none">
          {project()?.name ?? '…'}
        </h1>
        <A
          href={`/projects/${projectId()}/settings`}
          class="ml-auto text-[11px] text-ink-400 hover:text-ink-100"
        >
          settings
        </A>
      </header>

      <div class="flex gap-2 items-center text-[12px]">
        <input
          id="issue-search"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          placeholder="// search title…  (/ to focus)"
          class="flex-1 bg-ink-800 border border-ink-600 px-3 py-2 focus:border-crash focus:outline-none"
        />
        <StatusChip current={status()} value="unresolved" set={setStatus} />
        <StatusChip current={status()} value="resolved" set={setStatus} />
        <StatusChip current={status()} value="snoozed" set={setStatus} />
        <StatusChip current={status()} value="all" set={setStatus} />
        <button
          class="text-ink-400 hover:text-ink-100"
          onClick={() => void refetch()}
          title="refresh"
        >
          ↻
        </button>
      </div>

      <Show when={tags().length > 0}>
        <div class="flex flex-wrap gap-1.5 text-[11px]">
          <For each={tags()}>
            {([k, v], i) => (
              <button
                onClick={() => removeTag(i())}
                class="px-2 py-[2px] border border-crash text-crash hover:bg-crash/10"
                title="click to remove"
              >
                {k}:{v} ×
              </button>
            )}
          </For>
          <button
            onClick={() => setTags([])}
            class="px-2 py-[2px] text-ink-400 hover:text-ink-100"
          >
            clear all
          </button>
        </div>
      </Show>

      <div class="flex flex-wrap gap-1.5 text-[11px] items-center">
        <For each={views()}>
          {(v) => (
            <button
              onClick={() => applyView(v)}
              onContextMenu={(e) => {
                e.preventDefault()
                if (window.confirm(`// delete view "${v.name}"?`)) deleteView(v.id)
              }}
              class="px-2 py-[2px] border border-ink-600 text-ink-300 hover:border-ink-400 hover:text-ink-100"
              title="click to apply · right-click to delete"
            >
              ★ {v.name}
            </button>
          )}
        </For>
        <button
          onClick={saveCurrent}
          class="text-ink-500 hover:text-ink-100"
        >
          + save view
        </button>
      </div>

      <Show
        when={!issues.loading}
        fallback={<p class="text-ink-400 text-[12px]">// loading…</p>}
      >
        <Show
          when={(issues() ?? []).length > 0}
          fallback={
            <p class="text-ink-300 text-[13px] py-6">
              // nothing's on fire
            </p>
          }
        >
          <ul class="border-y border-ink-600 divide-y divide-ink-600">
            <For each={issues()}>
              {(issue, i) => (
                <IssueRow
                  issue={issue}
                  focused={i() === cursor()}
                  onClick={() => setCursor(i())}
                />
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </section>
  )
}

function StatusChip(props: {
  current: StatusFilter
  value: StatusFilter
  set: (s: StatusFilter) => void
}) {
  const active = () => props.current === props.value
  return (
    <button
      onClick={() => props.set(props.value)}
      class={`px-2 py-1 border ${
        active()
          ? 'border-crash text-crash'
          : 'border-ink-600 text-ink-300 hover:border-ink-400 hover:text-ink-100'
      }`}
    >
      {props.value}
    </button>
  )
}

function IssueRow(props: { issue: Issue; focused: boolean; onClick: () => void }) {
  const isResolved = () => props.issue.status === 'resolved'
  const dim = () => (isResolved() ? 'opacity-50' : '')
  return (
    <li class="flex items-stretch">
      <EdgeBar level={props.issue.level} resolved={isResolved()} />
      <A
        href={`/issues/${props.issue.id}`}
        onClick={props.onClick}
        class={`flex-1 flex items-center gap-3 px-3 py-2 hover:bg-ink-700/30 ${dim()} ${
          props.focused ? 'bg-ink-700/40' : ''
        }`}
      >
        <span class="text-[11px] text-ink-400 w-14 text-right tabular-nums shrink-0">
          {props.issue.event_count.toLocaleString()}×
        </span>
        <Sparkline buckets={props.issue.last_24h_buckets} />
        <span class="text-ink-100 truncate font-mono text-[13px]">{props.issue.title}</span>
        <span class="ml-auto text-[11px] text-ink-400 shrink-0">
          {relTime(props.issue.last_seen)}
        </span>
      </A>
    </li>
  )
}
