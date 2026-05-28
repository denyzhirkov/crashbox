export type User = {
  id: number
  email: string
  is_admin: boolean
  name?: string | null
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
  status?: 'unresolved' | 'resolved' | 'all'
  level?: string
  environment?: string
  release?: string
  query?: string
  limit?: number
  offset?: number
}
