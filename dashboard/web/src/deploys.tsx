import { Link, useParams } from '@tanstack/react-router'
import { send, urls, useDeploys, type DeployFile } from './api'
import { useApiMutation } from './mutations'
import { Pill, QueryState } from './ui'

function DiffBlock({ patch }: { patch: string }) {
  return (
    <div className="term" style={{ marginTop: 8 }}>
      <pre><code>
        {patch.split('\n').map((line, i) => {
          const c = line[0] === '+' ? 'var(--good)' : line[0] === '-' ? 'var(--bad)'
            : line.startsWith('@@') ? 'var(--term-accent)' : 'var(--term-text)'
          return <span key={i} style={{ color: c, display: 'block' }}>{line || ' '}</span>
        })}
      </code></pre>
    </div>
  )
}

function FileDiff({ f }: { f: DeployFile }) {
  return (
    <details>
      <summary style={{ cursor: 'pointer', fontFamily: 'var(--mono)', fontSize: 12.5 }}>
        {f.filename} <span style={{ color: 'var(--good)' }}>+{f.additions}</span> <span style={{ color: 'var(--bad)' }}>−{f.deletions}</span>{' '}
        <Pill kind="dim">{f.status}</Pill>
      </summary>
      {f.patch && <DiffBlock patch={f.patch} />}
    </details>
  )
}

export function Deploys() {
  const { org } = useParams({ from: '/projects/$org/deploys' })
  const q = useDeploys(org)
  const m = useApiMutation({ invalidate: [['deploys', org], ['events']] })

  return (
    <>
      <div className="crumb"><Link to="/">Projects</Link> / <Link to="/projects/$org" params={{ org }}>{org}</Link> / Deployments</div>
      <div className="head"><h1>Deployments</h1><span className="head sub">pending render PRs on {org}/ops</span></div>
      <QueryState isLoading={q.isLoading} error={q.error}>
        {q.data?.length === 0 && (
          <div className="empty">No pending deployment requests. Production changes appear here as render PRs awaiting review; stable auto-deploys.</div>
        )}
        {q.data?.map((d) => (
          <div key={d.number} className="panel">
            <div className="panel-h">
              <Pill kind="cls">{d.class}</Pill>
              <h2 style={{ fontWeight: 600 }}>#{d.number} · {d.title}</h2>
              <span className="grow" />
              <button className="btn primary sm" disabled={m.isPending} onClick={() => {
                if (confirm(`Merge PR #${d.number} and deploy env/${d.class}?`)) m.mutate(() => send(urls.deployMerge(org, d.number)))
              }}>{d.class === 'production' ? 'Approve & deploy' : 'Merge & deploy'}</button>
              <button className="btn danger sm" disabled={m.isPending} onClick={() => {
                if (confirm(`Close PR #${d.number} without deploying?`)) m.mutate(() => send(urls.deployClose(org, d.number)))
              }}>Close</button>
            </div>
            <div className="panel-b">
              {d.files.length === 0 && <div className="h">No file changes.</div>}
              {d.files.map((f) => <FileDiff key={f.filename} f={f} />)}
            </div>
          </div>
        ))}
      </QueryState>
    </>
  )
}
