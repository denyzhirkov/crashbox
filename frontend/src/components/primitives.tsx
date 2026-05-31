// Shared visual primitives for the Signal/Aurora UI.
// Framework-agnostic look lives in index.css; these wire it to data.

import { type JSX, createSignal, For, Show } from 'solid-js'

/* ---- minimal stroke icons (UI affordances only) ------------------------- */
const ICONS: Record<string, JSX.Element> = {
  copy: (
    <>
      <rect x="4" y="4" width="9" height="9" rx="2" />
      <path d="M7 4V3a2 2 0 0 1 2-2h5a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-1" transform="translate(-1 1)" />
    </>
  ),
  check: <path d="M3 8.5 6.5 12 13 4" />,
  search: (
    <>
      <circle cx="7.5" cy="7.5" r="4.5" />
      <path d="m14 14-3-3" />
    </>
  ),
  star: <path d="M8 1.6l1.9 3.9 4.3.6-3.1 3 .8 4.3L8 11.3 4.2 13.4l.8-4.3-3.1-3 4.3-.6z" />,
  x: <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />,
  chevron: <path d="M5 6l3 3 3-3" />,
  chevronr: <path d="M6 4l4 4-4 4" />,
  arrow: <path d="M9 3 4 8l5 5M4 8h10" />,
  refresh: <path d="M13 7a5 5 0 1 0-1.2 4.2M13 3v3.5H9.5" />,
  plus: <path d="M8 3v10M3 8h10" />,
  enter: <path d="M13 4v3a2 2 0 0 1-2 2H3m0 0 3-3M3 9l3 3" />,
  bolt: <path d="M9 1 3 9h4l-1 6 6-8H8z" />,
  clock: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 5v3.2l2 1.3" />
    </>
  ),
  snooze: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M6 6h4l-4 4h4" />
    </>
  ),
  dot: <circle cx="8" cy="8" r="2.4" />,
  ext: <path d="M6 4H4a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1v-2M9 3h4v4M13 3 7 9" />,
  cmd: <path d="M5.5 4.5A1.5 1.5 0 1 1 7 6v4a1.5 1.5 0 1 1-1.5-1.5h5A1.5 1.5 0 1 1 9 10V6a1.5 1.5 0 1 1 1.5 1.5h-5z" />,
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
      {ICONS[props.name]}
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
  const col = () => (props.accent ? 'url(#cbspark)' : props.dim ? 'var(--text-faint)' : 'var(--text-lo)')
  const peakCol = () => (props.accent ? 'url(#cbspark)' : props.dim ? 'var(--text-lo)' : 'var(--text-mid)')
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
        <defs>
          <linearGradient id="cbspark" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="var(--accent-violet)" />
            <stop offset="1" stop-color="var(--accent-cyan)" />
          </linearGradient>
        </defs>
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
          'font-size': '11px', color: copied() ? 'var(--accent-cyan)' : 'var(--text-faint)',
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
