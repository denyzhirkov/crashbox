import { createResource, createSignal, For, Show } from 'solid-js'
import { api } from '../api/client'
import type { ApiToken, CreatedToken } from '../api/types'
import { Breadcrumb, Page } from '../components/layout'
import { CopyBlock, Icon, Voice } from '../components/primitives'
import { useAuth } from '../lib/auth-context'
import { relTime } from '../lib/time'

// Personal API tokens — user-level, not project-level. The secret is displayed exactly once
// (the create response); afterwards only name + prefix + usage dates exist anywhere.

const EXPIRY_CHOICES: Array<{ label: string; days?: number }> = [
  { label: 'never' },
  { label: '30 days', days: 30 },
  { label: '90 days', days: 90 },
  { label: '1 year', days: 365 },
]

/** Compact future-time: "42m", "12h", "89d" — `relTime` only formats the past. */
function inTime(iso: string): string {
  const diff = (Date.parse(iso) - Date.now()) / 1000
  if (diff < 3600) return `${Math.max(1, Math.floor(diff / 60))}m`
  if (diff < 86_400) return `${Math.floor(diff / 3600)}h`
  return `${Math.floor(diff / 86_400)}d`
}

function expiryLabel(t: ApiToken): string {
  if (!t.expires_at) return 'never expires'
  if (Date.parse(t.expires_at) < Date.now()) return 'expired'
  return `expires in ${inTime(t.expires_at)}`
}

export default function TokensPage() {
  const [tokens, { refetch }] = createResource(() => api.tokens.list())
  const { user } = useAuth()
  const isAdmin = () => user()?.is_admin === true

  const [creating, setCreating] = createSignal(false)
  const [minted, setMinted] = createSignal<CreatedToken | null>(null)

  const remove = async (t: ApiToken) => {
    if (!window.confirm(`Revoke token "${t.name}" (${t.token_prefix}…)? Anything using it gets 401 immediately.`)) return
    await api.tokens.remove(t.id)
    if (minted()?.id === t.id) setMinted(null)
    void refetch()
  }

  return (
    <Page>
      <Breadcrumb items={[{ label: 'projects', href: '/projects' }, { label: 'api tokens' }]} />

      <div style={{ display: 'flex', 'align-items': 'center', gap: '16px', 'margin-bottom': '10px' }}>
        <h1 class="mono" style={{ 'font-size': '22px', 'font-weight': 600 }}>api tokens</h1>
        <span class="mono tnum" style={{ 'font-size': '13px', color: 'var(--text-faint)' }}>
          {(tokens() ?? []).length}
        </span>
        <div style={{ flex: '1' }} />
        <Show when={isAdmin() && !creating()}>
          <button class="btn sm primary" onClick={() => { setMinted(null); setCreating(true) }}>
            <Icon name="plus" size={13} /> new token
          </button>
        </Show>
      </div>

      <div class="mono" style={{ 'font-size': '12px', color: 'var(--text-faint)', 'margin-bottom': '18px' }}>
        bearer credentials for automation — scripts, CI, Claude Code. full admin access; tokens
        cannot manage tokens. use as{' '}
        <span style={{ color: 'var(--text-lo)' }}>Authorization: Bearer cbx_…</span>
      </div>

      <Show when={creating()}>
        <div style={{ 'margin-bottom': '16px' }}>
          <MintForm
            onCancel={() => setCreating(false)}
            onMinted={(t) => { setMinted(t); setCreating(false); void refetch() }}
          />
        </div>
      </Show>

      <Show when={minted()}>
        {(t) => (
          <div class="card" style={{ padding: '16px 18px', 'margin-bottom': '16px', 'border-color': 'var(--accent-ink)' }}>
            <div class="mono" style={{ 'font-size': '12.5px', color: 'var(--text-hi)', 'margin-bottom': '10px' }}>
              // token "{t().name}" — shown once, copy it now. it cannot be retrieved again.
            </div>
            <CopyBlock value={t().token} big />
          </div>
        )}
      </Show>

      <Show when={!tokens.loading || tokens()} fallback={<div class="skel" style={{ height: '90px' }} />}>
        <Show
          when={(tokens() ?? []).length > 0}
          fallback={
            <div class="card" style={{ padding: '40px', 'text-align': 'center' }}>
              <Voice>no tokens. mint one to automate crashbox from scripts or claude code.</Voice>
            </div>
          }
        >
          <div class="card" style={{ overflow: 'hidden' }}>
            <For each={tokens()}>
              {(t, i) => (
                <div
                  style={{
                    display: 'flex', 'align-items': 'center', gap: '14px', padding: '12px 16px',
                    'border-top': i() > 0 ? '1px solid var(--line-soft)' : 'none',
                  }}
                >
                  <span class="mono" style={{ 'font-size': '13px', 'font-weight': 500, color: 'var(--text-hi)', width: '200px', flex: 'none', overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }} title={t.name}>
                    {t.name}
                  </span>
                  <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)' }}>{t.token_prefix}…</span>
                  <div style={{ flex: '1' }} />
                  <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)' }} title={t.created_at}>
                    created {relTime(t.created_at)}
                  </span>
                  <span class="mono" style={{ 'font-size': '11.5px', color: t.expires_at && Date.parse(t.expires_at) < Date.now() ? 'var(--sev-error)' : 'var(--text-faint)', width: '130px', flex: 'none', 'text-align': 'right' }} title={t.expires_at ?? undefined}>
                    {expiryLabel(t)}
                  </span>
                  <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)', width: '110px', flex: 'none', 'text-align': 'right' }} title={t.last_used_at ?? undefined}>
                    {t.last_used_at ? `used ${relTime(t.last_used_at)}` : 'never used'}
                  </span>
                  <Show when={isAdmin()}>
                    <button class="btn ghost sm" onClick={() => void remove(t)} title="revoke — instant 401 for anything using it">
                      <Icon name="x" size={12} />
                    </button>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </Page>
  )
}

function MintForm(props: { onCancel: () => void; onMinted: (t: CreatedToken) => void }) {
  const [name, setName] = createSignal('')
  const [expiry, setExpiry] = createSignal(0) // index into EXPIRY_CHOICES
  const [busy, setBusy] = createSignal(false)
  const [err, setErr] = createSignal<string | null>(null)

  const submit = async () => {
    if (!name().trim()) return
    setBusy(true)
    setErr(null)
    try {
      const days = EXPIRY_CHOICES[expiry()]?.days
      const t = await api.tokens.create({
        name: name().trim(),
        ...(days ? { expires_in_days: days } : {}),
      })
      props.onMinted(t)
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class="card" style={{ padding: '20px' }}>
      <div class="mono" style={{ 'font-size': '13px', color: 'var(--text-hi)', 'margin-bottom': '14px' }}>// new api token</div>
      <div style={{ display: 'flex', 'align-items': 'flex-end', gap: '12px', 'flex-wrap': 'wrap' }}>
        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', flex: '1', 'min-width': '200px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>name</span>
          <span class="field cb-focusring">
            <input
              class="input mono"
              autofocus
              placeholder="claude-code"
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => e.key === 'Enter' && submit()}
            />
          </span>
        </label>
        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', width: '140px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>expires</span>
          <span class="field cb-focusring" style={{ display: 'block' }}>
            <select class="input mono" value={expiry()} onChange={(e) => setExpiry(Number(e.currentTarget.value))} style={{ cursor: 'pointer' }}>
              <For each={EXPIRY_CHOICES}>{(c, i) => <option value={i()}>{c.label}</option>}</For>
            </select>
          </span>
        </label>
        <button class={`btn primary ${busy() ? 'loading' : ''}`} disabled={!name().trim()} onClick={submit} style={{ position: 'relative' }}>mint</button>
        <button class="btn ghost" onClick={props.onCancel}>cancel</button>
      </div>
      <Show when={err()}>{(m) => <div class="mono" style={{ 'font-size': '12px', color: 'var(--sev-error)', 'margin-top': '12px' }}>// {m()}</div>}</Show>
    </div>
  )
}
