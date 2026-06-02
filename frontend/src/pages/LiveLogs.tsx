import { A, useParams } from '@solidjs/router'
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js'
import { createResource } from 'solid-js'
import { api } from '../api/client'
import type { LogLevel, LogRecord } from '../api/types'
import { Breadcrumb, Page } from '../components/layout'
import { Icon, Sparkline } from '../components/primitives'
import { useAuth } from '../lib/auth-context'

// Live tail of ephemeral logs for one project, streamed over SSE. Nothing is persisted server-side
// (a bounded RAM ring gives scrollback on (re)connect); the client keeps at most MAX_ROWS in memory.
const LEVELS: LogLevel[] = ['trace', 'debug', 'info', 'warn', 'error', 'fatal']
const RANK: Record<LogLevel, number> = { trace: 0, debug: 1, info: 2, warn: 3, error: 4, fatal: 5 }
// Map our six levels onto the design system's five severity cues.
const SEV: Record<LogLevel, string> = {
  trace: 'debug',
  debug: 'debug',
  info: 'info',
  warn: 'warning',
  error: 'error',
  fatal: 'fatal',
}
const MAX_ROWS = 1000
const RATE_WINDOW = 60 // seconds shown in the throughput sparkline

function ts(s: string): string {
  const d = new Date(s)
  return Number.isNaN(d.getTime()) ? s : d.toLocaleTimeString('en-GB', { hour12: false }) + '.' + String(d.getMilliseconds()).padStart(3, '0')
}

export default function LiveLogsPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const { user } = useAuth()
  const enabled = () => user()?.live_logs_enabled !== false

  const [records, setRecords] = createSignal<LogRecord[]>([])
  const [pending, setPending] = createSignal<LogRecord[]>([])
  const [paused, setPaused] = createSignal(false)
  const [connected, setConnected] = createSignal(false)
  const [minLevel, setMinLevel] = createSignal<LogLevel>('trace')
  const [query, setQuery] = createSignal('')
  const [groupByLogger, setGroupByLogger] = createSignal(false)
  // Throughput sparkline: one bucket per second, oldest→newest, advanced by a 1s timer.
  const [rate, setRate] = createSignal<number[]>(new Array(RATE_WINDOW).fill(0))

  let listRef: HTMLDivElement | undefined
  let atBottom = true

  const queueScroll = () => {
    if (!atBottom) return
    requestAnimationFrame(() => {
      if (listRef) listRef.scrollTop = listRef.scrollHeight
    })
  }
  const onScroll = () => {
    if (!listRef) return
    atBottom = listRef.scrollHeight - listRef.scrollTop - listRef.clientHeight < 40
  }

  // (Re)open the stream whenever the project changes. EventSource auto-reconnects on its own;
  // onerror just flips the indicator without tearing the connection down.
  createEffect(() => {
    if (!enabled()) return
    const id = projectId()
    setRecords([])
    setPending([])
    setRate(new Array(RATE_WINDOW).fill(0))
    setConnected(false)
    const es = new EventSource(api.livelog.streamUrl(id))
    es.onopen = () => setConnected(true)
    es.onerror = () => setConnected(false)
    es.onmessage = (e) => {
      let rec: LogRecord
      try {
        rec = JSON.parse(e.data) as LogRecord
      } catch {
        return
      }
      // Throughput counts every line received, regardless of pause/filter.
      setRate((r) => {
        const c = r.slice()
        c[c.length - 1] += 1
        return c
      })
      if (paused()) {
        setPending((p) => [...p, rec].slice(-MAX_ROWS))
      } else {
        setRecords((r) => [...r, rec].slice(-MAX_ROWS))
        queueScroll()
      }
    }
    const tick = setInterval(() => setRate((r) => [...r.slice(1), 0]), 1000)
    onCleanup(() => {
      es.close()
      clearInterval(tick)
    })
  })

  const resume = () => {
    setRecords((r) => [...r, ...pending()].slice(-MAX_ROWS))
    setPending([])
    setPaused(false)
    atBottom = true
    queueScroll()
  }
  const togglePause = () => (paused() ? resume() : setPaused(true))
  const clear = () => {
    setRecords([])
    setPending([])
  }

  const visible = createMemo(() => {
    const min = RANK[minLevel()]
    const q = query().trim().toLowerCase()
    return records().filter((r) => {
      if (RANK[r.level] < min) return false
      if (q) {
        const hay = `${r.message} ${r.logger ?? ''} ${JSON.stringify(r.attrs ?? {})}`.toLowerCase()
        if (!hay.includes(q)) return false
      }
      return true
    })
  })

  const groups = createMemo(() => {
    const map = new Map<string, { logger: string; count: number; last: LogRecord }>()
    for (const r of visible()) {
      const key = r.logger ?? '(none)'
      const g = map.get(key)
      if (g) {
        g.count += 1
        g.last = r
      } else {
        map.set(key, { logger: key, count: 1, last: r })
      }
    }
    return [...map.values()].sort((a, b) => b.count - a.count)
  })

  return (
    <Page>
      <div style={{ display: 'flex', 'align-items': 'flex-start', 'justify-content': 'space-between', 'margin-bottom': '16px' }}>
        <Breadcrumb
          items={[
            { label: 'projects', href: '/projects' },
            { label: project()?.name ?? '…', href: `/projects/${projectId()}/issues` },
            { label: 'live logs' },
          ]}
        />
        <div style={{ display: 'flex', gap: '8px' }}>
          <A href={`/projects/${projectId()}/issues`} class="btn ghost sm">issues</A>
          <A href={`/projects/${projectId()}/settings`} class="btn ghost sm">settings</A>
        </div>
      </div>

      <Show
        when={enabled()}
        fallback={
          <div class="voice" style={{ padding: '40px 20px', 'text-align': 'center' }}>
            <span class="pfx">// </span>live logs are disabled on this server (CRASHBOX_LIVE_LOGS_ENABLED=false)
          </div>
        }
      >
      {/* toolbar: connection · level floor · search · controls */}
      <div class="card" style={{ padding: '10px 12px', 'margin-bottom': '12px', display: 'flex', 'align-items': 'center', gap: '12px', 'flex-wrap': 'wrap' }}>
        <span style={{ display: 'flex', 'align-items': 'center', gap: '6px', 'font-size': '12px', color: 'var(--text-lo)' }}>
          <span class="livedot" style={{ opacity: connected() ? 1 : 0.25 }} />
          {connected() ? 'streaming' : 'connecting…'}
        </span>

        <div style={{ display: 'flex', gap: '4px' }}>
          <For each={LEVELS}>
            {(lv) => (
              <button
                class="chip"
                onClick={() => setMinLevel(lv)}
                title={`show ${lv} and above`}
                style={{
                  cursor: 'pointer',
                  opacity: RANK[lv] >= RANK[minLevel()] ? 1 : 0.4,
                  'border-color': lv === minLevel() ? 'var(--accent-cyan)' : undefined,
                }}
              >
                {lv}
              </button>
            )}
          </For>
        </div>

        <input
          class="cb-input mono"
          placeholder="filter message / logger / attrs…"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          style={{ flex: '1', 'min-width': '180px', background: 'var(--bg-sunken)', border: '1px solid var(--line)', 'border-radius': '7px', height: '30px', padding: '0 10px', color: 'var(--text-hi)', 'font-size': '12.5px' }}
        />

        <span title="logs/sec over the last minute" style={{ display: 'flex', 'align-items': 'center', gap: '6px' }}>
          <Sparkline buckets={rate()} w={90} h={22} accent />
          <span class="mono" style={{ 'font-size': '10.5px', color: 'var(--text-faint)' }}>/s · 60s</span>
        </span>

        <button class="btn ghost sm" onClick={() => setGroupByLogger((g) => !g)} title="group by logger">
          <Icon name="bolt" size={12} />
          {groupByLogger() ? 'ungroup' : 'group'}
        </button>
        <button class="btn ghost sm" onClick={togglePause}>
          <Icon name={paused() ? 'enter' : 'snooze'} size={12} />
          {paused() ? 'resume' : 'pause'}
        </button>
        <button class="btn ghost sm" onClick={clear} title="clear visible buffer">
          <Icon name="x" size={12} /> clear
        </button>
      </div>

      <Show when={paused() && pending().length > 0}>
        <button class="btn sm" onClick={resume} style={{ 'margin-bottom': '12px' }}>
          ↓ {pending().length} new {pending().length === 1 ? 'line' : 'lines'} — resume
        </button>
      </Show>

      <div
        ref={listRef}
        onScroll={onScroll}
        class="card"
        style={{ padding: '0', height: 'calc(100vh - 280px)', 'min-height': '320px', overflow: 'auto' }}
      >
        <Show
          when={visible().length > 0}
          fallback={
            <div class="voice" style={{ padding: '40px 20px', 'text-align': 'center' }}>
              <span class="pfx">// </span>
              {records().length === 0 ? 'waiting for logs…' : 'no lines match the current filter'}
            </div>
          }
        >
          <Show
            when={groupByLogger()}
            fallback={
              <For each={visible()}>
                {(r) => (
                  <div
                    style={{
                      display: 'grid',
                      'grid-template-columns': '92px 56px 1fr',
                      gap: '10px',
                      padding: '4px 12px',
                      'border-bottom': '1px solid var(--line)',
                      'align-items': 'baseline',
                    }}
                  >
                    <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)', 'white-space': 'nowrap' }}>{ts(r.ts)}</span>
                    <span class="mono" style={{ 'font-size': '10.5px', 'text-transform': 'uppercase', 'letter-spacing': '0.04em', display: 'flex', 'align-items': 'center', gap: '5px' }}>
                      <span class={`sevcue dot sev-${SEV[r.level]}`} />
                      <span style={{ color: 'var(--text-lo)' }}>{r.level}</span>
                    </span>
                    <span style={{ 'min-width': 0 }}>
                      <Show when={r.logger}>
                        <span class="mono" style={{ 'font-size': '11px', color: 'var(--accent-cyan)', 'margin-right': '8px' }}>{r.logger}</span>
                      </Show>
                      <span class="mono" style={{ 'font-size': '12.5px', color: 'var(--text-hi)', 'word-break': 'break-word' }}>{r.message}</span>
                      <Show when={r.attrs && Object.keys(r.attrs).length > 0}>
                        <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)', 'margin-left': '8px' }}>{JSON.stringify(r.attrs)}</span>
                      </Show>
                    </span>
                  </div>
                )}
              </For>
            }
          >
            <For each={groups()}>
              {(g) => (
                <div
                  style={{
                    display: 'grid',
                    'grid-template-columns': '160px 56px 1fr',
                    gap: '10px',
                    padding: '6px 12px',
                    'border-bottom': '1px solid var(--line)',
                    'align-items': 'baseline',
                  }}
                >
                  <span class="mono" style={{ 'font-size': '12px', color: 'var(--accent-cyan)', overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}>{g.logger}</span>
                  <span class="chip mono" style={{ 'font-size': '10.5px', 'justify-self': 'start' }}>{g.count}</span>
                  <span style={{ 'min-width': 0, display: 'flex', 'align-items': 'baseline', gap: '6px' }}>
                    <span class={`sevcue dot sev-${SEV[g.last.level]}`} />
                    <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)', overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}>{g.last.message}</span>
                  </span>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>

      <div class="mono" style={{ 'margin-top': '8px', 'font-size': '11px', color: 'var(--text-faint)' }}>
        {visible().length} shown · {records().length}/{MAX_ROWS} buffered{paused() ? ' · paused' : ''}
      </div>
      </Show>
    </Page>
  )
}
