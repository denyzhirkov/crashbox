import { A, useNavigate } from '@solidjs/router'
import { type JSX, Show } from 'solid-js'
import { useAuth } from '../lib/auth-context'

export function AppShell(props: { children: JSX.Element }) {
  const { user, logout } = useAuth()
  const nav = useNavigate()

  const onLogout = async () => {
    await logout()
    nav('/login', { replace: true })
  }

  return (
    <div class="min-h-full flex flex-col">
      <header class="h-12 border-b border-ink-600 flex items-center px-6 gap-6 text-[13px]">
        <A
          href="/"
          class="font-serif text-[18px] tracking-tight text-ink-50 hover:text-crash transition-colors"
        >
          crashbox
        </A>
        <nav class="flex gap-4 text-ink-300">
          <A href="/projects" class="hover:text-ink-100" activeClass="text-ink-100">
            projects
          </A>
        </nav>
        <div class="ml-auto flex items-center gap-4 text-ink-300">
          <Show when={user()}>
            {(u) => (
              <>
                <span class="text-ink-400 truncate max-w-[200px]">{u().email}</span>
                <button class="hover:text-crash" onClick={onLogout}>
                  logout
                </button>
              </>
            )}
          </Show>
        </div>
      </header>
      <main class="flex-1 px-6 py-6 max-w-[1200px] w-full mx-auto">{props.children}</main>
      <footer class="border-t border-ink-600 px-6 py-2 text-[11px] text-ink-400 flex gap-4">
        <span><kbd class="text-ink-200">j</kbd>/<kbd class="text-ink-200">k</kbd> nav</span>
        <span><kbd class="text-ink-200">e</kbd> resolve</span>
        <span><kbd class="text-ink-200">/</kbd> search</span>
        <span class="ml-auto opacity-60">// crashbox</span>
      </footer>
    </div>
  )
}
