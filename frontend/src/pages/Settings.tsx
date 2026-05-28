import { A, useParams } from '@solidjs/router'
import { createResource, createSignal, Show } from 'solid-js'
import { api } from '../api/client'
import { useAuth } from '../lib/auth-context'

export default function SettingsPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const [dsn, { mutate }] = createResource(projectId, (id) => api.projects.dsn(id))
  const [confirming, setConfirming] = createSignal(false)
  const [rotating, setRotating] = createSignal(false)
  const { user } = useAuth()
  const [copied, setCopied] = createSignal(false)

  const rotate = async () => {
    setRotating(true)
    try {
      const fresh = await api.projects.rotateKey(projectId())
      mutate(fresh)
      setConfirming(false)
    } finally {
      setRotating(false)
    }
  }

  const copyDsn = async () => {
    const d = dsn()
    if (!d) return
    try {
      await navigator.clipboard.writeText(d.dsn)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      /* clipboard blocked */
    }
  }

  return (
    <section class="flex flex-col gap-6 max-w-[720px]">
      <header class="flex items-baseline gap-3">
        <A href="/projects" class="text-ink-400 hover:text-ink-100 text-[12px]">
          projects
        </A>
        <span class="text-ink-500">/</span>
        <A
          href={`/projects/${projectId()}/issues`}
          class="text-ink-400 hover:text-ink-100 text-[12px]"
        >
          {project()?.name ?? '…'}
        </A>
        <span class="text-ink-500">/</span>
        <h1 class="font-serif text-[20px] text-ink-50 leading-none">settings</h1>
      </header>

      <div class="flex flex-col gap-2">
        <p class="text-[11px] text-ink-400">// dsn</p>
        <Show when={dsn()}>
          {(d) => (
            <button
              onClick={copyDsn}
              class={`text-left text-[13px] font-mono px-3 py-2 bg-ink-700/40 border border-ink-600 hover:border-ink-400 ${
                copied() ? 'underline decoration-crash decoration-2 underline-offset-4' : ''
              }`}
              title="click to copy"
            >
              {d().dsn}
            </button>
          )}
        </Show>
        <p class="text-[11px] text-ink-400">public_key: {dsn()?.public_key}</p>
      </div>

      <Show when={user()?.is_admin}>
        <div class="flex flex-col gap-2 border-t border-ink-600 pt-6">
          <p class="text-[11px] text-ink-400">// rotate key</p>
          <Show
            when={confirming()}
            fallback={
              <button
                onClick={() => setConfirming(true)}
                class="self-start text-[12px] border border-ink-600 px-3 py-1 hover:border-crash hover:text-crash"
              >
                rotate
              </button>
            }
          >
            <p class="text-[12px] text-ink-200">
              // this invalidates the current DSN. SDKs using the old key will get 401.
            </p>
            <div class="flex gap-2">
              <button
                onClick={rotate}
                disabled={rotating()}
                class="bg-crash text-ink-50 px-3 py-1 hover:bg-crash-dim disabled:opacity-50"
              >
                {rotating() ? 'rotating…' : 'confirm rotate'}
              </button>
              <button
                onClick={() => setConfirming(false)}
                class="border border-ink-600 px-3 py-1 hover:border-ink-400"
              >
                cancel
              </button>
            </div>
          </Show>
        </div>
      </Show>
    </section>
  )
}
