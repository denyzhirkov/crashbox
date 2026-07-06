import type {
  ApiToken,
  CreatedToken,
  DsnInfo,
  EventDetail,
  HeartbeatMonitor,
  Issue,
  IssueFilters,
  Project,
  ProjectOverview,
  User,
} from './types'

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, {
    credentials: 'same-origin',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json',
      ...(init?.headers ?? {}),
    },
    ...init,
  })
  if (!resp.ok) {
    let msg = resp.statusText
    try {
      const body = await resp.json()
      if (body && typeof body.error === 'string') msg = body.error
    } catch {
      // body wasn't json — fine, keep statusText.
    }
    throw new ApiError(resp.status, msg)
  }
  if (resp.status === 204) return undefined as T
  return (await resp.json()) as T
}

export const api = {
  auth: {
    me: () => req<User>('/api/auth/me'),
    login: (email: string, password: string) =>
      req<{ user: User }>('/api/auth/login', {
        method: 'POST',
        body: JSON.stringify({ email, password }),
      }),
    logout: () => req<{ ok: true }>('/api/auth/logout', { method: 'POST' }),
  },
  projects: {
    list: () => req<Project[]>('/api/projects'),
    overview: () => req<ProjectOverview[]>('/api/projects/overview'),
    get: (id: number) => req<Project>(`/api/projects/${id}`),
    create: (body: { name: string; platform?: string }) =>
      req<Project>('/api/projects', { method: 'POST', body: JSON.stringify(body) }),
    dsn: (id: number) => req<DsnInfo>(`/api/projects/${id}/dsn`),
    rotateKey: (id: number) =>
      req<DsnInfo>(`/api/projects/${id}/rotate-key`, { method: 'POST' }),
  },
  issues: {
    list: (projectId: number, filters: IssueFilters = {}, tags: Array<[string, string]> = []) => {
      const qs = new URLSearchParams()
      for (const [k, v] of Object.entries(filters)) {
        if (v !== undefined && v !== null && v !== '') qs.set(k, String(v))
      }
      // ?tag=key=value can repeat — URLSearchParams.append handles that.
      for (const [k, v] of tags) qs.append('tag', `${k}=${v}`)
      const suffix = qs.toString() ? `?${qs}` : ''
      return req<Issue[]>(`/api/projects/${projectId}/issues${suffix}`)
    },
    get: (id: number) => req<Issue>(`/api/issues/${id}`),
    setStatus: (id: number, status: 'resolved' | 'unresolved') =>
      req<Issue>(`/api/issues/${id}`, {
        method: 'PATCH',
        body: JSON.stringify({ status }),
      }),
    snooze: (id: number, action: '1h' | '1d' | '1w' | 'forever' | 'wake') =>
      req<Issue>(`/api/issues/${id}`, {
        method: 'PATCH',
        body: JSON.stringify({ snooze: action }),
      }),
    events: (id: number, limit = 50, offset = 0) =>
      req<EventRow[]>(`/api/issues/${id}/events?limit=${limit}&offset=${offset}`),
  },
  events: {
    get: (id: number) => req<EventDetail>(`/api/events/${id}`),
  },
  tokens: {
    list: () => req<ApiToken[]>('/api/tokens'),
    create: (body: { name: string; expires_in_days?: number }) =>
      req<CreatedToken>('/api/tokens', { method: 'POST', body: JSON.stringify(body) }),
    remove: (id: number) => req<void>(`/api/tokens/${id}`, { method: 'DELETE' }),
  },
  heartbeats: {
    list: (projectId: number) =>
      req<HeartbeatMonitor[]>(`/api/projects/${projectId}/heartbeats`),
    create: (
      projectId: number,
      body: { name: string; description?: string; period_seconds: number; grace_seconds?: number },
    ) =>
      req<HeartbeatMonitor>(`/api/projects/${projectId}/heartbeats`, {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    update: (
      id: number,
      // description: omit = keep, '' = clear, value = set
      body: { name?: string; description?: string; period_seconds?: number; grace_seconds?: number; status?: 'paused' | 'pending' },
    ) =>
      req<HeartbeatMonitor>(`/api/heartbeats/${id}`, {
        method: 'PATCH',
        body: JSON.stringify(body),
      }),
    remove: (id: number) => req<void>(`/api/heartbeats/${id}`, { method: 'DELETE' }),
  },
  livelog: {
    // The live-log stream is consumed via EventSource, not fetch — this just builds the URL.
    // Cookies ride along automatically on same-origin EventSource requests.
    streamUrl: (projectId: number, opts: { level?: string } = {}) => {
      const qs = new URLSearchParams()
      if (opts.level) qs.set('level', opts.level)
      const suffix = qs.toString() ? `?${qs}` : ''
      return `/api/projects/${projectId}/logs/stream${suffix}`
    },
  },
}

// Re-export so callers only need one import path.
export type {
  ApiToken,
  CreatedToken,
  DsnInfo,
  EventDetail,
  EventRow,
  HeartbeatMonitor,
  HeartbeatStatus,
  Issue,
  IssueFilters,
  LogLevel,
  LogRecord,
  Project,
  ProjectOverview,
  User,
} from './types'
import type { EventRow } from './types'
