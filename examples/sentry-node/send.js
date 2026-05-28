// Smoke test: send a real error from @sentry/node into a running Crashbox.
// Usage:
//   DSN=http://PUBLIC_KEY@localhost:8080/1 node send.js
//   # or default DSN below:
//   node send.js

import * as Sentry from '@sentry/node'

const dsn = process.env.DSN || 'http://dockerkey@localhost:18080/1'

Sentry.init({
  dsn,
  // Keep noise low — we want the bare minimum the SDK sends.
  defaultIntegrations: false,
  tracesSampleRate: 0,
})

try {
  // The kind of error a real app would produce.
  const user = null
  console.log(user.name)
} catch (e) {
  const id = Sentry.captureException(e)
  console.log('captured:', id)
}

await Sentry.flush(2000)
console.log('done — check the UI at', new URL('/', dsn).toString().replace(/^https?:\/\/[^@]+@/, 'http://'))
