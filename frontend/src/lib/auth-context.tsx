import {
  createContext,
  createResource,
  type JSX,
  type Resource,
  useContext,
} from 'solid-js'
import { api, ApiError, type User } from '../api/client'

type AuthCtx = {
  user: Resource<User | null>
  refresh: () => void
  logout: () => Promise<void>
}

const Ctx = createContext<AuthCtx>()

export function AuthProvider(props: { children: JSX.Element }) {
  const [user, { refetch }] = createResource<User | null>(async () => {
    try {
      return await api.auth.me()
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) return null
      throw e
    }
  })

  const ctx: AuthCtx = {
    user,
    refresh: () => void refetch(),
    logout: async () => {
      await api.auth.logout()
      void refetch()
    },
  }
  return <Ctx.Provider value={ctx}>{props.children}</Ctx.Provider>
}

export function useAuth(): AuthCtx {
  const c = useContext(Ctx)
  if (!c) throw new Error('useAuth outside AuthProvider')
  return c
}
