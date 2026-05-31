import { useNavigate } from '@solidjs/router'
import { createSignal, Show } from 'solid-js'
import { api, ApiError } from '../api/client'
import { Wordmark } from '../components/layout'
import { Voice } from '../components/primitives'
import { useAuth } from '../lib/auth-context'

export default function LoginPage() {
  const [email, setEmail] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [error, setError] = createSignal<string | null>(null)
  const [busy, setBusy] = createSignal(false)
  const nav = useNavigate()
  const { refresh } = useAuth()

  const onSubmit = async (e: SubmitEvent) => {
    e.preventDefault()
    setError(null)
    setBusy(true)
    try {
      await api.auth.login(email(), password())
      refresh()
      nav('/projects', { replace: true })
    } catch (err) {
      if (err instanceof ApiError) setError(`// ${err.message}`)
      else setError('// auth failed. check email + password.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div style={{ display: 'flex', 'min-height': '100vh', 'justify-content': 'center', 'align-items': 'center', padding: '24px' }}>
      <form onSubmit={onSubmit} class="card" style={{ width: '372px', padding: '28px', position: 'relative' }}>
        <div style={{ 'margin-bottom': '22px' }}>
          <Wordmark href="/projects" />
        </div>
        <Voice style={{ 'margin-bottom': '20px' }}>sign in to continue</Voice>

        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', 'margin-bottom': '14px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)', 'letter-spacing': '0.04em' }}>email</span>
          <span class="field cb-focusring" style={{ 'border-radius': 'var(--r-md)' }}>
            <input
              class="input mono"
              type="email"
              value={email()}
              onInput={(e) => setEmail(e.currentTarget.value)}
              autocomplete="username"
              spellcheck={false}
              required
            />
          </span>
        </label>

        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', 'margin-bottom': error() ? '10px' : '22px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)', 'letter-spacing': '0.04em' }}>password</span>
          <span class="field cb-focusring" style={{ 'border-radius': 'var(--r-md)' }}>
            <input
              class="input mono"
              type="password"
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
              autocomplete="current-password"
              required
            />
          </span>
        </label>

        <Show when={error()}>
          {(msg) => <div class="mono" style={{ 'font-size': '12px', color: 'var(--sev-error)', 'margin-bottom': '16px' }}>{msg()}</div>}
        </Show>

        <button type="submit" class={`btn primary ${busy() ? 'loading' : ''}`} style={{ width: '100%', height: '40px', 'justify-content': 'center' }}>
          unlock
        </button>
        <div style={{ display: 'flex', 'justify-content': 'center', 'margin-top': '16px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)' }}>crashbox · self-hosted</span>
        </div>
      </form>
    </div>
  )
}
