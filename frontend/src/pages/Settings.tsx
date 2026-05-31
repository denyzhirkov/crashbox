import { useParams } from '@solidjs/router'
import { createResource, createSignal, Show } from 'solid-js'
import { api } from '../api/client'
import { Breadcrumb, Page } from '../components/layout'
import { CopyBlock, PlatformTag } from '../components/primitives'
import { useAuth } from '../lib/auth-context'

export default function SettingsPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = () => Number(params.projectId)
  const [project] = createResource(projectId, (id) => api.projects.get(id))
  const [dsn, { mutate }] = createResource(projectId, (id) => api.projects.dsn(id))
  const [confirming, setConfirming] = createSignal(false)
  const [rotating, setRotating] = createSignal(false)
  const { user } = useAuth()

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

  return (
    <Page>
      <Breadcrumb
        items={[
          { label: 'projects', href: '/projects' },
          { label: project()?.name ?? '…', href: `/projects/${projectId()}/issues` },
          { label: 'settings' },
        ]}
      />

      <h1 class="mono" style={{ 'font-size': '22px', 'font-weight': 600, 'margin-bottom': '4px' }}>{project()?.name ?? '…'}</h1>
      <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'margin-bottom': '28px' }}>
        <span class="mono" style={{ 'font-size': '12.5px', color: 'var(--text-faint)' }}>{project()?.slug}</span>
        <Show when={project()?.platform}>
          <span style={{ color: 'var(--text-faint)', opacity: 0.4 }}>·</span>
          <PlatformTag platform={project()?.platform} />
        </Show>
      </div>

      <div class="card" style={{ padding: '24px', 'margin-bottom': '16px' }}>
        <Field label="DSN" hint="point your SDK here">
          <Show when={dsn()} fallback={<div class="skel" style={{ height: '48px' }} />}>
            {(d) => <CopyBlock value={d().dsn} big />}
          </Show>
        </Field>
        <Field label="public key">
          <div class="codeblk" style={{ padding: '10px 12px', 'font-size': '12.5px', color: 'var(--text-mid)' }}>{dsn()?.public_key ?? '…'}</div>
        </Field>
        <Field label="project id">
          <div class="codeblk" style={{ padding: '10px 12px', 'font-size': '12.5px', color: 'var(--text-mid)' }}>{project()?.id ?? '…'}</div>
        </Field>
      </div>

      <Show when={user()?.is_admin}>
        <div class="card" style={{ padding: '24px', 'border-color': confirming() ? 'oklch(0.690 0.150 45 / 0.3)' : 'var(--line)' }}>
          <div style={{ display: 'flex', 'align-items': 'flex-start', 'justify-content': 'space-between' }}>
            <div style={{ display: 'flex', 'flex-direction': 'column', gap: '4px' }}>
              <span class="mono" style={{ 'font-size': '13.5px', 'font-weight': 600, color: 'var(--text-hi)' }}>rotate key</span>
              <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-lo)', 'max-width': '460px' }}>
                generate a new public key. the current DSN stops working immediately.
              </span>
            </div>
            <Show when={!confirming()}>
              <button class="btn danger sm" onClick={() => setConfirming(true)}>rotate</button>
            </Show>
          </div>

          <Show when={confirming()}>
            <div style={{ 'margin-top': '18px', 'padding-top': '18px', 'border-top': '1px solid var(--line-soft)' }}>
              <div class="mono" style={{ 'font-size': '12.5px', color: 'var(--sev-error)', 'margin-bottom': '14px', 'line-height': 1.5 }}>
                // this invalidates the current DSN. SDKs using the old key will get 401.
              </div>
              <div style={{ display: 'flex', gap: '12px' }}>
                <button class={`btn danger solid sm ${rotating() ? 'loading' : ''}`} onClick={rotate} style={{ position: 'relative' }}>confirm rotate</button>
                <button class="btn ghost sm" onClick={() => setConfirming(false)} disabled={rotating()}>cancel</button>
              </div>
            </div>
          </Show>
        </div>
      </Show>
    </Page>
  )
}

function Field(props: { label: string; hint?: string; children: any }) {
  return (
    <div style={{ display: 'flex', 'flex-direction': 'column', gap: '8px', 'margin-bottom': '22px' }}>
      <div style={{ display: 'flex', 'align-items': 'center', gap: '8px' }}>
        <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-lo)', 'letter-spacing': '0.04em', 'text-transform': 'uppercase' }}>{props.label}</span>
        <Show when={props.hint}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-faint)' }}>{props.hint}</span>
        </Show>
      </div>
      {props.children}
    </div>
  )
}
