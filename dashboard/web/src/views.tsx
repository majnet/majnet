import { Link, useParams } from '@tanstack/react-router'
import { useApps, useDeploys, useEvents, useNodes, useProjects, useWhoami } from './api'
import { DeployStatus, latestEventFor, Pill, QueryState, short } from './ui'

const Chevron = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--faint)" strokeWidth="2"><path d="M9 6l6 6-6 6" /></svg>
)

// ── Projects ─────────────────────────────────────────────────────────────────
export function Projects() {
  const q = useProjects()
  const { data: me } = useWhoami()
  return (
    <>
      <div className="head">
        <h1>Projects</h1><span className="grow" />
        {me?.admin && <Link to="/new-project" className="btn primary">+ New project</Link>}
      </div>
      <QueryState isLoading={q.isLoading} error={q.error}>
        <div className="grid">
          {q.data?.length === 0 && <div className="empty">No projects registered yet.</div>}
          {q.data?.map((p) =>
            p.onboarded ? (
              <Link key={p.org} to="/projects/$org" params={{ org: p.org }} className="card">
                <h3>{p.name}</h3>
                <div className="meta">{p.org}</div>
                <div className="foot">
                  <span><b>{p.apps}</b> app{p.apps === 1 ? '' : 's'}</span>
                  <Pill kind="ok" dot>onboarded</Pill>
                </div>
              </Link>
            ) : (
              <div key={p.org} className="card" style={{ opacity: 0.6, borderStyle: 'dashed', cursor: 'default' }}>
                <h3>{p.name}</h3>
                <div className="meta">{p.org}</div>
                <div className="foot"><Pill kind="dim">registered · App not installed</Pill></div>
              </div>
            ),
          )}
        </div>
        <div className="note">
          Projects map 1:1 to GitHub orgs. A project is live only when it is listed in <code>projects.yaml</code> <b>and</b> the
          App is installed on the org. "New project" registers the org; org creation stays on GitHub.
        </div>
      </QueryState>
    </>
  )
}

// ── Project detail ───────────────────────────────────────────────────────────
export function ProjectDetail() {
  const { org } = useParams({ from: '/projects/$org' })
  const projects = useProjects()
  const name = projects.data?.find((x) => x.org === org)?.name ?? org
  const apps = useApps(org)
  const events = useEvents()
  const deploys = useDeploys(org)
  const pending = deploys.data?.length ?? 0

  return (
    <>
      <div className="crumb"><Link to="/">Projects</Link> / {name}</div>
      <div className="head">
        <h1>{name}</h1>
        <span className="head sub" style={{ fontFamily: 'var(--mono)' }}>{org}</span>
        <span className="grow" />
        <Link to="/projects/$org/deploys" params={{ org }} className="btn ghost sm">
          Deployments{pending ? ` · ${pending}` : ''}
        </Link>
        <Link to="/projects/$org/members" params={{ org }} className="btn ghost sm">Members</Link>
        <Link to="/projects/$org/new-app" params={{ org }} className="btn primary">+ New app</Link>
      </div>

      <div className="panel-h" style={{ border: 0, padding: '0 0 10px' }}><h2>Apps</h2></div>
      <QueryState isLoading={apps.isLoading} error={apps.error}>
        <div className="rows">
          {apps.data?.length === 0 && <div className="empty">No apps yet — create one.</div>}
          {apps.data?.map((a) => {
            const dm = [short(a.image), a.database].filter(Boolean).join('  ·  ')
            const ev = latestEventFor(events.data, name, a.name)
            return (
              <Link key={a.name} to="/projects/$org/apps/$app" params={{ org, app: a.name }} className="row link">
                <div>
                  <div className="nm">
                    {a.name}
                    {a.classes.map((c) => <Pill key={c} kind="cls">{c}</Pill>)}
                  </div>
                  <div className="dm">{dm || '—'}</div>
                </div>
                <div className="rt">
                  {a.host && <span style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--muted)' }}>{a.host}</span>}
                  <DeployStatus ev={ev} />
                  {Chevron}
                </div>
              </Link>
            )
          })}
        </div>
      </QueryState>
    </>
  )
}

// ── Nodes ────────────────────────────────────────────────────────────────────
const ZONE: Record<string, string> = { main: 'control plane', prod: 'public', private: 'internal' }
export function Nodes() {
  const q = useNodes()
  return (
    <>
      <div className="head"><h1>Nodes</h1></div>
      <QueryState isLoading={q.isLoading} error={q.error}>
        <div className="rows">
          {q.data?.length === 0 && <div className="empty">No nodes enrolled.</div>}
          {q.data?.map((n) => {
            const enrolled = !!n.wireguard_pubkey
            const ep = [n.wireguard_ip, n.public_endpoint].filter(Boolean).join(' · ')
            return (
              <div key={n.role} className="row" style={enrolled ? undefined : { opacity: 0.6 }}>
                <div>
                  <div className="nm">{n.name} <Pill kind="dim">{ZONE[n.role] ?? n.role}</Pill></div>
                  <div className="dm">{ep || '—'}</div>
                </div>
                {enrolled ? <Pill kind="ok" dot>enrolled</Pill> : <Pill kind="dim">pending</Pill>}
              </div>
            )
          })}
        </div>
      </QueryState>
    </>
  )
}

// ── Activity ─────────────────────────────────────────────────────────────────
export function Activity() {
  const q = useEvents(100)
  return (
    <>
      <div className="head"><h1>Activity</h1></div>
      <QueryState isLoading={q.isLoading} error={q.error}>
        <div className="panel"><div className="panel-b" style={{ padding: '6px 18px 14px' }}>
          <table className="ev">
            <thead><tr><th>time</th><th>project</th><th>node</th><th>action</th><th>result</th><th>commit</th></tr></thead>
            <tbody>
              {q.data?.length === 0 && <tr><td colSpan={6} className="empty">No events yet.</td></tr>}
              {q.data?.map((e, i) => (
                <tr key={i}>
                  <td>{e.at}</td><td>{e.project}</td><td>{e.node}</td><td>{e.action}</td>
                  <td style={{ color: e.result.startsWith('FAILED') ? 'var(--bad)' : 'inherit' }}>{e.result}</td>
                  <td>{e.commit.slice(0, 12)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div></div>
      </QueryState>
    </>
  )
}

