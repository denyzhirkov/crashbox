import { useNavigate } from '@solidjs/router'
import { createSignal, Show } from 'solid-js'
import { api, ApiError } from '../api/client'
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
      if (err instanceof ApiError) setError(err.message)
      else setError('unexpected error')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class="min-h-full flex items-center justify-center px-6">
      <form
        onSubmit={onSubmit}
        class="w-[360px] flex flex-col gap-4 border border-ink-600 bg-ink-700/40 p-8 rounded-sm"
      >
        <div class="mb-2">
          <h1 class="font-serif text-[36px] text-ink-50 tracking-tight leading-none">
            crashbox
          </h1>
          <p class="text-[12px] text-ink-300 mt-1">// sign in to continue</p>
        </div>

        <label class="flex flex-col gap-1">
          <span class="text-[11px] text-ink-300">email</span>
          <input
            type="email"
            value={email()}
            onInput={(e) => setEmail(e.currentTarget.value)}
            autocomplete="username"
            required
            class="bg-ink-800 border border-ink-600 px-3 py-2 text-ink-100 focus:border-crash focus:outline-none"
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="text-[11px] text-ink-300">password</span>
          <input
            type="password"
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
            autocomplete="current-password"
            required
            class="bg-ink-800 border border-ink-600 px-3 py-2 text-ink-100 focus:border-crash focus:outline-none"
          />
        </label>

        <Show when={error()}>
          {(msg) => (
            <p class="text-[12px] text-crash">// {msg()}</p>
          )}
        </Show>

        <button
          type="submit"
          disabled={busy()}
          class="mt-2 bg-crash text-ink-50 px-3 py-2 hover:bg-crash-dim disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {busy() ? 'unlocking…' : 'unlock'}
        </button>
      </form>
    </div>
  )
}
