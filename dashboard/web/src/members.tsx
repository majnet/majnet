import { useState } from 'react'
import { Link, useParams } from '@tanstack/react-router'
import { toast } from 'sonner'
import { send, urls, useMembers } from './api'
import { useApiMutation } from './mutations'
import { ConfirmButton, Empty, QueryState } from './ui'
import { Crumbs, PageHead } from './views'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

export function Members() {
  const { org } = useParams({ from: '/projects/$org/members' })
  const q = useMembers(org)
  const [user, setUser] = useState('')
  const [role, setRole] = useState('developer')
  const m = useApiMutation({ invalidate: [['members', org]] })

  return (
    <>
      <Crumbs><Link to="/projects">Projects</Link> / <Link to="/projects/$org" params={{ org }}>{org}</Link> / Members</Crumbs>
      <PageHead title="Members" />
      <Card><CardContent className="flex flex-col gap-4 pt-6">
        <QueryState isLoading={q.isLoading} error={q.error}>
          <div className="flex flex-col gap-2">
            {q.data?.length === 0 && <Empty>No members.</Empty>}
            {q.data?.map((mem) => (
              <div key={mem.user} className="flex items-center gap-3 rounded-lg border px-4 py-3">
                <div className="flex-1"><div className="font-semibold">{mem.user}</div><div className="font-mono text-xs text-muted-foreground">{mem.role}</div></div>
                <ConfirmButton variant="outline" size="sm" className="text-destructive" title={`Remove ${mem.user}?`}
                  confirmText="Remove" onConfirm={() => m.mutate(() => send(urls.members(org), { json: { user: mem.user, role: 'remove' } }))}>Remove</ConfirmButton>
              </div>
            ))}
          </div>
        </QueryState>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-1.5"><Label>GitHub username</Label><Input value={user} onChange={(e) => setUser(e.target.value)} placeholder="octocat" /></div>
          <div className="flex flex-col gap-1.5"><Label>Role</Label>
            <Select value={role} onValueChange={setRole}>
              <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem value="developer">developer</SelectItem><SelectItem value="admin">admin</SelectItem></SelectContent>
            </Select>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <Button disabled={m.isPending} onClick={() => {
            if (!user.trim()) return toast.error('username required')
            m.mutate(() => send(urls.members(org), { json: { user: user.trim(), role } }))
            setUser('')
          }}>Add / update member</Button>
          <span className="text-xs text-muted-foreground">Commits to project.yaml; teams + Tailscale ACLs sync automatically.</span>
        </div>
      </CardContent></Card>
    </>
  )
}
