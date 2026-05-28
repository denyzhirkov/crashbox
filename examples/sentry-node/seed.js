// Seed Crashbox with a varied set of realistic errors for UI inspection.
// Sends events sequentially via raw fetch (workaround for the SQLite-busy bug
// triggered by Sentry SDK's batched flush — tracked separately).
//
// Usage:
//   node seed.js [DSN]
//   node seed.js http://demokey@localhost:8080/1

const dsn = process.argv[2] || process.env.DSN || 'http://demokey@localhost:8080/1'

// Parse `http(s)://<public_key>@<host>[:port]/<project_id>`
const m = dsn.match(/^(https?):\/\/([^@]+)@([^/]+)\/(\d+)/)
if (!m) {
  console.error('bad DSN:', dsn)
  process.exit(1)
}
const [, scheme, publicKey, host, projectId] = m
const endpoint = `${scheme}://${host}/api/${projectId}/envelope/`

function randomHex(n) {
  let s = ''
  for (let i = 0; i < n; i++) s += Math.floor(Math.random() * 16).toString(16)
  return s
}

async function sendEvent(event) {
  const payload = JSON.stringify(event)
  const envelope =
    JSON.stringify({ event_id: event.event_id, sent_at: new Date().toISOString() }) +
    '\n' +
    JSON.stringify({ type: 'event', length: Buffer.byteLength(payload) }) +
    '\n' +
    payload +
    '\n'
  const resp = await fetch(endpoint, {
    method: 'POST',
    headers: {
      'content-type': 'application/x-sentry-envelope',
      'x-sentry-auth': `Sentry sentry_version=7, sentry_key=${publicKey}, sentry_client=crashbox-seed/0.1`,
    },
    body: envelope,
  })
  if (!resp.ok) {
    console.error('  !', resp.status, await resp.text())
  }
  return resp.ok
}

function frame(fn, file, line, inApp = true, module = undefined) {
  return { function: fn, filename: file, lineno: line, in_app: inApp, module }
}

const REL = '1.4.2'
const ENV = 'production'
const NOW = () => new Date().toISOString()

const cases = []

// 1. The flagship: TypeError, repeated many times → fattest issue with breadcrumbs/user/tags
for (let i = 0; i < 8; i++) {
  cases.push({
    event_id: randomHex(32),
    timestamp: NOW(),
    platform: 'node',
    level: 'error',
    environment: ENV,
    release: REL,
    server_name: 'web-1',
    transaction: 'POST /api/cart/add',
    user: { id: `u_${1000 + i}`, email: `customer${i}@example.com` },
    tags: { route: '/api/cart/add', shard: i % 2 === 0 ? 'us-east' : 'eu-west' },
    exception: {
      values: [
        {
          type: 'TypeError',
          value: "Cannot read properties of null (reading 'id')",
          stacktrace: {
            frames: [
              frame('Module.bootstrap', 'node:internal/modules/run_main', 117, false),
              frame('handleRequest', '/app/src/server.js', 84, true),
              frame('addToCart', '/app/src/routes/cart.js', 42, true),
              frame('Cart.add', '/app/src/lib/cart.ts', 17, true),
            ],
          },
          mechanism: { type: 'generic', handled: true },
        },
      ],
    },
    breadcrumbs: {
      values: [
        { category: 'navigation', message: 'GET /cart', level: 'info' },
        { category: 'http', message: 'GET /api/products/42 → 200', level: 'info' },
        { category: 'auth', message: 'session restored', level: 'info' },
        { category: 'ui.click', message: 'AddToCartButton clicked', level: 'info' },
      ],
    },
  })
}

// 2. RangeError, single occurrence — separate issue
cases.push({
  event_id: randomHex(32),
  timestamp: NOW(),
  platform: 'node',
  level: 'error',
  environment: ENV,
  release: REL,
  transaction: 'GET /api/admin/jobs',
  user: { id: 'u_admin', email: 'ops@example.com' },
  tags: { route: '/api/admin/jobs' },
  exception: {
    values: [
      {
        type: 'RangeError',
        value: 'Invalid array length',
        stacktrace: {
          frames: [
            frame('listJobs', '/app/src/routes/admin.js', 12, true),
            frame('formatJobs', '/app/src/lib/jobs.ts', 88, true),
          ],
        },
      },
    ],
  },
  breadcrumbs: {
    values: [
      { category: 'auth', message: 'admin token verified', level: 'info' },
      { category: 'db', message: 'SELECT * FROM jobs ORDER BY created_at', level: 'debug' },
    ],
  },
})

// 3. Custom DatabaseError, several occurrences with normalized number → grouped together
for (let i = 0; i < 3; i++) {
  cases.push({
    event_id: randomHex(32),
    timestamp: NOW(),
    platform: 'node',
    level: 'error',
    environment: i === 0 ? 'staging' : ENV,
    release: REL,
    transaction: 'POST /api/orders',
    user: { id: `u_${2000 + i}`, email: `user${i}@example.com` },
    tags: { route: '/api/orders', db: 'orders-primary' },
    exception: {
      values: [
        {
          type: 'DatabaseError',
          value: `order ${600000000 + i} failed: deadlock with pid ${10_000 + i}`,
          stacktrace: {
            frames: [
              frame('handleRequest', '/app/src/server.js', 84, true),
              frame('createOrder', '/app/src/routes/orders.js', 31, true),
              frame('Orders.insert', '/app/src/db/orders.ts', 56, true),
            ],
          },
        },
      ],
    },
    breadcrumbs: {
      values: [
        { category: 'http.request', message: 'POST /api/orders', level: 'info' },
        { category: 'auth', message: 'jwt verified', level: 'info' },
        { category: 'validation', message: 'body schema ok', level: 'info' },
        { category: 'db.query', message: 'INSERT INTO orders ... rows=1', level: 'info' },
        { category: 'db.query', message: 'INSERT INTO order_items ...', level: 'error' },
      ],
    },
  })
}

// 4. Pure message capture (warning level) — message grouping with normalization
cases.push({
  event_id: randomHex(32),
  timestamp: NOW(),
  platform: 'node',
  level: 'warning',
  environment: ENV,
  release: REL,
  message: 'feature flag "new_checkout" missing for tier "enterprise"',
  tags: { flag: 'new_checkout' },
})
cases.push({
  event_id: randomHex(32),
  timestamp: NOW(),
  platform: 'node',
  level: 'warning',
  environment: ENV,
  release: REL,
  message: 'feature flag "new_checkout" missing for tier "pro"',
  tags: { flag: 'new_checkout' },
})

// 5. ReferenceError
cases.push({
  event_id: randomHex(32),
  timestamp: NOW(),
  platform: 'node',
  level: 'error',
  environment: ENV,
  release: REL,
  transaction: 'POST /api/webhooks/stripe',
  tags: { route: '/api/webhooks/stripe', source: 'stripe' },
  exception: {
    values: [
      {
        type: 'ReferenceError',
        value: 'handleStripeEvent is not defined',
        stacktrace: {
          frames: [
            frame('handleRequest', '/app/src/server.js', 84, true),
            frame('webhook', '/app/src/routes/webhooks.js', 19, true),
          ],
        },
      },
    ],
  },
  breadcrumbs: {
    values: [
      { category: 'webhook', message: 'stripe signature verified', level: 'info' },
      { category: 'webhook', message: 'event type=charge.succeeded', level: 'info' },
    ],
  },
})

// 6. SyntaxError burst from a worker job — pile up to test the scrubber
for (let i = 0; i < 5; i++) {
  cases.push({
    event_id: randomHex(32),
    timestamp: NOW(),
    platform: 'node',
    level: 'error',
    environment: ENV,
    release: REL,
    transaction: 'job:resize_image',
    tags: { worker: 'image-resizer', queue: 'media' },
    exception: {
      values: [
        {
          type: 'SyntaxError',
          value: 'Unexpected token n in JSON at position 0',
          stacktrace: {
            frames: [
              frame('runJob', '/app/src/workers/runner.js', 22, true),
              frame('resizeImage', '/app/src/workers/resize.js', 9, true),
              frame('JSON.parse', '<anonymous>', null, false),
            ],
          },
        },
      ],
    },
    breadcrumbs: {
      values: [
        { category: 'job', message: `picked up job_id=${20_000 + i}`, level: 'info' },
        { category: 'fs', message: '/tmp/upload_xyz.png read', level: 'info' },
      ],
    },
  })
}

// 7. Info-level message — level variety
cases.push({
  event_id: randomHex(32),
  timestamp: NOW(),
  platform: 'node',
  level: 'info',
  environment: ENV,
  release: REL,
  message: 'background sync completed in 412ms',
  tags: { worker: 'sync' },
})

console.log(`sending ${cases.length} events sequentially → ${endpoint}`)
let ok = 0
for (const ev of cases) {
  if (await sendEvent(ev)) ok++
}
console.log(`done — ${ok}/${cases.length} accepted`)
