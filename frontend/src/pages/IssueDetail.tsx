import { A, useParams } from '@solidjs/router'
import { createResource, createSignal, For, Show } from 'solid-js'
import { api } from '../api/client'
import type { EventDetail, EventRow, Issue } from '../api/types'
import { EdgeBar } from '../components/EdgeBar'
import { absTime, relTime } from '../lib/time'

type FrameLike = {
  function?: string
  filename?: string
  abs_path?: string
  module?: string
  lineno?: number | string
  in_app?: boolean
  pre_context?: string[]
  context_line?: string
  post_context?: string[]
}

export default function IssueDetailPage() {
  const params = useParams<{ issueId: string }>()
  const issueId = () => Number(params.issueId)
  const [issue, { refetch: refetchIssue }] = createResource(issueId, (id) =>
    api.issues.get(id),
  )
  const [events] = createResource(issueId, (id) => api.issues.events(id))
  const [activeEventId, setActiveEventId] = createSignal<number | null>(null)

  // Default to the most recent event
  const currentEventId = () =>
    activeEventId() ?? events()?.[0]?.id ?? null

  const [eventDetail] = createResource(currentEventId, (id) =>
    id != null ? api.events.get(id) : Promise.resolve(null),
  )

  const toggleStatus = async () => {
    const i = issue()
    if (!i) return
    const next = i.status === 'resolved' ? 'unresolved' : 'resolved'
    await api.issues.setStatus(i.id, next)
    refetchIssue()
  }

  return (
    <article class="flex flex-col gap-6">
      <Show when={issue()} fallback={<p class="text-ink-400 text-[12px]">// loading…</p>}>
        {(i) => (
          <>
            <header class="flex items-stretch">
              <EdgeBar level={i().level} resolved={i().status === 'resolved'} />
              <div class="flex-1 pl-4 flex items-start justify-between">
                <div>
                  <p class="text-[11px] text-ink-400">
                    <A
                      href={`/projects/${i().project_id}/issues`}
                      class="hover:text-ink-100"
                    >
                      ← back to issues
                    </A>
                  </p>
                  <h1 class="font-mono text-[18px] text-ink-50 mt-1 break-words">
                    {i().title}
                  </h1>
                  <p class="text-[11px] text-ink-400 mt-1 flex gap-3 flex-wrap">
                    <span>{i().event_count.toLocaleString()} events</span>
                    <span>first: {absTime(i().first_seen)}</span>
                    <span>last: {absTime(i().last_seen)}</span>
                    <Show when={i().platform}>
                      <span>· {i().platform}</span>
                    </Show>
                    <span>· {i().status}</span>
                  </p>
                </div>
                <button
                  onClick={toggleStatus}
                  class="text-[12px] border border-ink-600 px-3 py-1 hover:border-crash hover:text-crash"
                >
                  {i().status === 'resolved' ? 'reopen' : 'mark fixed'}
                </button>
              </div>
            </header>

            <EventScrubber
              events={events() ?? []}
              activeId={currentEventId()}
              onPick={setActiveEventId}
            />

            <Show
              when={eventDetail()}
              fallback={<p class="text-ink-400 text-[12px]">// loading event…</p>}
            >
              {(detail) => <EventBody detail={detail()} issue={i()} />}
            </Show>
          </>
        )}
      </Show>
    </article>
  )
}

function EventScrubber(props: {
  events: EventRow[]
  activeId: number | null
  onPick: (id: number) => void
}) {
  return (
    <Show when={props.events.length > 1}>
      <div class="flex flex-col gap-1">
        <p class="text-[11px] text-ink-400">// events ({props.events.length})</p>
        <div class="flex gap-[2px] h-6 items-end overflow-x-auto">
          <For each={props.events}>
            {(e) => (
              <button
                onClick={() => props.onPick(e.id)}
                title={absTime(e.received_at)}
                class={`w-[6px] hover:opacity-100 transition-opacity ${
                  props.activeId === e.id
                    ? 'bg-crash h-6'
                    : 'bg-ink-400 h-4 opacity-70'
                }`}
              />
            )}
          </For>
        </div>
      </div>
    </Show>
  )
}

function EventBody(props: { detail: EventDetail; issue: Issue }) {
  const data = props.detail.data as Record<string, any>
  const ev = props.detail.event

  // Exception extraction
  const exception = data?.exception?.values?.[data.exception.values.length - 1] as
    | { type?: string; value?: string; stacktrace?: { frames?: FrameLike[] } }
    | undefined

  const frames = (): FrameLike[] => {
    const f = exception?.stacktrace?.frames
    return Array.isArray(f) ? f : []
  }

  const breadcrumbs = (): Array<Record<string, any>> => {
    const b = data?.breadcrumbs
    if (Array.isArray(b)) return b
    if (b && Array.isArray(b.values)) return b.values
    return []
  }

  const tags = (): Array<[string, string]> => {
    const t = data?.tags
    if (t && typeof t === 'object' && !Array.isArray(t)) {
      return Object.entries(t).map(([k, v]) => [k, String(v)])
    }
    if (Array.isArray(t)) {
      return t
        .filter((p): p is [unknown, unknown] => Array.isArray(p) && p.length === 2)
        .map(([k, v]) => [String(k), String(v)])
    }
    return []
  }

  return (
    <div class="flex flex-col gap-8">
      <Section title="exception">
        <Show
          when={exception}
          fallback={<p class="text-ink-400 text-[12px]">// no exception in payload</p>}
        >
          {(exc) => (
            <div class="flex flex-col gap-3">
              <p class="font-mono text-[14px] text-ink-50">
                <span class="text-crash">{exc().type}</span>
                <Show when={exc().value}>: <span>{exc().value}</span></Show>
              </p>
              <div class="flex flex-col gap-1">
                <For each={frames().slice().reverse()}>
                  {(frame, i) => <Frame frame={frame} topmost={i() === 0} />}
                </For>
              </div>
            </div>
          )}
        </Show>
      </Section>

      <Show when={breadcrumbs().length > 0}>
        <Section title={`breadcrumbs (${breadcrumbs().length})`}>
          <ol class="flex flex-col gap-1 text-[12px]">
            <For each={breadcrumbs()}>
              {(b) => (
                <li class="flex gap-3 text-ink-200">
                  <span class="text-ink-500 w-24 shrink-0">{b.category ?? '—'}</span>
                  <span class="font-mono truncate">{b.message ?? ''}</span>
                  <span class="ml-auto text-ink-500">{b.level ?? ''}</span>
                </li>
              )}
            </For>
          </ol>
        </Section>
      </Show>

      <Show when={tags().length > 0}>
        <Section title="tags">
          <div class="flex flex-wrap gap-2">
            <For each={tags()}>
              {([k, v]) => (
                <span class="text-[12px] border border-ink-600 px-2 py-[2px]">
                  <span class="text-ink-400">{k}:</span> {v}
                </span>
              )}
            </For>
          </div>
        </Section>
      </Show>

      <Show when={ev.user_email || ev.user_id || data?.user}>
        <Section title="user">
          <div class="flex flex-col gap-1 text-[12px] text-ink-200">
            <Show when={ev.user_email}>
              {(em) => <p>email: {em()}</p>}
            </Show>
            <Show when={ev.user_id}>
              {(id) => <p>id: {id()}</p>}
            </Show>
          </div>
        </Section>
      </Show>

      <Show when={ev.request_url || data?.request}>
        <Section title="request">
          <div class="flex flex-col gap-1 text-[12px] text-ink-200">
            <Show when={ev.request_url}>
              {(u) => <p class="font-mono break-all">{u()}</p>}
            </Show>
          </div>
        </Section>
      </Show>

      <Section title="raw json">
        <pre class="text-[11px] font-mono p-3 bg-ink-700/40 border border-ink-600 overflow-x-auto whitespace-pre">
{JSON.stringify(props.detail.data, null, 2)}
        </pre>
      </Section>
    </div>
  )
}

function Section(props: { title: string; children: any }) {
  const [open, setOpen] = createSignal(true)
  return (
    <section>
      <button
        onClick={() => setOpen(!open())}
        class="text-[11px] text-ink-400 hover:text-ink-100 mb-2 font-mono"
      >
        [{open() ? '−' : '+'}] // {props.title}
      </button>
      <Show when={open()}>{props.children}</Show>
    </section>
  )
}

function Frame(props: { frame: FrameLike; topmost: boolean }) {
  const inApp = () => props.frame.in_app === true
  const file = () =>
    props.frame.filename ?? props.frame.abs_path ?? props.frame.module ?? '<unknown>'
  return (
    <div
      class={`flex items-baseline gap-3 px-3 py-1 border-l-2 ${
        inApp() ? 'border-crash bg-ink-700/20' : 'border-ink-600'
      } ${inApp() ? '' : 'opacity-60'}`}
    >
      <span class="text-ink-50 font-mono text-[12px]">
        {props.frame.function ?? '<anon>'}
      </span>
      <span class="text-ink-400 font-mono text-[11px]">
        {file()}
        <Show when={props.frame.lineno != null}>
          :<span class="text-ink-200">{props.frame.lineno}</span>
        </Show>
      </span>
      <Show when={props.topmost}>
        <span class="ml-auto text-[10px] text-crash uppercase">↑ top</span>
      </Show>
    </div>
  )
}

// Re-export relTime so this file's structure stays single-import.
export { relTime }
