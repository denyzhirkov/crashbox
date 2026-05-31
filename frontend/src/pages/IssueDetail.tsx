import { A, useNavigate, useParams } from '@solidjs/router'
import { createResource, createSignal, For, onCleanup, Show } from 'solid-js'
import { api } from '../api/client'
import type { EventDetail, EventRow, Issue, SnoozeAction } from '../api/types'
import { JsonViewer } from '../components/JsonViewer'
import { Breadcrumb, Page } from '../components/layout'
import { fmt, Icon, SevCue, Voice } from '../components/primitives'
import { absTime, relTime } from '../lib/time'

type FrameLike = {
  function?: string
  filename?: string
  abs_path?: string
  module?: string
  lineno?: number | string
  colno?: number | string
  in_app?: boolean
  pre_context?: string[]
  context_line?: string
  post_context?: string[]
}

const SEV_COLOR: Record<string, string> = {
  fatal: 'var(--sev-fatal)', error: 'var(--sev-error)', warning: 'var(--sev-warning)',
  info: 'var(--sev-info)', debug: 'var(--sev-debug)',
}

export default function IssueDetailPage() {
  const params = useParams<{ issueId: string }>()
  const issueId = () => Number(params.issueId)
  const [issue, { refetch: refetchIssue }] = createResource(issueId, (id) => api.issues.get(id))
  const [events] = createResource(issueId, (id) => api.issues.events(id))
  const [activeEventId, setActiveEventId] = createSignal<number | null>(null)

  const currentEventId = () => activeEventId() ?? events()?.[0]?.id ?? null
  const [eventDetail] = createResource(currentEventId, (id) =>
    id != null ? api.events.get(id) : Promise.resolve(null),
  )

  const toggleStatus = async () => {
    const i = issue()
    if (!i) return
    await api.issues.setStatus(i.id, i.status === 'resolved' ? 'unresolved' : 'resolved')
    refetchIssue()
  }

  const [showSnooze, setShowSnooze] = createSignal(false)
  const snooze = async (action: SnoozeAction) => {
    const i = issue()
    if (!i) return
    await api.issues.snooze(i.id, action)
    setShowSnooze(false)
    refetchIssue()
  }
  const isSnoozed = () => {
    const i = issue()
    if (!i?.snoozed_until) return false
    if (i.snoozed_until === 'forever') return true
    return Date.parse(i.snoozed_until) > Date.now()
  }

  return (
    <Page>
      <Show when={issue()} fallback={<Voice>loading…</Voice>}>
        {(i) => (
          <>
            <div style={{ display: 'flex', 'align-items': 'flex-start', 'justify-content': 'space-between', 'margin-bottom': '16px' }}>
              <Breadcrumb
                items={[
                  { label: 'projects', href: '/projects' },
                  { label: 'issues', href: `/projects/${i().project_id}/issues` },
                  { label: `#${i().id}` },
                ]}
              />
              <A href={`/projects/${i().project_id}/issues`} class="btn ghost sm"><Icon name="arrow" size={13} /> back to issues</A>
            </div>

            {/* header */}
            <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '16px', 'margin-bottom': '14px' }}>
              <SevCue level={i().level} variant="glow" style={{ height: '50px', 'margin-top': '2px' }} />
              <div style={{ flex: '1', 'min-width': 0 }}>
                <h1 class="mono" style={{ 'font-size': '19px', 'font-weight': 600, 'line-height': 1.35, color: 'var(--text-hi)', 'margin-bottom': '10px', 'word-break': 'break-word' }}>
                  {i().title}
                </h1>
                <div style={{ display: 'flex', 'align-items': 'center', gap: '16px', 'flex-wrap': 'wrap', 'row-gap': '8px' }}>
                  <MetaItem label="events"><span class="tnum">{fmt(i().event_count)}</span></MetaItem>
                  <MetaItem label="first">{relTime(i().first_seen)}</MetaItem>
                  <MetaItem label="last">{relTime(i().last_seen)}</MetaItem>
                  <Show when={i().platform}><MetaItem label="platform">{i().platform}</MetaItem></Show>
                  <span class="pill"><span class="d" style={{ background: SEV_COLOR[i().level ?? 'error'] ?? 'var(--sev-error)' }} />{i().level ?? 'error'}</span>
                  <span class="pill" style={{ color: i().status === 'resolved' ? 'var(--text-faint)' : 'var(--text-mid)' }}>
                    {isSnoozed() ? 'snoozed' : i().status}
                  </span>
                </div>
              </div>
              <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '8px', flex: 'none', position: 'relative' }}>
                <div style={{ position: 'relative' }}>
                  <button class="btn sm" onClick={(e) => { e.stopPropagation(); setShowSnooze((o) => !o) }}>
                    <Icon name="snooze" size={13} /> {isSnoozed() ? 'snoozed' : 'snooze'} <Icon name="chevron" size={11} />
                  </button>
                  <Show when={showSnooze()}>
                    <SnoozeMenu onPick={snooze} onClose={() => setShowSnooze(false)} snoozed={isSnoozed()} />
                  </Show>
                </div>
                <Show
                  when={i().status === 'resolved'}
                  fallback={<button class="btn sm primary" onClick={toggleStatus}><Icon name="check" size={13} /> mark fixed</button>}
                >
                  <button class="btn sm" onClick={toggleStatus}><Icon name="refresh" size={13} /> reopen</button>
                </Show>
              </div>
            </div>

            <Scrubber events={events() ?? []} activeId={currentEventId()} onPick={setActiveEventId} />

            <Show when={eventDetail()} fallback={<Voice>loading event…</Voice>}>
              {(detail) => <EventBody detail={detail()} issue={i()} />}
            </Show>
          </>
        )}
      </Show>
    </Page>
  )
}

function MetaItem(props: { label: string; children: any }) {
  return (
    <div style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}>
      <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)' }}>{props.label}</span>
      <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-mid)' }}>{props.children}</span>
    </div>
  )
}

function SnoozeMenu(props: { onPick: (a: SnoozeAction) => void; onClose: () => void; snoozed: boolean }) {
  const opts: Array<[SnoozeAction, string, string]> = [
    ['1h', 'snooze 1 hour', '1h'],
    ['1d', 'snooze 1 day', '1d'],
    ['1w', 'snooze 1 week', '1w'],
    ['forever', 'until next crash', '⏎'],
    ['wake', 'wake now', 'w'],
  ]
  const onWin = () => props.onClose()
  window.addEventListener('click', onWin)
  onCleanup(() => window.removeEventListener('click', onWin))
  return (
    <div
      class="card"
      onClick={(e) => e.stopPropagation()}
      style={{ position: 'absolute', top: '40px', right: 0, width: '190px', padding: '5px', 'z-index': 50, background: 'var(--bg-float)', 'box-shadow': 'var(--shadow-pop)' }}
    >
      <For each={opts}>
        {([action, label, key]) => (
          <Show when={action !== 'wake' || props.snoozed}>
            <button
              class="cb-menuitem"
              onClick={() => props.onPick(action)}
              style={{ display: 'flex', 'align-items': 'center', gap: '12px', width: '100%', 'text-align': 'left', background: 'none', border: 'none', cursor: 'pointer', padding: '8px 10px', 'border-radius': '6px', color: action === 'wake' ? 'var(--accent-cyan)' : 'var(--text)' }}
            >
              <Icon name={action === 'wake' ? 'bolt' : 'snooze'} size={13} style={{ color: 'var(--text-faint)' }} />
              <span class="mono" style={{ 'font-size': '12.5px' }}>{label}</span>
              <span style={{ flex: '1' }} />
              <span class="kbd">{key}</span>
            </button>
          </Show>
        )}
      </For>
    </div>
  )
}

function Scrubber(props: { events: EventRow[]; activeId: number | null; onPick: (id: number) => void }) {
  // oldest left → latest right
  const ordered = () => props.events.slice().reverse()
  const current = () => props.events.find((e) => e.id === props.activeId) ?? props.events[0]
  return (
    <Show when={props.events.length > 1}>
      <div class="card" style={{ padding: '14px 18px', 'margin-bottom': '14px' }}>
        <div style={{ display: 'flex', 'align-items': 'center', 'justify-content': 'space-between', 'margin-bottom': '10px' }}>
          <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-mid)' }}>
            events <span class="tnum" style={{ color: 'var(--text-faint)' }}>· {props.events.length} sampled</span>
          </span>
          <Show when={current()}>
            {(c) => (
              <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)' }}>
                <span style={{ color: 'var(--text-hi)' }}>{c().event_id ?? `#${c().id}`}</span> · {relTime(c().received_at)}
              </span>
            )}
          </Show>
        </div>
        <div style={{ display: 'flex', gap: '3px', 'align-items': 'flex-end', height: '30px' }}>
          <For each={ordered()}>
            {(ev) => {
              const active = () => ev.id === props.activeId || (props.activeId == null && ev.id === props.events[0]?.id)
              return (
                <button
                  class="cb-tick"
                  onClick={() => props.onPick(ev.id)}
                  title={absTime(ev.received_at)}
                  style={{
                    flex: '1', height: active() ? '28px' : '16px', 'min-width': '4px', 'border-radius': '3px', border: 'none', cursor: 'pointer', padding: 0,
                    background: active() ? 'var(--accent-grad-v)' : 'oklch(1 0 0 / 0.1)',
                    'box-shadow': active() ? '0 0 12px -2px var(--accent-glow)' : 'none',
                    transition: 'height 0.14s, background 0.14s',
                  }}
                />
              )
            }}
          </For>
        </div>
        <div style={{ display: 'flex', 'justify-content': 'space-between', 'margin-top': '8px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)' }}>← older</span>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)' }}>latest →</span>
        </div>
      </div>
    </Show>
  )
}

function Section(props: { title: string; count?: number; accent?: boolean; defaultOpen?: boolean; children: any }) {
  const [open, setOpen] = createSignal(props.defaultOpen ?? true)
  return (
    <section class="card" style={{ padding: 0, overflow: 'hidden', 'margin-bottom': '14px' }}>
      <header
        onClick={() => setOpen((o) => !o)}
        style={{ display: 'flex', 'align-items': 'center', gap: '12px', padding: '13px 18px', cursor: 'pointer', 'user-select': 'none', 'border-bottom': open() ? '1px solid var(--line-soft)' : 'none' }}
      >
        <span class="mono" style={{ display: 'inline-flex', 'align-items': 'center', 'justify-content': 'center', border: '1px solid var(--line)', 'border-radius': '5px', width: '20px', height: '20px', color: 'var(--text-mid)', flex: 'none', 'font-size': '13px', 'line-height': 1 }}>
          {open() ? '−' : '+'}
        </span>
        <span class="mono" style={{ 'font-size': '13px', 'font-weight': 600, color: props.accent ? 'var(--accent-ink)' : 'var(--text-hi)', 'letter-spacing': '0.02em' }}>{props.title}</span>
        <Show when={props.count != null}>
          <span class="mono tnum" style={{ 'font-size': '12px', color: 'var(--text-faint)' }}>{props.count}</span>
        </Show>
      </header>
      <Show when={open()}><div style={{ padding: '18px' }}>{props.children}</div></Show>
    </section>
  )
}

function buildCtx(frame: FrameLike): Array<[number, string]> | null {
  const line = typeof frame.lineno === 'string' ? Number(frame.lineno) : frame.lineno
  if (frame.context_line == null || line == null || Number.isNaN(line)) return null
  const pre = frame.pre_context ?? []
  const post = frame.post_context ?? []
  const out: Array<[number, string]> = []
  pre.forEach((code, idx) => out.push([line - pre.length + idx, code]))
  out.push([line, frame.context_line])
  post.forEach((code, idx) => out.push([line + 1 + idx, code]))
  return out
}

function StackFrame(props: { frame: FrameLike; top: boolean }) {
  const ctx = () => buildCtx(props.frame)
  const inApp = () => props.frame.in_app === true
  const [open, setOpen] = createSignal(props.top && !!ctx())
  const file = () => props.frame.filename ?? props.frame.abs_path ?? props.frame.module ?? '<unknown>'
  return (
    <div style={{ 'border-radius': 'var(--r-md)', border: '1px solid var(--line-soft)', overflow: 'hidden', background: inApp() ? 'oklch(1 0 0 / 0.018)' : 'transparent', opacity: inApp() ? 1 : 0.62 }}>
      <div onClick={() => ctx() && setOpen((o) => !o)} style={{ display: 'flex', 'align-items': 'center', gap: '12px', padding: '9px 13px', cursor: ctx() ? 'pointer' : 'default' }}>
        <Show when={props.top}>
          <span class="mono" style={{ 'font-size': '9.5px', 'font-weight': 600, 'letter-spacing': '0.06em', color: 'var(--accent-ink)', border: '1px solid var(--accent-faint)', 'border-radius': '4px', padding: '2px 5px', flex: 'none' }}>TOP</span>
        </Show>
        <span class="mono" style={{ 'font-size': '13px', color: inApp() ? 'var(--text-hi)' : 'var(--text-lo)', flex: 'none' }}>{props.frame.function ?? '<anon>'}</span>
        <span class="mono" style={{ flex: '1', 'min-width': 0, overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap', 'font-size': '12px', color: 'var(--text-faint)' }}>
          {file()}
          <Show when={props.frame.lineno != null}>
            <span style={{ color: 'var(--text-lo)' }}>:{props.frame.lineno}{props.frame.colno != null ? `:${props.frame.colno}` : ''}</span>
          </Show>
        </span>
        <Show when={!inApp()}><span class="mono" style={{ 'font-size': '10.5px', color: 'var(--text-faint)', flex: 'none' }}>vendor</span></Show>
        <Show when={ctx()}>
          <span style={{ color: 'var(--text-faint)', transform: open() ? 'none' : 'rotate(-90deg)', transition: 'transform 0.12s', display: 'inline-flex' }}><Icon name="chevron" size={12} /></span>
        </Show>
      </div>
      <Show when={open() && ctx()}>
        <div class="codeblk" style={{ 'border-radius': 0, border: 'none', 'border-top': '1px solid var(--line-soft)', padding: '8px 0', 'font-size': '12.5px' }}>
          <For each={ctx()!}>
            {([ln, code]) => {
              const hot = ln === (typeof props.frame.lineno === 'string' ? Number(props.frame.lineno) : props.frame.lineno)
              return (
                <div style={{ display: 'flex', background: hot ? 'oklch(0.690 0.150 45 / 0.08)' : 'transparent', padding: '1px 14px' }}>
                  <span class="tnum" style={{ width: '40px', color: hot ? 'var(--sev-error)' : 'var(--text-faint)', 'text-align': 'right', 'padding-right': '14px', flex: 'none', 'user-select': 'none' }}>{ln}</span>
                  <span style={{ color: hot ? 'var(--text-hi)' : 'var(--text-mid)', 'white-space': 'pre' }}>{code}</span>
                </div>
              )
            }}
          </For>
        </div>
      </Show>
    </div>
  )
}

function EventBody(props: { detail: EventDetail; issue: Issue }) {
  const nav = useNavigate()
  const data = () => props.detail.data as Record<string, any>
  const ev = () => props.detail.event

  const exception = () => {
    const vals = data()?.exception?.values
    return Array.isArray(vals) ? (vals[vals.length - 1] as { type?: string; value?: string; stacktrace?: { frames?: FrameLike[] } }) : undefined
  }
  const frames = (): FrameLike[] => {
    const f = exception()?.stacktrace?.frames
    return Array.isArray(f) ? f.slice().reverse() : []
  }
  const breadcrumbs = (): Array<Record<string, any>> => {
    const b = data()?.breadcrumbs
    if (Array.isArray(b)) return b
    if (b && Array.isArray(b.values)) return b.values
    return []
  }
  const tags = (): Array<[string, string]> => {
    const t = data()?.tags
    if (t && typeof t === 'object' && !Array.isArray(t)) return Object.entries(t).map(([k, v]) => [k, String(v)])
    if (Array.isArray(t)) return t.filter((p): p is [unknown, unknown] => Array.isArray(p) && p.length === 2).map(([k, v]) => [String(k), String(v)])
    return []
  }
  const bcLevel = (lvl: string | undefined) => (lvl === 'error' || lvl === 'fatal' ? 'error' : lvl === 'warning' ? 'warning' : 'info')

  return (
    <>
      <Section title="exception" defaultOpen>
        <Show when={exception()} fallback={<Voice>no exception in payload</Voice>}>
          {(exc) => (
            <>
              <div style={{ 'margin-bottom': '14px' }}>
                <span class="mono" style={{ 'font-size': '14px', 'font-weight': 600, color: SEV_COLOR[props.issue.level ?? 'error'] ?? 'var(--sev-error)' }}>{exc().type}</span>
                <Show when={exc().value}>
                  <div class="mono" style={{ 'font-size': '13.5px', color: 'var(--text)', 'margin-top': '4px', 'line-height': 1.5 }}>{exc().value}</div>
                </Show>
              </div>
              <div style={{ display: 'flex', 'flex-direction': 'column', gap: '8px' }}>
                <For each={frames()}>{(f, i) => <StackFrame frame={f} top={i() === 0} />}</For>
              </div>
            </>
          )}
        </Show>
      </Section>

      <Show when={breadcrumbs().length > 0}>
        <Section title="breadcrumbs" count={breadcrumbs().length}>
          <div style={{ display: 'flex', 'flex-direction': 'column' }}>
            <For each={breadcrumbs()}>
              {(b, i) => (
                <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '12px', padding: '6px 0', 'border-bottom': i() < breadcrumbs().length - 1 ? '1px solid var(--line-soft)' : 'none' }}>
                  <span class="mono tnum" style={{ 'font-size': '11.5px', color: 'var(--text-faint)', width: '64px', flex: 'none', 'text-align': 'right' }}>{b.timestamp ? relTime(b.timestamp) : ''}</span>
                  <SevCue level={bcLevel(b.level)} variant="dot" style={{ 'margin-top': '5px' }} />
                  <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-lo)', width: '90px', flex: 'none' }}>{b.category ?? '—'}</span>
                  <span class="mono" style={{ flex: '1', 'min-width': 0, 'font-size': '12.5px', color: 'var(--text)' }}>{b.message ?? ''}</span>
                </div>
              )}
            </For>
          </div>
        </Section>
      </Show>

      <Show when={tags().length > 0}>
        <Section title="tags" count={tags().length}>
          <div style={{ display: 'flex', gap: '8px', 'flex-wrap': 'wrap' }}>
            <For each={tags()}>
              {([k, v]) => (
                <button
                  class="chip mono cb-tagchip"
                  onClick={() => nav(`/projects/${props.issue.project_id}/issues?tag=${encodeURIComponent(`${k}=${v}`)}`)}
                  title={`filter issues by ${k}:${v}`}
                >
                  <span style={{ color: 'var(--text-faint)' }}>{k}</span>
                  <span style={{ color: 'var(--text-hi)' }}>{v}</span>
                </button>
              )}
            </For>
          </div>
        </Section>
      </Show>

      <Show when={ev().user_email || ev().user_id || data()?.user}>
        <Section title="user" defaultOpen={false}>
          <div style={{ display: 'flex', 'flex-direction': 'column', gap: '8px' }}>
            <Show when={ev().user_email}>{(em) => <UserRow k="email" v={em()} />}</Show>
            <Show when={ev().user_id}>{(id) => <UserRow k="id" v={id()} />}</Show>
          </div>
        </Section>
      </Show>

      <Show when={ev().request_url || data()?.request}>
        <Section title="request" defaultOpen={false}>
          <div style={{ display: 'flex', 'flex-direction': 'column', gap: '8px' }}>
            <Show when={ev().request_url}>{(u) => <UserRow k="url" v={u()} />}</Show>
          </div>
        </Section>
      </Show>

      <Section title="raw json" defaultOpen={false}>
        <JsonViewer data={props.detail.data} />
      </Section>
    </>
  )
}

function UserRow(props: { k: string; v: string }) {
  return (
    <div style={{ display: 'flex', gap: '12px' }}>
      <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-faint)', width: '80px', flex: 'none' }}>{props.k}</span>
      <span class="mono" style={{ 'font-size': '12.5px', color: 'var(--text)', 'word-break': 'break-all' }}>{props.v}</span>
    </div>
  )
}
