import { Navigate, Route, Router } from '@solidjs/router'
import { lazy } from 'solid-js'
import { AppShell } from './components/AppShell'
import { RequireAuth } from './components/RequireAuth'
import { AuthProvider } from './lib/auth-context'

const Login = lazy(() => import('./pages/Login'))
const Projects = lazy(() => import('./pages/Projects'))
const Issues = lazy(() => import('./pages/Issues'))
const IssueDetail = lazy(() => import('./pages/IssueDetail'))
const Settings = lazy(() => import('./pages/Settings'))
const LiveLogs = lazy(() => import('./pages/LiveLogs'))

function Shell(props: { children: any }) {
  return (
    <RequireAuth>
      <AppShell>{props.children}</AppShell>
    </RequireAuth>
  )
}

export default function App() {
  return (
    <AuthProvider>
      <Router>
        <Route path="/login" component={Login} />
        <Route path="/" component={() => <Navigate href="/projects" />} />
        <Route path="/projects" component={() => <Shell><Projects /></Shell>} />
        <Route
          path="/projects/:projectId/issues"
          component={() => <Shell><Issues /></Shell>}
        />
        <Route
          path="/projects/:projectId/settings"
          component={() => <Shell><Settings /></Shell>}
        />
        <Route
          path="/projects/:projectId/logs"
          component={() => <Shell><LiveLogs /></Shell>}
        />
        <Route
          path="/issues/:issueId"
          component={() => <Shell><IssueDetail /></Shell>}
        />
        <Route
          path="*"
          component={() => (
            <Shell>
              <div class="voice" style={{ padding: '36px 24px' }}><span class="pfx">// </span>not found</div>
            </Shell>
          )}
        />
      </Router>
    </AuthProvider>
  )
}
