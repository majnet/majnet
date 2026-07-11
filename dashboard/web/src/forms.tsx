import { useState } from 'react'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { send, urls } from './api'
import { useApiMutation } from './mutations'
import { useToast } from './ui'

// ── New project ──────────────────────────────────────────────────────────────
export function NewProject() {
  const nav = useNavigate()
  const toast = useToast()
  const [name, setName] = useState('')
  const [org, setOrg] = useState('')
  const m = useApiMutation({ invalidate: [['projects']], onDone: () => nav({ to: '/' }) })

  return (
    <>
      <div className="crumb"><Link to="/">Projects</Link> / New</div>
      <div className="head"><h1>New project</h1></div>
      <div className="panel"><div className="panel-b">
        <div className="banner">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="9" /><path d="M12 8h.01M11 12h1v4h1" /></svg>
          <div>Create the GitHub org yourself (GitHub has no org-creation API), then register it here. Discovery needs the org listed <b>and</b> the App installed.</div>
        </div>
        <div className="row2">
          <div className="field"><label>Project name</label>
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="blog" />
            <span className="h">Lowercase slug, used in the dashboard.</span></div>
          <div className="field"><label>GitHub org</label>
            <input type="text" value={org} onChange={(e) => setOrg(e.target.value)} placeholder="majksa-projects" />
            <span className="h">The org this project's repos live in.</span></div>
        </div>
        <div className="field"><label>1 · Install the App on the org</label>
          <div className="term"><pre><code>https://github.com/apps/majnet-platform/installations/new</code></pre></div></div>
        <div className="actions">
          <button className="btn primary" disabled={m.isPending} onClick={() => {
            if (!name.trim() || !org.trim()) return toast('name and org are required', true)
            m.mutate(() => send(urls.projects, { json: { name: name.trim(), org: org.trim() } }))
          }}>Register project</button>
          <span className="h">Commits to projects.yaml; the ops repo is created on the next org sync.</span>
        </div>
      </div></div>
    </>
  )
}

// ── New app ──────────────────────────────────────────────────────────────────
const CLASSES = ['production', 'stable', 'ephemeral'] as const
export function NewApp() {
  const { org } = useParams({ from: '/projects/$org/new-app' })
  const nav = useNavigate()
  const toast = useToast()
  const [name, setName] = useState('')
  const [image, setImage] = useState('')
  const [host, setHost] = useState('')
  const [port, setPort] = useState('8080')
  const [domains, setDomains] = useState('')
  const [database, setDatabase] = useState('')
  const [classes, setClasses] = useState<string[]>(['production'])
  const m = useApiMutation({ invalidate: [['apps', org]], onDone: () => nav({ to: '/projects/$org', params: { org } }) })

  const toggle = (c: string) => setClasses((cs) => (cs.includes(c) ? cs.filter((x) => x !== c) : [...cs, c]))

  return (
    <>
      <div className="crumb"><Link to="/">Projects</Link> / <Link to="/projects/$org" params={{ org }}>{org}</Link> / New app</div>
      <div className="head"><h1>New app</h1></div>
      <div className="panel"><div className="panel-b">
        <div className="row2">
          <div className="field"><label>App name</label>
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="blog" />
            <span className="h">Lowercase; its manifest directory.</span></div>
          <div className="field"><label>Image</label>
            <input type="text" value={image} onChange={(e) => setImage(e.target.value)} placeholder="ghcr.io/org/app@sha256:…" />
            <span className="h">Digest-pinned; tags are rejected.</span></div>
        </div>
        <div className="row2">
          <div className="field"><label>Primary domain <span style={{ color: 'var(--faint)' }}>— optional</span></label>
            <input type="text" value={host} onChange={(e) => setHost(e.target.value)} placeholder="blog.majksa.cz" />
            <span className="h">Cloudflare + cert handled automatically for production.</span></div>
          <div className="field"><label>Container port</label>
            <input type="number" value={port} onChange={(e) => setPort(e.target.value)} min={1} max={65535} /></div>
        </div>
        <div className="field"><label>Additional domains <span style={{ color: 'var(--faint)' }}>— optional, one per line</span></label>
          <textarea value={domains} onChange={(e) => setDomains(e.target.value)} style={{ minHeight: 60 }} placeholder="www.majksa.cz" /></div>
        <div className="field"><label>Classes</label>
          <div className="actions">
            {CLASSES.map((c) => (
              <label key={c} className={`pill ${classes.includes(c) ? 'cls' : 'dim'}`} style={{ cursor: 'pointer' }}>
                <input type="checkbox" checked={classes.includes(c)} onChange={() => toggle(c)} style={{ width: 'auto' }} /> {c}
              </label>
            ))}
          </div>
          <span className="h">Which environments this app deploys to. Production goes through the reviewed render PR.</span></div>
        <div className="field"><label>Database <span style={{ color: 'var(--faint)' }}>— optional</span></label>
          <select value={database} onChange={(e) => setDatabase(e.target.value)}>
            <option value="">none</option><option>postgres</option><option>mariadb</option><option>valkey</option><option>mongodb</option>
          </select></div>
        <div className="actions">
          <button className="btn primary" disabled={m.isPending} onClick={() => {
            if (!name.trim() || !image.trim()) return toast('name and image are required', true)
            if (!classes.length) return toast('select at least one class', true)
            m.mutate(() => send(urls.apps(org), {
              json: {
                name: name.trim(), image: image.trim(), host: host.trim(), port: Number(port),
                domains: domains.split('\n').map((s) => s.trim()).filter(Boolean),
                classes, database: database || null,
              },
            }))
          }}>Create app</button>
          <span className="h">Writes base.yaml + overlays to the ops repo.</span>
        </div>
      </div></div>
    </>
  )
}
