import { useEffect, useState } from 'react'
import { Link, useParams } from '@tanstack/react-router'
import { send, urls, useApps, useEvents, useManifest, type ManifestFile } from './api'
import { useApiMutation } from './mutations'
import { DeployStatus, QueryState, short } from './ui'
import { fromData, ManifestForm, toManifest, type ManifestDraft } from './manifestForm'

const FILES = ['base.yaml', 'stable.yaml', 'production.yaml', 'ephemeral.yaml']

export function AppDetail() {
  const { org, app } = useParams({ from: '/projects/$org/apps/$app' })
  const apps = useApps(org)
  const a = apps.data?.find((x) => x.name === app)
  const manifest = useManifest(org, app)
  const events = useEvents()
  const appEvents = (events.data ?? []).filter((e) => e.action.trim().split(/\s+/).pop() === app)

  const act = useApiMutation({ invalidate: [['events']] })
  const deploy = useApiMutation({ invalidate: [['deploys', org], ['events']] })

  return (
    <>
      <div className="crumb">
        <Link to="/">Projects</Link> / <Link to="/projects/$org" params={{ org }}>{org}</Link> / {app}
      </div>
      <div className="head">
        <h1>{app}</h1><span className="grow" />
        {a && a.classes.length > 0 && <RestartControl org={org} app={app} classes={a.classes} run={act.mutate} busy={act.isPending} />}
        <button className="btn ghost sm" disabled={deploy.isPending} onClick={() => {
          if (confirm(`Revert the last change on ${org}/ops?`)) deploy.mutate(() => send(urls.rollback(org)))
        }}>Roll back</button>
        <button className="btn primary sm" disabled={deploy.isPending} onClick={() => {
          if (confirm(`Promote ${app} to production? An admin still merges the render PR.`)) deploy.mutate(() => send(urls.promote(org, app)))
        }}>Promote → production</button>
      </div>

      {a && (
        <div className="panel"><div className="panel-b" style={{ gap: 10 }}>
          <div className="kv"><span className="k">Deploy status</span><span className="v">
            <DeployStatus ev={appEvents[0]} /> {appEvents[0] && <span style={{ color: 'var(--muted)' }}>{appEvents[0].result} · {appEvents[0].at}</span>}
          </span></div>
          <div className="kv"><span className="k">Classes</span><span className="v">{a.classes.join(', ') || '—'}</span></div>
          <div className="kv"><span className="k">Domains</span><span className="v">{a.domains.join(', ') || '—'}</span></div>
          <div className="kv"><span className="k">Image</span><span className="v">{short(a.image)}</span></div>
          {a.database && <div className="kv"><span className="k">Database</span><span className="v">{a.database}</span></div>}
        </div></div>
      )}

      <QueryState isLoading={manifest.isLoading} error={manifest.error}>
        {manifest.data && <ManifestEditor org={org} app={app} files={manifest.data} />}
      </QueryState>

      {appEvents.length > 0 && (
        <div className="panel">
          <div className="panel-h"><h2>Recent deploys</h2></div>
          <div className="panel-b" style={{ padding: '6px 18px 14px' }}>
            <table className="ev">
              <thead><tr><th>time</th><th>node</th><th>action</th><th>result</th><th>commit</th></tr></thead>
              <tbody>{appEvents.slice(0, 8).map((e, i) => (
                <tr key={i}><td>{e.at}</td><td>{e.node}</td><td>{e.action}</td>
                  <td style={{ color: e.result.startsWith('FAILED') ? 'var(--bad)' : 'inherit' }}>{e.result}</td>
                  <td>{e.commit.slice(0, 12)}</td></tr>
              ))}</tbody>
            </table>
          </div>
        </div>
      )}
    </>
  )
}

function RestartControl({ org, app, classes, run, busy }: {
  org: string; app: string; classes: string[]; run: (fn: () => Promise<string>) => void; busy: boolean
}) {
  const [cls, setCls] = useState(classes[0]!)
  return (
    <>
      <select style={{ width: 'auto' }} value={cls} onChange={(e) => setCls(e.target.value)}>
        {classes.map((c) => <option key={c}>{c}</option>)}
      </select>
      <button className="btn ghost sm" disabled={busy} onClick={() => run(() => send(urls.restart(org, cls, app)))}>Restart</button>
    </>
  )
}

// ── manifest editor: file tabs + Form/YAML ────────────────────────────────────
function ManifestEditor({ org, app, files }: { org: string; app: string; files: Record<string, ManifestFile> }) {
  const [file, setFile] = useState('base.yaml')
  const [mode, setMode] = useState<'form' | 'yaml'>('form')
  const [draft, setDraft] = useState<ManifestDraft>(() => fromData(files[file]?.data))
  const [yaml, setYaml] = useState(() => files[file]?.yaml ?? '')

  // Reset both editors when the file changes or the manifest refetches (post-save).
  useEffect(() => {
    setDraft(fromData(files[file]?.data))
    setYaml(files[file]?.yaml ?? '')
  }, [file, files])

  const save = useApiMutation({ invalidate: [['manifest', org, app], ['apps', org], ['deploys', org], ['events']] })
  const onSave = () => {
    if (mode === 'form') save.mutate(() => send(urls.manifestFile(org, app, file), { method: 'PUT', json: toManifest(draft, file, app) }))
    else save.mutate(() => send(urls.manifestFile(org, app, file), { method: 'PUT', body: yaml }))
  }

  return (
    <div className="panel">
      <div className="panel-h">
        <div className="tabs" style={{ border: 0, padding: 0 }}>
          {FILES.map((f) => (
            <button key={f} className={`tab ${f === file ? 'on' : ''}`} onClick={() => setFile(f)}>
              {f}{!files[f] && <span style={{ color: 'var(--faint)' }}> (new)</span>}
            </button>
          ))}
        </div>
        <span className="grow" />
        <div className="actions">
          <button className={`btn ghost sm ${mode === 'form' ? 'primary' : ''}`} onClick={() => setMode('form')}>Form</button>
          <button className={`btn ghost sm ${mode === 'yaml' ? 'primary' : ''}`} onClick={() => setMode('yaml')}>YAML</button>
        </div>
      </div>
      <div className="panel-b">
        {mode === 'form'
          ? <ManifestForm file={file} draft={draft} onChange={setDraft} />
          : <textarea spellCheck={false} value={yaml} onChange={(e) => setYaml(e.target.value)} />}
        <div className="actions" style={{ marginTop: 14 }}>
          <button className="btn primary sm" disabled={save.isPending} onClick={onSave}>Save &amp; commit</button>
          <span className="h">Validated + committed to ops main; a render PR follows. production.yaml requires admin. Switching Form/YAML reloads the last saved state.</span>
        </div>
      </div>
    </div>
  )
}
