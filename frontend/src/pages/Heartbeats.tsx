import { useParams } from '@solidjs/router'
import { createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js'
import { api } from '../api/client'
import type { HeartbeatMonitor, HeartbeatStatus } from '../api/types'
import { Breadcrumb, Page, ProjectNav } from '../components/layout'
import { CadenceLane, CopyBlock, Icon, SevCue, Voice } from '../components/primitives'
import { useAuth } from '../lib/auth-context'
import { relTime } from '../lib/time'

// Dead-man's-switch monitors for one project, rendered as a TAPE (like the issues list):
// severity edge cue → name → cadence lane → due countdown. All state transitions happen
// server-side (ping endpoint + sweep job); this page only renders and edits. The cadence
// lane + "due / overdue" countdown are client-side, driven by a 1s tick; the server sweep
// remains the authority.

const REFRESH_MS = 30_000 // matches the default sweep interval

// Map monitor status onto the design system's severity levels (for the edge cue colour).
const SEV: Record<HeartbeatStatus, string> = {
  up: 'info',
  down: 'error',
  pending: 'debug',
  paused: 'warning',
}

// Sort so what needs attention floats up: down first, then up (soonest-due first),
// then pending, then paused. "Whisper" urgency = ordering + colour, never motion.
const STATUS_RANK: Record<HeartbeatStatus, number> = { down: 0, up: 1, pending: 2, paused: 3 }

const UNIT_SECONDS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86_400 }

function fmtPeriod(secs: number): string {
  if (secs % 86_400 === 0 && secs >= 86_400) return `${secs / 86_400}d`
  if (secs % 3600 === 0 && secs >= 3600) return `${secs / 3600}h`
  if (secs % 60 === 0 && secs >= 60) return `${secs / 60}m`
  return `${secs}s`
}

/** Split raw seconds into {value, unit} choosing the largest whole unit. */
function splitPeriod(secs: number): { value: number; unit: string } {
  for (const u of ['d', 'h', 'm'] as const) {
    const m = UNIT_SECONDS[u]
    if (secs % m === 0 && secs >= m) return { value: secs / m, unit: u }
  }
  return { value: secs, unit: 's' }
}

type DueInfo = { text: string; color: string; phase: 'ok' | 'grace' | 'overdue' | 'paused' | 'pending' }

/** "due in 42m" / "late · grace 3m left" / "overdue by 3m", relative to last_ping. */
function dueInfo(m: HeartbeatMonitor, nowMs: number): DueInfo {
  if (m.status === 'pending') return { text: 'waiting for first ping', color: 'var(--text-faint)', phase: 'pending' }
  if (m.status === 'paused') return { text: 'paused', color: 'var(--text-faint)', phase: 'paused' }
  if (!m.last_ping_at) return { text: '—', color: 'var(--text-faint)', phase: 'ok' }
  const last = Date.parse(m.last_ping_at)
  if (Number.isNaN(last)) return { text: '—', color: 'var(--text-faint)', phase: 'ok' }
  const dueAt = last + m.period_seconds * 1000
  const deadline = dueAt + m.grace_seconds * 1000
  if (nowMs <= dueAt) return { text: `due in ${fmtPeriod(Math.max(1, Math.round((dueAt - nowMs) / 1000)))}`, color: 'var(--text-lo)', phase: 'ok' }
  if (nowMs <= deadline) return { text: `late · grace ${fmtPeriod(Math.max(1, Math.round((deadline - nowMs) / 1000)))} left`, color: 'var(--sev-warning)', phase: 'grace' }
  return { text: `overdue by ${fmtPeriod(Math.max(1, Math.round((nowMs - deadline) / 1000)))}`, color: 'var(--sev-error)', phase: 'overdue' }
}

export default function HeartbeatsPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const [monitors, { refetch }] = createResource(projectId, (id) => api.heartbeats.list(id))
  const { user } = useAuth()
  const isAdmin = () => user()?.is_admin === true

  const [creating, setCreating] = createSignal(false)
  const [editingId, setEditingId] = createSignal<number | null>(null)
  // 1s tick drives the cadence lane + countdown; 30s refetch picks up server-side flips.
  const [nowMs, setNowMs] = createSignal(Date.now())
  const tick = setInterval(() => setNowMs(Date.now()), 1000)
  const poll = setInterval(() => void refetch(), REFRESH_MS)
  onCleanup(() => {
    clearInterval(tick)
    clearInterval(poll)
  })

  const sorted = createMemo(() => {
    const list = monitors() ?? []
    return [...list].sort((a, b) => {
      const r = STATUS_RANK[a.status] - STATUS_RANK[b.status]
      if (r !== 0) return r
      if (a.status === 'up' && b.status === 'up') {
        const da = a.last_ping_at ? Date.parse(a.last_ping_at) + a.period_seconds * 1000 : Infinity
        const db = b.last_ping_at ? Date.parse(b.last_ping_at) + b.period_seconds * 1000 : Infinity
        return da - db
      }
      return 0
    })
  })

  const downCount = createMemo(() => (monitors() ?? []).filter((m) => m.status === 'down').length)

  const setStatus = async (m: HeartbeatMonitor, status: 'paused' | 'pending') => {
    await api.heartbeats.update(m.id, { status })
    void refetch()
  }
  const remove = async (m: HeartbeatMonitor) => {
    if (!window.confirm(`Delete monitor "${m.name}"? Its ping URL stops working immediately.`)) return
    await api.heartbeats.remove(m.id)
    void refetch()
  }

  return (
    <Page>
      <div style={{ display: 'flex', 'align-items': 'flex-start', 'justify-content': 'space-between', 'margin-bottom': '16px' }}>
        <Breadcrumb
          items={[
            { label: 'projects', href: '/projects' },
            { label: project()?.name ?? '…', href: `/projects/${projectId()}/issues` },
            { label: 'heartbeats' },
          ]}
        />
        <ProjectNav projectId={projectId()} current="heartbeats" />
      </div>

      <div style={{ display: 'flex', 'align-items': 'center', gap: '16px', 'margin-bottom': '18px' }}>
        <h1 class="mono" style={{ 'font-size': '22px', 'font-weight': 600 }}>heartbeats</h1>
        <span class="mono tnum" style={{ 'font-size': '13px', color: 'var(--text-faint)' }}>
          {(monitors() ?? []).length}
        </span>
        <Show when={downCount() > 0}>
          <span class="chip mono" style={{ color: 'var(--sev-error)', 'border-color': 'var(--sev-error)' }}>
            {downCount()} down
          </span>
        </Show>
        <div style={{ flex: '1' }} />
        <Show when={isAdmin() && !creating()}>
          <button class="btn sm primary" onClick={() => setCreating(true)}>
            <Icon name="plus" size={13} /> new monitor
          </button>
        </Show>
      </div>

      <Show when={creating()}>
        <div style={{ 'margin-bottom': '16px' }}>
          <MonitorForm
            onCancel={() => setCreating(false)}
            onSaved={() => { setCreating(false); void refetch() }}
            submit={(body) => api.heartbeats.create(projectId(), body)}
          />
        </div>
      </Show>

      <Show when={!monitors.loading || monitors()} fallback={<div class="skel" style={{ height: '120px' }} />}>
        <Show
          when={(monitors() ?? []).length > 0}
          fallback={
            <div class="card" style={{ padding: '40px', 'text-align': 'center' }}>
              <Voice>
                no heartbeat monitors yet. create one and curl its ping URL from the end of a cron job —
                silence becomes an alert.
              </Voice>
            </div>
          }
        >
          {/* the tape: one card, hairline-divided rows */}
          <div class="card" style={{ overflow: 'hidden' }}>
            <For each={sorted()}>
              {(m, i) => (
                <Show
                  when={editingId() !== m.id}
                  fallback={
                    <div style={{ padding: '4px 4px 8px', 'border-top': i() > 0 ? '1px solid var(--line-soft)' : 'none' }}>
                      <MonitorForm
                        initial={m}
                        embedded
                        onCancel={() => setEditingId(null)}
                        onSaved={() => { setEditingId(null); void refetch() }}
                        submit={(body) => api.heartbeats.update(m.id, body)}
                      />
                    </div>
                  }
                >
                  <MonitorRow
                    monitor={m}
                    first={i() === 0}
                    nowMs={nowMs()}
                    isAdmin={isAdmin()}
                    onEdit={() => setEditingId(m.id)}
                    onPause={() => void setStatus(m, 'paused')}
                    onResume={() => void setStatus(m, 'pending')}
                    onDelete={() => void remove(m)}
                  />
                </Show>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </Page>
  )
}

function MonitorRow(props: {
  monitor: HeartbeatMonitor
  first: boolean
  nowMs: number
  isAdmin: boolean
  onEdit: () => void
  onPause: () => void
  onResume: () => void
  onDelete: () => void
}) {
  const m = () => props.monitor
  const paused = () => m().status === 'paused'
  const info = () => dueInfo(m(), props.nowMs)
  const [showUrl, setShowUrl] = createSignal(false)

  return (
    <div class="cb-hbrow" style={{ 'border-top': props.first ? 'none' : '1px solid var(--line-soft)', opacity: paused() ? 0.62 : 1 }}>
      <div style={{ display: 'flex', 'align-items': 'center', gap: '14px', padding: '13px 16px 13px 14px' }}>
        {/* whisper edge cue — same muted 3px severity bar as everywhere else */}
        <SevCue level={SEV[m().status]} variant="bar" style={{ height: '20px', 'align-self': 'center' }} />

        <span
          class="mono"
          style={{ 'font-size': '13.5px', 'font-weight': 500, color: 'var(--text-hi)', width: '210px', flex: 'none', overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}
          title={m().name}
        >
          {m().name}
        </span>

        <CadenceLane
          status={m().status}
          lastPingAt={m().last_ping_at}
          periodSeconds={m().period_seconds}
          graceSeconds={m().grace_seconds}
          now={props.nowMs}
        />

        <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)', width: '92px', flex: 'none', 'text-align': 'right' }}>
          every {fmtPeriod(m().period_seconds)}
        </span>

        <span
          class="mono"
          style={{ 'font-size': '12px', color: info().color, width: '156px', flex: 'none', 'text-align': 'right' }}
          title={m().last_ping_at ? `last ping ${relTime(m().last_ping_at)}` : 'never pinged'}
        >
          {info().text}
        </span>

        <div style={{ display: 'flex', gap: '4px', flex: 'none' }}>
          <button class="btn ghost sm" onClick={() => setShowUrl((v) => !v)} title="ping URL">
            <Icon name="copy" size={12} />
          </button>
          <Show when={props.isAdmin}>
            <button class="btn ghost sm" onClick={paused() ? props.onResume : props.onPause} title={paused() ? 'resume (waits for the next ping)' : 'pause (stops alerting)'}>
              <Icon name={paused() ? 'enter' : 'snooze'} size={12} />
            </button>
            <button class="btn ghost sm" onClick={props.onEdit}>edit</button>
            <button class="btn ghost sm" onClick={props.onDelete} title="delete — invalidates the ping URL">
              <Icon name="x" size={12} />
            </button>
          </Show>
        </div>
      </div>

      <Show when={showUrl()}>
        <div style={{ padding: '0 16px 14px 31px' }}>
          <CopyBlock value={m().ping_url} />
        </div>
      </Show>
    </div>
  )
}

function MonitorForm(props: {
  initial?: HeartbeatMonitor
  embedded?: boolean
  submit: (body: { name: string; period_seconds: number; grace_seconds: number }) => Promise<unknown>
  onCancel: () => void
  onSaved: () => void
}) {
  const startPeriod = splitPeriod(props.initial?.period_seconds ?? 3600)
  const [name, setName] = createSignal(props.initial?.name ?? '')
  const [periodValue, setPeriodValue] = createSignal(String(startPeriod.value))
  const [periodUnit, setPeriodUnit] = createSignal(startPeriod.unit)
  const [graceSec, setGraceSec] = createSignal(props.initial?.grace_seconds ?? 60)
  const [graceCustom, setGraceCustom] = createSignal(false)
  const [busy, setBusy] = createSignal(false)
  const [err, setErr] = createSignal<string | null>(null)

  const GRACE_PRESETS: Array<{ label: string; v: number }> = [
    { label: '30s', v: 30 },
    { label: '1m', v: 60 },
    { label: '5m', v: 300 },
    { label: '15m', v: 900 },
  ]

  const periodSeconds = () => Math.round(Number(periodValue()) * (UNIT_SECONDS[periodUnit()] ?? 1))
  const valid = () => name().trim().length > 0 && periodSeconds() >= 10 && periodSeconds() <= 86_400 * 30 && graceSec() >= 0 && graceSec() <= 86_400

  const submit = async () => {
    if (!valid()) return
    setBusy(true)
    setErr(null)
    try {
      await props.submit({ name: name().trim(), period_seconds: periodSeconds(), grace_seconds: graceSec() })
      props.onSaved()
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const unitBtn = (u: string, label: string) => (
    <button
      onClick={() => setPeriodUnit(u)}
      class="mono"
      style={{
        height: '38px', padding: '0 13px', border: 'none', cursor: 'pointer',
        'border-right': u === 'd' ? 'none' : '1px solid var(--line)',
        background: periodUnit() === u ? 'oklch(1 0 0 / 0.10)' : 'transparent',
        color: periodUnit() === u ? 'var(--text-hi)' : 'var(--text-faint)', 'font-size': '12.5px',
      }}
    >
      {label}
    </button>
  )

  return (
    <div class="card" style={{ padding: props.embedded ? '16px' : '20px', ...(props.embedded ? { background: 'var(--bg-sunken)' } : {}) }}>
      <div class="mono" style={{ 'font-size': '13px', color: 'var(--text-hi)', 'margin-bottom': '14px' }}>
        // {props.initial ? `edit ${props.initial.name}` : 'new heartbeat monitor'}
      </div>

      <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '16px', 'flex-wrap': 'wrap' }}>
        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', flex: '1', 'min-width': '200px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>name</span>
          <span class="field cb-focusring">
            <input
              class="input mono"
              autofocus
              placeholder="nightly-db-backup"
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => e.key === 'Enter' && submit()}
            />
          </span>
          <span class="mono" style={{ 'font-size': '10px', color: 'var(--text-faint)' }}>what pings us</span>
        </label>

        <div style={{ display: 'flex', 'flex-direction': 'column', gap: '6px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>every</span>
          <div style={{ display: 'flex', gap: '8px' }}>
            <span class="field cb-focusring" style={{ width: '78px' }}>
              <input
                class="input mono tnum"
                type="number"
                min="1"
                value={periodValue()}
                onInput={(e) => setPeriodValue(e.currentTarget.value)}
                onKeyDown={(e) => e.key === 'Enter' && submit()}
              />
            </span>
            <div style={{ display: 'flex', border: '1px solid var(--line)', 'border-radius': 'var(--r-md)', overflow: 'hidden', height: '38px' }}>
              {unitBtn('s', 's')}{unitBtn('m', 'm')}{unitBtn('h', 'h')}{unitBtn('d', 'd')}
            </div>
          </div>
          <span class="mono" style={{ 'font-size': '10px', color: 'var(--text-faint)' }}>expected ping interval</span>
        </div>

        <div style={{ display: 'flex', 'flex-direction': 'column', gap: '6px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>grace</span>
          <div style={{ display: 'flex', gap: '6px', 'align-items': 'center', height: '38px' }}>
            <For each={GRACE_PRESETS}>
              {(g) => (
                <button
                  class="chip mono"
                  classList={{ on: !graceCustom() && graceSec() === g.v }}
                  onClick={() => { setGraceCustom(false); setGraceSec(g.v) }}
                >
                  {g.label}
                </button>
              )}
            </For>
            <Show
              when={graceCustom()}
              fallback={<button class="chip mono" onClick={() => setGraceCustom(true)}>custom</button>}
            >
              <span class="field cb-focusring" style={{ width: '96px' }}>
                <input
                  class="input mono tnum"
                  type="number"
                  min="0"
                  placeholder="sec"
                  value={String(graceSec())}
                  onInput={(e) => setGraceSec(Math.max(0, Math.round(Number(e.currentTarget.value))))}
                  onKeyDown={(e) => e.key === 'Enter' && submit()}
                />
              </span>
            </Show>
          </div>
          <span class="mono" style={{ 'font-size': '10px', color: 'var(--text-faint)' }}>slack before alerting</span>
        </div>

        <div style={{ display: 'flex', gap: '8px', 'margin-top': '23px' }}>
          <button class={`btn primary ${busy() ? 'loading' : ''}`} disabled={!valid()} onClick={submit} style={{ position: 'relative' }}>
            {props.initial ? 'save' : 'create'}
          </button>
          <button class="btn ghost" onClick={props.onCancel}>cancel</button>
        </div>
      </div>

      <div class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)', 'margin-top': '14px', 'border-top': '1px solid var(--line-soft)', 'padding-top': '12px' }}>
        // pings expected every {fmtPeriod(periodSeconds())}, alert {fmtPeriod(graceSec())} late
      </div>
      <Show when={err()}>{(msg) => <div class="mono" style={{ 'font-size': '12px', color: 'var(--sev-error)', 'margin-top': '10px' }}>// {msg()}</div>}</Show>
    </div>
  )
}
