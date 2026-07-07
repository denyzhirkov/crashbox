// Shared visual primitives for the Signal/Aurora UI.
// Framework-agnostic look lives in index.css; these wire it to data.

import { type JSX, createSignal, For, Show } from 'solid-js'

/* ---- minimal stroke icons (UI affordances only) --------------------------
   Factories, not JSX values: a plain `name: <path/>` entry compiles to ONE
   real DOM node created at module load, and mounting the same icon twice
   makes the later <Icon> steal the node from the earlier one (bit us on the
   heartbeats tape — icons appeared only on the last row). `() => JSX` clones
   fresh nodes per instance. */
const ICONS: Record<string, () => JSX.Element> = {
  copy: () => (
    <>
      <rect x="4" y="4" width="9" height="9" rx="2" />
      <path d="M7 4V3a2 2 0 0 1 2-2h5a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-1" transform="translate(-1 1)" />
    </>
  ),
  check: () => <path d="M3 8.5 6.5 12 13 4" />,
  search: () => (
    <>
      <circle cx="7.5" cy="7.5" r="4.5" />
      <path d="m14 14-3-3" />
    </>
  ),
  star: () => <path d="M8 1.6l1.9 3.9 4.3.6-3.1 3 .8 4.3L8 11.3 4.2 13.4l.8-4.3-3.1-3 4.3-.6z" />,
  x: () => <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />,
  chevron: () => <path d="M5 6l3 3 3-3" />,
  chevronr: () => <path d="M6 4l4 4-4 4" />,
  arrow: () => <path d="M9 3 4 8l5 5M4 8h10" />,
  refresh: () => <path d="M13 7a5 5 0 1 0-1.2 4.2M13 3v3.5H9.5" />,
  plus: () => <path d="M8 3v10M3 8h10" />,
  enter: () => <path d="M13 4v3a2 2 0 0 1-2 2H3m0 0 3-3M3 9l3 3" />,
  bolt: () => <path d="M9 1 3 9h4l-1 6 6-8H8z" />,
  clock: () => (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 5v3.2l2 1.3" />
    </>
  ),
  snooze: () => (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M6 6h4l-4 4h4" />
    </>
  ),
  pulse: () => <path d="M1 8h3l2-5 3 10 2-5h4" />,
  dot: () => <circle cx="8" cy="8" r="2.4" />,
  dots: () => (
    <>
      <circle cx="3.2" cy="8" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="8" cy="8" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="12.8" cy="8" r="1.2" fill="currentColor" stroke="none" />
    </>
  ),
  ext: () => <path d="M6 4H4a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1v-2M9 3h4v4M13 3 7 9" />,
  cmd: () => <path d="M5.5 4.5A1.5 1.5 0 1 1 7 6v4a1.5 1.5 0 1 1-1.5-1.5h5A1.5 1.5 0 1 1 9 10V6a1.5 1.5 0 1 1 1.5 1.5h-5z" />,
}

export function Icon(props: { name: keyof typeof ICONS | string; size?: number; style?: JSX.CSSProperties }) {
  const size = () => props.size ?? 14
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      style={{ flex: 'none', ...props.style }}
    >
      {ICONS[props.name]?.()}
    </svg>
  )
}

const PLATFORM: Record<string, string> = {
  javascript: 'js', node: 'node', rust: 'rs', python: 'py', go: 'go', ruby: 'rb',
}
export function PlatformTag(props: { platform?: string | null }) {
  const label = () => (props.platform ? PLATFORM[props.platform] ?? props.platform : '')
  return (
    <Show when={props.platform}>
      <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)', 'letter-spacing': '0.02em' }}>
        {label()}
      </span>
    </Show>
  )
}

/* ---- Sparkline · 24h micro-histogram (bars). bucket 0 = oldest, last = now */
export function Sparkline(props: {
  buckets?: number[]
  w?: number
  h?: number
  dim?: boolean
  accent?: boolean
}) {
  const w = () => props.w ?? 72
  const h = () => props.h ?? 22
  const hours = () => props.buckets ?? []
  const max = () => Math.max(1, ...hours())
  const bw = () => w() / Math.max(1, hours().length)
  const col = () => (props.accent ? 'var(--accent)' : props.dim ? 'var(--text-faint)' : 'var(--text-lo)')
  const peakCol = () => (props.accent ? 'var(--accent)' : props.dim ? 'var(--text-lo)' : 'var(--text-mid)')
  return (
    <Show
      when={hours().length > 0}
      fallback={<span class="mono" style={{ color: 'var(--text-faint)', 'font-size': '10px' }}>—</span>}
    >
      <svg
        width={w()}
        height={h()}
        viewBox={`0 0 ${w()} ${h()}`}
        style={{ display: 'block', opacity: props.dim ? 0.6 : 1, flex: 'none' }}
        role="img"
        aria-label={`24h activity: ${hours().reduce((a, b) => a + b, 0)} events`}
      >
        <For each={hours()}>
          {(v, i) => {
            const bh = Math.max(1, (v / max()) * (h() - 2))
            const last = i() >= hours().length - 3
            return (
              <rect
                x={i() * bw() + 0.5}
                y={h() - bh}
                width={Math.max(0.8, bw() - 1)}
                height={bh}
                rx="0.6"
                fill={last ? peakCol() : col()}
                opacity={0.45 + (v / max()) * 0.55}
              />
            )
          }}
        </For>
      </svg>
    </Show>
  )
}

/* ---- Severity edge cue (load-bearing level signal) ---------------------- */
export function SevCue(props: { level?: string | null; variant?: 'bar' | 'dot' | 'glow'; style?: JSX.CSSProperties }) {
  const level = () => props.level ?? 'error'
  const variant = () => props.variant ?? 'bar'
  return <span class={`sevcue ${variant()} sev-${level()}`} aria-label={level()} style={props.style} />
}

/* ---- Cadence lane · signature element for heartbeats --------------------
 * Where "now" sits inside the current heartbeat cycle, derived purely from
 * status / last_ping / period / grace (+ a parent-supplied `now` in ms). We
 * store NO ping history, so this is not an uptime chart — it's the *approach*
 * to the next deadline: anchor (last ping, left) → due notch (72%) → grace
 * band → overdue tail. Reuses the event-scrubber visual language (tick + head).
 *
 * Down loudness stays "whisper": the fill/head simply turn severity-red past
 * the deadline. No pulse, no glow — urgency comes from ordering + color, so
 * the one-accent rule is untouched. */
export function CadenceLane(props: {
  status: string
  lastPingAt: string | null
  periodSeconds: number
  graceSeconds: number
  now: number
  h?: number
}) {
  const DUE = 72 // % position of the "due" notch — fixed so a wall of lanes aligns
  const DL = 92 // % position where grace runs out (deadline)
  const model = () => {
    const h = props.h ?? 8
    if (props.status === 'pending' || props.status === 'paused' || !props.lastPingAt) {
      return { h, pct: null as number | null, color: 'var(--text-mid)' }
    }
    const last = Date.parse(props.lastPingAt)
    if (Number.isNaN(last)) return { h, pct: null as number | null, color: 'var(--text-mid)' }
    const dueAt = last + props.periodSeconds * 1000
    const deadline = dueAt + props.graceSeconds * 1000
    const now = props.now
    let pct: number
    let color: string
    if (now <= dueAt) {
      pct = ((now - last) / Math.max(1, dueAt - last)) * DUE
      color = 'var(--text-mid)'
    } else if (now <= deadline) {
      pct = DUE + ((now - dueAt) / Math.max(1, deadline - dueAt)) * (DL - DUE)
      color = 'var(--sev-warning)'
    } else {
      const over = (now - deadline) / (props.periodSeconds * 1000)
      pct = Math.min(100, DL + over * 8)
      color = 'var(--sev-error)'
    }
    return { h, pct: Math.max(0, Math.min(100, pct)), color }
  }
  return (
    <div style={{ position: 'relative', flex: '1', 'min-width': '110px', height: `${model().h}px` }} aria-hidden="true">
      <div style={{ position: 'absolute', inset: 0, 'border-radius': '999px', background: 'var(--bg-sunken)', border: '1px solid var(--line)', overflow: 'hidden' }}>
        {/* grace band (due → deadline) */}
        <div style={{ position: 'absolute', top: 0, bottom: 0, left: '72%', width: '20%', background: 'oklch(0.792 0.108 82 / 0.09)' }} />
        {/* fill (last ping → now) */}
        <Show when={model().pct != null}>
          <div style={{ position: 'absolute', top: 0, bottom: 0, left: 0, width: `${model().pct}%`, background: model().color, opacity: 0.5 }} />
        </Show>
      </div>
      {/* due notch */}
      <div style={{ position: 'absolute', top: '-3px', bottom: '-3px', left: '72%', width: '1px', background: 'var(--line-strong)' }} />
      {/* anchor: last ping */}
      <div style={{ position: 'absolute', top: '50%', left: 0, width: '6px', height: '6px', 'border-radius': '50%', background: 'var(--text-mid)', transform: 'translate(-1px,-50%)' }} />
      {/* now head */}
      <Show when={model().pct != null}>
        <div style={{ position: 'absolute', top: '-3px', bottom: '-3px', left: `${model().pct}%`, width: '2px', background: 'var(--text-hi)', 'border-radius': '2px', transform: 'translateX(-1px)' }} />
      </Show>
    </div>
  )
}

/* ---- DSN click-to-copy (underline affordance, no toast) ----------------- */
export function CopyBlock(props: { value: string; big?: boolean }) {
  const [copied, setCopied] = createSignal(false)
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(props.value)
    } catch {
      /* clipboard blocked */
    }
    setCopied(true)
    setTimeout(() => setCopied(false), 1400)
  }
  return (
    <button
      onClick={onCopy}
      title="click to copy"
      class="mono cb-copy"
      style={{
        display: 'flex', 'align-items': 'center', gap: '10px', width: '100%', 'text-align': 'left',
        background: 'var(--bg-sunken)', border: '1px solid var(--line)', 'border-radius': 'var(--r-md)',
        color: 'var(--text)', cursor: 'pointer', padding: props.big ? '14px 16px' : '10px 12px',
        'font-size': props.big ? '13px' : '12px', 'line-height': 1.5,
      }}
    >
      <span
        style={{
          flex: '1', 'min-width': 0, 'word-break': 'break-all', 'white-space': 'normal',
          'border-bottom': `1px dashed ${copied() ? 'transparent' : 'var(--line-strong)'}`, 'padding-bottom': '1px',
        }}
      >
        {props.value}
      </span>
      <span
        style={{
          display: 'inline-flex', 'align-items': 'center', gap: '4px', flex: 'none',
          'font-size': '11px', color: copied() ? 'var(--accent-ink)' : 'var(--text-faint)',
        }}
      >
        <Icon name={copied() ? 'check' : 'copy'} size={13} />
        {copied() ? 'copied' : 'copy'}
      </span>
    </button>
  )
}

/* ---- Stat (tabular numeral + caption) ----------------------------------- */
export function Stat(props: { label: string; value: string | number; accent?: boolean }) {
  return (
    <div style={{ display: 'flex', 'flex-direction': 'column', gap: '3px' }}>
      <span
        class="mono tnum"
        style={{ 'font-size': '20px', 'font-weight': 600, 'line-height': 1, color: props.accent ? 'var(--accent-ink)' : 'var(--text-hi)' }}
      >
        {typeof props.value === 'number' ? props.value.toLocaleString('en-US') : props.value}
      </span>
      <span class="mono" style={{ 'font-size': '10.5px', 'letter-spacing': '0.06em', 'text-transform': 'uppercase', color: 'var(--text-faint)' }}>
        {props.label}
      </span>
    </div>
  )
}

/* ---- comment-line voice (empty / loading states) ------------------------ */
export function Voice(props: { children: JSX.Element; style?: JSX.CSSProperties }) {
  return (
    <div class="voice" style={props.style}>
      <span class="pfx">// </span>
      {props.children}
    </div>
  )
}

export function fmt(n: number): string {
  return n.toLocaleString('en-US')
}
