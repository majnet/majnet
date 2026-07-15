// Minimal zero-dependency HTTP server with the MajNet standard endpoints
// (design doc §16): `/healthz` (liveness — the platform's default health path)
// and `/info` (build metadata the reconciler scrapes at deploy time and shows
// per env in the dashboard). Build metadata is injected at image-build time via
// Docker ARGs → ENV (see Dockerfile + the build/release workflows). Replace the
// catch-all with your real app.
import { createServer } from 'node:http'

const PORT = Number(process.env.PORT) || 8080

const INFO = {
  version: process.env.APP_VERSION || 'dev',
  commit: process.env.GIT_COMMIT || 'unknown',
  build_time: process.env.BUILD_TIME || null,
}

const server = createServer((req, res) => {
  if (req.url === '/healthz') {
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('ok')
    return
  }
  if (req.url === '/info') {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(JSON.stringify(INFO))
    return
  }
  res.writeHead(200, { 'content-type': 'text/plain' })
  res.end('web-app is running')
})

server.listen(PORT, () => console.log(`web-app listening on :${PORT}`))
