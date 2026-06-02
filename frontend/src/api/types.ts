export type User = {
  id: number
  email: string
  is_admin: boolean
  name?: string | null
  /** Global feature flag from the server; when false the UI hides the Live Logs section. */
  live_logs_enabled?: boolean
}

export type Project = {
  id: number
  name: string
  slug: string
  platform: string | null
  default_environment: string | null
  public_key: string
  created_at: string
  updated_at: string
}

export type Issue = {
  id: number
  project_id: number
  fingerprint: string
  title: string
  status: 'unresolved' | 'resolved'
  level: string | null
  platform: string | null
  first_seen: string
  last_seen: string
  event_count: number
  last_event_id: number | null
  created_at: string
  updated_at: string
  spike_alerted_at?: string | null
  snoozed_until?: string | null
  /** 24-element array, oldest→newest, of events per hour over last 24h. Server-populated on
   *  list / overview endpoints; absent on direct GET /api/issues/:id. */
  last_24h_buckets?: number[]
}

export type EventRow = {
  id: number
  event_id: string | null
  project_id: number
  issue_id: number | null
  timestamp: string | null
  received_at: string
  level: string | null
  platform: string | null
  environment: string | null
  release: string | null
  logger: string | null
  transaction_name: string | null
  message: string | null
  exception_type: string | null
  exception_value: string | null
  culprit: string | null
  server_name: string | null
  request_url: string | null
  user_id: string | null
  user_email: string | null
  fingerprint: string | null
  raw_json: string
}

export type EventDetail = {
  event: EventRow
  // Parsed raw Sentry payload — shape is whatever the SDK sent.
  data: Record<string, unknown>
}

export type DsnInfo = {
  dsn: string
  public_key: string
}

export type ProjectOverview = Project & {
  unresolved_count: number
  events_24h: number
  recent_issues: Issue[]
}

export type IssueFilters = {
  status?: 'unresolved' | 'resolved' | 'snoozed' | 'all'
  level?: string
  environment?: string
  release?: string
  query?: string
  limit?: number
  offset?: number
}

export type SnoozeAction = '1h' | '1d' | '1w' | 'forever' | 'wake'

export type LogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'fatal'

/** One ephemeral live-log line. Mirrors backend `livelog::LogRecord` (RAM-only, never persisted). */
export type LogRecord = {
  ts: string
  level: LogLevel
  message: string
  logger?: string
  trace_id?: string
  attrs?: Record<string, unknown>
}
