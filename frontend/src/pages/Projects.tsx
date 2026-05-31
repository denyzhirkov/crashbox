import { A } from '@solidjs/router'
import { createResource, createSignal, For, Show } from 'solid-js'
import { api } from '../api/client'
import type { Issue, ProjectOverview } from '../api/types'
import { Page } from '../components/layout'
import { CopyBlock, fmt, Icon, PlatformTag, SevCue, Sparkline, Stat, Voice } from '../components/primitives'
import { useAuth } from '../lib/auth-context'
import { relTime } from '../lib/time'

export default function ProjectsPage() {
  const [overview, { refetch }] = createResource(() => api.projects.overview())
  const { user } = useAuth()
  const [creating, setCreating] = createSignal(false)
  const [refreshing, setRefreshing] = createSignal(false)

  const doRefresh = async () => {
    setRefreshing(true)
    await refetch()
    setRefreshing(false)
  }

  return (
    <Page>
      <div style={{ display: 'flex', 'align-items': 'center', gap: '16px', 'margin-bottom': '22px' }}>
        <h1 class="mono" style={{ 'font-size': '26px', 'font-weight': 600 }}>projects</h1>
        <span class="mono tnum" style={{ 'font-size': '13px', color: 'var(--text-faint)', 'margin-top': '6px' }}>
          {(overview() ?? []).length}
        </span>
        <div style={{ flex: '1' }} />
        <button class={`btn sm ${refreshing() ? 'loading' : ''}`} onClick={doRefresh} style={{ position: 'relative' }}>
          <Icon name="refresh" size={13} /> refresh
        </button>
        <Show when={user()?.is_admin && !creating()}>
          <button class="btn sm primary" onClick={() => setCreating(true)}>
            <Icon name="plus" size={13} /> new
          </button>
        </Show>
      </div>

      <Show when={creating()}>
        <div style={{ 'margin-bottom': '18px' }}>
          <CreateProject onCancel={() => setCreating(false)} onCreated={() => { setCreating(false); void refetch() }} />
        </div>
      </Show>

      <Show
        when={!overview.loading}
        fallback={<div style={{ display: 'flex', 'flex-direction': 'column', gap: '16px' }}><For each={[0, 1, 2]}>{() => <ProjectSkeleton />}</For></div>}
      >
        <Show
          when={(overview() ?? []).length > 0}
          fallback={<div class="card" style={{ padding: '40px', 'text-align': 'center' }}><Voice>no projects yet. spin one up to get a DSN.</Voice></div>}
        >
          <div style={{ display: 'flex', 'flex-direction': 'column', gap: '16px' }}>
            <For each={overview()}>{(p) => <ProjectCard project={p} />}</For>
          </div>
        </Show>
      </Show>
    </Page>
  )
}

function ProjectCard(props: { project: ProjectOverview }) {
  const [dsn] = createResource(() => api.projects.dsn(props.project.id))
  const recent = () => props.project.recent_issues.filter((i) => i.status === 'unresolved').slice(0, 3)

  return (
    <div class="card" style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '16px', padding: '18px 20px 14px' }}>
        <div style={{ flex: '1', 'min-width': 0, display: 'flex', 'flex-direction': 'column', gap: '5px' }}>
          <div style={{ display: 'flex', 'align-items': 'center', gap: '12px' }}>
            <A
              href={`/projects/${props.project.id}/issues`}
              class="mono cb-link"
              style={{ 'font-size': '16px', 'font-weight': 600, color: 'var(--text-hi)', 'white-space': 'nowrap' }}
            >
              {props.project.name}
            </A>
            <Show when={props.project.unresolved_count > 0}>
              <span class="livedot" title="actively receiving events" />
            </Show>
          </div>
          <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'white-space': 'nowrap' }}>
            <span class="mono" style={{ 'font-size': '12px', color: 'var(--text-faint)' }}>{props.project.slug}</span>
            <Show when={props.project.platform}>
              <span style={{ color: 'var(--text-faint)', opacity: 0.4 }}>·</span>
              <PlatformTag platform={props.project.platform} />
            </Show>
          </div>
        </div>
        <div style={{ display: 'flex', 'align-items': 'flex-start', gap: '24px', flex: 'none' }}>
          <Stat label="unresolved" value={props.project.unresolved_count} accent={props.project.unresolved_count > 0} />
          <Stat label="events / 24h" value={props.project.events_24h} />
        </div>
      </div>

      <div style={{ padding: '0 20px 14px' }}>
        <Show when={dsn()} fallback={<div class="skel" style={{ height: '40px' }} />}>
          {(d) => <CopyBlock value={d().dsn} />}
        </Show>
      </div>

      <div style={{ 'border-top': '1px solid var(--line-soft)', padding: '8px 10px 10px' }}>
        <Show
          when={recent().length > 0}
          fallback={
            <Voice style={{ padding: '10px' }}>
              {props.project.unresolved_count === 0 ? "nothing's on fire" : 'waiting for the first crash'}
            </Voice>
          }
        >
          <For each={recent()}>{(iss) => <MiniIssue issue={iss} />}</For>
        </Show>
      </div>

      <div style={{ display: 'flex', gap: '12px', padding: '0 20px 16px' }}>
        <A href={`/projects/${props.project.id}/issues`} class="btn sm">issues</A>
        <A href={`/projects/${props.project.id}/settings`} class="btn sm ghost">settings</A>
      </div>
    </div>
  )
}

function MiniIssue(props: { issue: Issue }) {
  const resolved = () => props.issue.status === 'resolved'
  return (
    <A
      href={`/issues/${props.issue.id}`}
      class="cb-minirow"
      style={{
        display: 'flex', 'align-items': 'center', gap: '12px', width: '100%', 'text-align': 'left',
        padding: '7px 10px', 'border-radius': '7px', opacity: resolved() ? 0.5 : 1,
      }}
    >
      <SevCue level={props.issue.level} />
      <span class="mono tnum" style={{ 'font-size': '12px', color: 'var(--text-lo)', width: '52px', 'text-align': 'right', flex: 'none' }}>
        {fmt(props.issue.event_count)}×
      </span>
      <Sparkline buckets={props.issue.last_24h_buckets} w={52} h={16} dim={resolved()} />
      <span class="mono" style={{ flex: '1', 'min-width': 0, overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap', 'font-size': '12.5px', color: 'var(--text)' }}>
        {props.issue.title}
      </span>
      <span class="mono" style={{ 'font-size': '11.5px', color: 'var(--text-faint)', flex: 'none' }}>{relTime(props.issue.last_seen)}</span>
    </A>
  )
}

function CreateProject(props: { onCancel: () => void; onCreated: () => void }) {
  const [name, setName] = createSignal('')
  const [platform, setPlatform] = createSignal('javascript')
  const [busy, setBusy] = createSignal(false)
  const [err, setErr] = createSignal<string | null>(null)

  const submit = async () => {
    if (!name().trim()) return
    setBusy(true)
    setErr(null)
    try {
      await api.projects.create({ name: name().trim(), platform: platform() || undefined })
      props.onCreated()
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class="card" style={{ padding: '20px' }}>
      <div class="mono" style={{ 'font-size': '13px', color: 'var(--text-hi)', 'margin-bottom': '14px' }}>// new project</div>
      <div style={{ display: 'flex', 'align-items': 'flex-end', gap: '12px', 'flex-wrap': 'wrap' }}>
        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', flex: '1', 'min-width': '200px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>name</span>
          <span class="field cb-focusring">
            <input
              class="input mono"
              autofocus
              placeholder="billing-worker"
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => e.key === 'Enter' && submit()}
            />
          </span>
        </label>
        <label style={{ display: 'flex', 'flex-direction': 'column', gap: '6px', width: '160px' }}>
          <span class="mono" style={{ 'font-size': '11px', color: 'var(--text-lo)' }}>platform</span>
          <span class="field cb-focusring" style={{ display: 'block' }}>
            <select class="input mono" value={platform()} onChange={(e) => setPlatform(e.currentTarget.value)} style={{ cursor: 'pointer' }}>
              <For each={['javascript', 'node', 'rust', 'python', 'go', 'ruby']}>{(p) => <option value={p}>{p}</option>}</For>
            </select>
          </span>
        </label>
        <button class={`btn primary ${busy() ? 'loading' : ''}`} disabled={!name().trim()} onClick={submit} style={{ position: 'relative' }}>create</button>
        <button class="btn ghost" onClick={props.onCancel}>cancel</button>
      </div>
      <Show when={err()}>{(m) => <div class="mono" style={{ 'font-size': '12px', color: 'var(--sev-error)', 'margin-top': '12px' }}>// {m()}</div>}</Show>
    </div>
  )
}

function ProjectSkeleton() {
  return (
    <div class="card" style={{ padding: '20px' }}>
      <div style={{ display: 'flex', 'align-items': 'center', gap: '16px' }}>
        <div style={{ flex: '1', display: 'flex', 'flex-direction': 'column', gap: '8px' }}>
          <div class="skel" style={{ width: '140px', height: '18px' }} />
          <div class="skel" style={{ width: '90px', height: '12px' }} />
        </div>
        <div class="skel" style={{ width: '70px', height: '34px' }} />
        <div class="skel" style={{ width: '70px', height: '34px' }} />
      </div>
      <div class="skel" style={{ height: '40px', 'margin-top': '16px' }} />
      <div class="skel" style={{ height: '30px', 'margin-top': '14px', width: '70%' }} />
    </div>
  )
}
