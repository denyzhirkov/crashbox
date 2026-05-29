import { A } from '@solidjs/router'
import { createResource, createSignal, For, Show } from 'solid-js'
import { api } from '../api/client'
import type { Issue, ProjectOverview } from '../api/types'
import { EdgeBar } from '../components/EdgeBar'
import { Sparkline } from '../components/Sparkline'
import { useAuth } from '../lib/auth-context'
import { relTime } from '../lib/time'

export default function ProjectsPage() {
  const [overview, { refetch }] = createResource(() => api.projects.overview())
  const { user } = useAuth()
  const [showCreate, setShowCreate] = createSignal(false)

  return (
    <section class="flex flex-col gap-6">
      <header class="flex items-baseline justify-between">
        <h1 class="font-serif text-[28px] text-ink-50 leading-none">projects</h1>
        <div class="flex gap-3 text-[12px] text-ink-400">
          <button onClick={() => void refetch()} class="hover:text-ink-100" title="refresh">
            ↻ refresh
          </button>
          <Show when={user()?.is_admin}>
            <button
              class="hover:text-ink-100"
              onClick={() => setShowCreate((s) => !s)}
            >
              {showCreate() ? 'cancel' : '+ new'}
            </button>
          </Show>
        </div>
      </header>

      <Show when={showCreate()}>
        <CreateProjectForm
          onDone={() => {
            setShowCreate(false)
            void refetch()
          }}
        />
      </Show>

      <Show
        when={!overview.loading}
        fallback={<p class="text-ink-400 text-[12px]">// loading…</p>}
      >
        <Show
          when={(overview() ?? []).length > 0}
          fallback={
            <p class="text-ink-300 text-[13px]">
              // no projects yet. set <code>CRASHBOX_PROJECT_NAME</code> and restart.
            </p>
          }
        >
          <div class="flex flex-col gap-3">
            <For each={overview()}>{(p) => <ProjectCard project={p} />}</For>
          </div>
        </Show>
      </Show>
    </section>
  )
}

function ProjectCard(props: { project: ProjectOverview }) {
  const [dsn] = createResource(() => api.projects.dsn(props.project.id))
  const [copied, setCopied] = createSignal(false)

  const copy = async () => {
    const d = dsn()
    if (!d) return
    try {
      await navigator.clipboard.writeText(d.dsn)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      /* clipboard blocked */
    }
  }

  return (
    <article class="border border-ink-600 bg-ink-700/20 flex flex-col">
      <header class="flex items-baseline gap-3 px-4 py-3 border-b border-ink-600">
        <A
          href={`/projects/${props.project.id}/issues`}
          class="text-ink-50 hover:text-crash text-[15px]"
        >
          {props.project.name}
        </A>
        <span class="text-ink-400 text-[11px]">{props.project.slug}</span>
        <Show when={props.project.platform}>
          <span class="text-ink-400 text-[11px]">· {props.project.platform}</span>
        </Show>
        <div class="ml-auto flex gap-4 text-[11px] text-ink-300">
          <A
            href={`/projects/${props.project.id}/issues`}
            class="hover:text-ink-100"
          >
            issues
          </A>
          <A
            href={`/projects/${props.project.id}/settings`}
            class="hover:text-ink-100"
          >
            settings
          </A>
        </div>
      </header>

      <div class="px-4 py-3 flex items-baseline gap-6 text-[12px] border-b border-ink-600">
        <Stat
          value={props.project.unresolved_count}
          label="unresolved"
          accent={props.project.unresolved_count > 0}
        />
        <Stat value={props.project.events_24h} label="events / 24h" />
        <Show when={dsn()}>
          {(d) => (
            <button
              onClick={copy}
              class={`ml-auto font-mono text-[11px] text-left text-ink-300 hover:text-ink-100 truncate max-w-[440px] ${
                copied() ? 'underline decoration-crash decoration-2 underline-offset-4' : ''
              }`}
              title="click to copy DSN"
            >
              {d().dsn}
            </button>
          )}
        </Show>
      </div>

      <Show
        when={props.project.recent_issues.length > 0}
        fallback={
          <p class="px-4 py-4 text-[12px] text-ink-400">
            // no events yet — point your SDK at the DSN above
          </p>
        }
      >
        <ul class="divide-y divide-ink-700">
          <For each={props.project.recent_issues}>
            {(issue) => <RecentIssueRow issue={issue} />}
          </For>
        </ul>
      </Show>
    </article>
  )
}

function Stat(props: { value: number; label: string; accent?: boolean }) {
  return (
    <div class="flex items-baseline gap-1.5">
      <span
        class={`tabular-nums text-[15px] ${
          props.accent ? 'text-crash' : 'text-ink-100'
        }`}
      >
        {props.value.toLocaleString()}
      </span>
      <span class="text-ink-400">{props.label}</span>
    </div>
  )
}

function RecentIssueRow(props: { issue: Issue }) {
  const isResolved = () => props.issue.status === 'resolved'
  return (
    <li class="flex items-stretch">
      <EdgeBar level={props.issue.level} resolved={isResolved()} />
      <A
        href={`/issues/${props.issue.id}`}
        class={`flex-1 flex items-center gap-3 px-3 py-2 hover:bg-ink-700/30 ${
          isResolved() ? 'opacity-50' : ''
        }`}
      >
        <span class="text-[11px] text-ink-400 w-12 text-right tabular-nums shrink-0">
          {props.issue.event_count.toLocaleString()}×
        </span>
        <Sparkline buckets={props.issue.last_24h_buckets} />
        <span class="text-ink-100 truncate font-mono text-[12px]">{props.issue.title}</span>
        <span class="ml-auto text-[11px] text-ink-400 shrink-0">
          {relTime(props.issue.last_seen)}
        </span>
      </A>
    </li>
  )
}

function CreateProjectForm(props: { onDone: () => void }) {
  const [name, setName] = createSignal('')
  const [platform, setPlatform] = createSignal('')
  const [busy, setBusy] = createSignal(false)
  const [err, setErr] = createSignal<string | null>(null)

  const submit = async (e: SubmitEvent) => {
    e.preventDefault()
    setBusy(true)
    setErr(null)
    try {
      await api.projects.create({
        name: name(),
        platform: platform() || undefined,
      })
      props.onDone()
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      onSubmit={submit}
      class="flex flex-col gap-3 border border-ink-600 bg-ink-700/40 p-4"
    >
      <div class="flex gap-3">
        <label class="flex flex-col gap-1 flex-1">
          <span class="text-[11px] text-ink-300">name</span>
          <input
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            required
            class="bg-ink-800 border border-ink-600 px-3 py-2 focus:border-crash focus:outline-none"
          />
        </label>
        <label class="flex flex-col gap-1 w-[160px]">
          <span class="text-[11px] text-ink-300">platform</span>
          <input
            value={platform()}
            onInput={(e) => setPlatform(e.currentTarget.value)}
            placeholder="javascript"
            class="bg-ink-800 border border-ink-600 px-3 py-2 focus:border-crash focus:outline-none"
          />
        </label>
      </div>
      <Show when={err()}>
        {(m) => <p class="text-[12px] text-crash">// {m()}</p>}
      </Show>
      <button
        type="submit"
        disabled={busy()}
        class="self-start bg-crash text-ink-50 px-3 py-1 hover:bg-crash-dim disabled:opacity-50"
      >
        {busy() ? 'creating…' : 'create'}
      </button>
    </form>
  )
}
