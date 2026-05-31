// Custom monospaced JSON viewer — inherits Crashbox type, no 3rd-party widget.
// Collapsed at depth >= 2.

import { createSignal, For, type JSX, Show } from 'solid-js'
import { Icon } from './primitives'

const C = {
  key: 'oklch(0.72 0.030 250)',
  str: 'oklch(0.760 0.055 158)',
  num: 'oklch(0.800 0.065 64)',
  bool: 'oklch(0.700 0.110 300)',
  null: 'var(--text-faint)',
  punc: 'var(--text-faint)',
}

function renderPrim(v: unknown): JSX.Element {
  if (v === null) return <span style={{ color: C.null }}>null</span>
  if (typeof v === 'string') return <span style={{ color: C.str }}>"{v}"</span>
  if (typeof v === 'number') return <span class="tnum" style={{ color: C.num }}>{v}</span>
  if (typeof v === 'boolean') return <span style={{ color: C.bool }}>{String(v)}</span>
  return <span>{String(v)}</span>
}

function JsonNode(props: { k?: string | null; value: unknown; depth: number; last: boolean }) {
  const isObj = () => props.value !== null && typeof props.value === 'object'
  const isArr = () => Array.isArray(props.value)
  const [open, setOpen] = createSignal(props.depth < 2)
  const indent = (): JSX.CSSProperties => ({ 'padding-left': props.depth === 0 ? '0' : '16px' })
  const entries = (): Array<[string | number, unknown]> =>
    !isObj()
      ? []
      : isArr()
        ? (props.value as unknown[]).map((v, i) => [i, v])
        : Object.entries(props.value as Record<string, unknown>)
  const openB = () => (isArr() ? '[' : '{')
  const closeB = () => (isArr() ? ']' : '}')

  const keyEl = () => (
    <Show when={props.k != null}>
      <span style={{ color: C.key }}>"{props.k}"</span>
      <span style={{ color: C.punc }}>: </span>
    </Show>
  )

  return (
    <Show
      when={isObj()}
      fallback={
        <div style={indent()}>
          {keyEl()}
          {renderPrim(props.value)}
          <Show when={!props.last}><span style={{ color: C.punc }}>,</span></Show>
        </div>
      }
    >
      <div style={indent()}>
        <div class="cb-jsontoggle" onClick={() => setOpen((o) => !o)} style={{ cursor: 'pointer', display: 'inline-flex', 'align-items': 'center', gap: '4px' }}>
          <span style={{ color: 'var(--text-faint)', transform: open() ? 'none' : 'rotate(-90deg)', transition: 'transform 0.12s', display: 'inline-flex' }}>
            <Icon name="chevron" size={11} />
          </span>
          {keyEl()}
          <span style={{ color: C.punc }}>{openB()}</span>
          <Show when={!open()}>
            <span style={{ color: 'var(--text-faint)' }}>{isArr() ? `${entries().length} items` : `${entries().length} keys`}</span>
            <span style={{ color: C.punc }}>{closeB()}{!props.last ? ',' : ''}</span>
          </Show>
        </div>
        <Show when={open()}>
          <div style={{ 'border-left': '1px solid var(--line-soft)', 'margin-left': '5px' }}>
            <For each={entries()}>
              {([ck, cv], i) => (
                <JsonNode k={isArr() ? null : (ck as string)} value={cv} depth={props.depth + 1} last={i() === entries().length - 1} />
              )}
            </For>
          </div>
          <div style={{ color: C.punc }}>{closeB()}{!props.last ? ',' : ''}</div>
        </Show>
      </div>
    </Show>
  )
}

export function JsonViewer(props: { data: unknown }) {
  return (
    <div class="codeblk" style={{ padding: '12px 14px', 'font-size': '12.5px', 'line-height': 1.7, 'overflow-x': 'auto' }}>
      <JsonNode value={props.data} depth={0} last={true} />
    </div>
  )
}
