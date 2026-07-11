import { useState } from 'react'
import { Link, useParams } from '@tanstack/react-router'
import { send, urls, useMembers } from './api'
import { useApiMutation } from './mutations'
import { QueryState, useToast } from './ui'

export function Members() {
  const { org } = useParams({ from: '/projects/$org/members' })
  const q = useMembers(org)
  const toast = useToast()
  const [user, setUser] = useState('')
  const [role, setRole] = useState('developer')
  const m = useApiMutation({ invalidate: [['members', org]] })

  return (
    <>
      <div className="crumb"><Link to="/">Projects</Link> / <Link to="/projects/$org" params={{ org }}>{org}</Link> / Members</div>
      <div className="head"><h1>Members</h1></div>
      <div className="panel"><div className="panel-b">
        <QueryState isLoading={q.isLoading} error={q.error}>
          <div className="rows">
            {q.data?.length === 0 && <div className="empty">No members.</div>}
            {q.data?.map((mem) => (
              <div key={mem.user} className="row">
                <div><div className="nm">{mem.user}</div><div className="dm">{mem.role}</div></div>
                <div className="rt">
                  <button className="btn danger sm" disabled={m.isPending} onClick={() => {
                    if (confirm(`Remove ${mem.user}?`)) m.mutate(() => send(urls.members(org), { json: { user: mem.user, role: 'remove' } }))
                  }}>Remove</button>
                </div>
              </div>
            ))}
          </div>
        </QueryState>
        <div className="row2">
          <div className="field"><label>GitHub username</label><input type="text" value={user} onChange={(e) => setUser(e.target.value)} placeholder="octocat" /></div>
          <div className="field"><label>Role</label><select value={role} onChange={(e) => setRole(e.target.value)}><option>developer</option><option>admin</option></select></div>
        </div>
        <div className="actions">
          <button className="btn primary" disabled={m.isPending} onClick={() => {
            if (!user.trim()) return toast('username required', true)
            m.mutate(() => send(urls.members(org), { json: { user: user.trim(), role } }))
            setUser('')
          }}>Add / update member</button>
          <span className="h">Commits to project.yaml; teams + Tailscale ACLs sync automatically.</span>
        </div>
      </div></div>
    </>
  )
}
