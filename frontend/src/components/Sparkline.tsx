// 24h activity sparkline: one SVG, no chart library.
// Bucket index 0 = oldest (23-24h ago), 23 = current hour.

import { For } from 'solid-js'

const WIDTH = 80
const HEIGHT = 18
const GAP = 1

export function Sparkline(props: { buckets: number[] | undefined }) {
  if (!props.buckets || props.buckets.length === 0) {
    return <span class="text-ink-500 text-[10px]">—</span>
  }
  const buckets = props.buckets
  const max = Math.max(1, ...buckets)
  const barW = (WIDTH - GAP * (buckets.length - 1)) / buckets.length
  const total = buckets.reduce((a, b) => a + b, 0)

  return (
    <svg
      width={WIDTH}
      height={HEIGHT}
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      class="shrink-0"
      role="img"
      aria-label={`24h activity: ${total} events`}
    >
      <For each={buckets}>
        {(c, i) => {
          const empty = c === 0
          const h = empty ? 1 : Math.max(2, (c / max) * HEIGHT)
          const x = i() * (barW + GAP)
          const y = HEIGHT - h
          const hoursAgo = buckets.length - 1 - i()
          return (
            <rect
              x={x}
              y={y}
              width={barW}
              height={h}
              rx={0.5}
              fill={empty ? 'var(--color-ink-600)' : 'var(--color-crash)'}
              opacity={empty ? 0.4 : Math.max(0.45, c / max)}
            >
              <title>
                {c} event{c === 1 ? '' : 's'} · {hoursAgo === 0 ? 'this hour' : `${hoursAgo}h ago`}
              </title>
            </rect>
          )
        }}
      </For>
    </svg>
  )
}
