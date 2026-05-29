// Per-project saved filter views in localStorage. Keep it dumb on purpose —
// when the user wants this to sync across devices, we'll move it server-side.

import type { IssueFilters } from '../api/types'

export type SavedView = {
  id: string
  name: string
  filters: IssueFilters
  tags: Array<[string, string]>
}

const key = (projectId: number) => `crashbox.savedViews.${projectId}`

export function loadViews(projectId: number): SavedView[] {
  try {
    const raw = localStorage.getItem(key(projectId))
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed as SavedView[]
  } catch {
    return []
  }
}

export function saveView(projectId: number, view: SavedView) {
  const existing = loadViews(projectId).filter((v) => v.id !== view.id)
  const next = [...existing, view]
  localStorage.setItem(key(projectId), JSON.stringify(next))
}

export function removeView(projectId: number, id: string) {
  const next = loadViews(projectId).filter((v) => v.id !== id)
  localStorage.setItem(key(projectId), JSON.stringify(next))
}
