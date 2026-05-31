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
        <div class="voice" style={{ padding: '32px' }}><span class="pfx">// </span>loading…</div>
      </Match>
      <Match when={user() === null}>
        <div />
      </Match>
      <Match when={user()}>{props.children}</Match>
    </Switch>
  )
}
