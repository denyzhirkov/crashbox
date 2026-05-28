import { useNavigate } from '@solidjs/router'
import { createEffect, type JSX, Match, Switch } from 'solid-js'
import { useAuth } from '../lib/auth-context'

export function RequireAuth(props: { children: JSX.Element }) {
  const { user } = useAuth()
  const nav = useNavigate()

  createEffect(() => {
    if (user.state === 'ready' && user() === null) {
      nav('/login', { replace: true })
    }
  })

  return (
    <Switch>
      <Match when={user.loading}>
        <div class="p-8 text-ink-400 text-[12px]">// loading…</div>
      </Match>
      <Match when={user() === null}>
        <div />
      </Match>
      <Match when={user()}>{props.children}</Match>
    </Switch>
  )
}
