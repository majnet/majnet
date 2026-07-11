import { useState } from 'react'
import { ServerCog } from 'lucide-react'
import { sendForm, urls, useNodes, useVersion, useWhoami, type PlatformNode } from './api'
import { useApiMutation } from './mutations'
import { PageHead } from './views'
import { QueryState, StatusBadge } from './ui'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'

const ZONE: Record<string, string> = { main: 'control plane', prod: 'public', private: 'internal' }
// Enrollable worker roles (main enrolls itself at install).
const WORKER_ROLES = ['prod', 'private'] as const

export function Settings() {
  const nodes = useNodes()
  const version = useVersion()
  const { data: me } = useWhoami()

  const enrolled = new Map((nodes.data ?? []).filter((n) => n.wireguard_pubkey).map((n) => [n.role, n]))
  const pending = WORKER_ROLES.filter((r) => !enrolled.has(r))

  return (
    <>
      <PageHead title="Settings" />

      <Card className="mb-4">
        <CardHeader><CardTitle className="text-base">Platform</CardTitle></CardHeader>
        <CardContent className="flex flex-col gap-2.5 text-sm">
          <Row k="Control-plane pin">{version.isLoading ? '…' : <code className="font-mono text-xs">{version.data?.slice(0, 12) ?? '—'}</code>}</Row>
          <Row k="Signed in as"><span className="inline-flex items-center gap-2">{me?.login ?? 'infra'} {me?.admin && <StatusBadge tone="success">admin</StatusBadge>}</span></Row>
          <Row k="Config source"><span className="text-muted-foreground">All platform config lives in <code className="font-mono text-xs">majksa-platform/platform</code> (git). Edits commit there; the reconciler converges.</span></Row>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle className="text-base">Nodes</CardTitle></CardHeader>
        <CardContent className="flex flex-col gap-3">
          <QueryState isLoading={nodes.isLoading} error={nodes.error}>
            <div className="flex flex-col gap-2">
              {(nodes.data ?? []).map((n) => <NodeRow key={n.role} n={n} />)}
            </div>
            {pending.length > 0 && <Separator className="my-1" />}
            {pending.map((role) => <Onboard key={role} role={role} />)}
            {pending.length === 0 && <p className="text-xs text-muted-foreground">All worker nodes are enrolled.</p>}
          </QueryState>
        </CardContent>
      </Card>
    </>
  )
}

function Row({ k, children }: { k: string; children: React.ReactNode }) {
  return <div className="flex gap-2.5"><span className="min-w-36 text-muted-foreground">{k}</span><span>{children}</span></div>
}

function NodeRow({ n }: { n: PlatformNode }) {
  const enrolled = !!n.wireguard_pubkey
  const ep = [n.wireguard_ip, n.public_endpoint].filter(Boolean).join(' · ')
  return (
    <div className={`flex items-center gap-3 rounded-lg border px-4 py-3 ${enrolled ? '' : 'opacity-60'}`}>
      <div className="flex-1"><div className="font-semibold">{n.name} <span className="text-xs font-normal text-muted-foreground">{ZONE[n.role] ?? n.role}</span></div>
        <div className="mt-0.5 font-mono text-xs text-muted-foreground">{ep || '—'}</div></div>
      {enrolled ? <StatusBadge tone="success" dot>enrolled</StatusBadge> : <StatusBadge tone="muted">pending</StatusBadge>}
    </div>
  )
}

function Onboard({ role }: { role: string }) {
  const [host, setHost] = useState('')
  const m = useApiMutation({ invalidate: [['nodes']], onDone: () => setHost('') })
  return (
    <div className="rounded-lg border border-dashed p-3.5">
      <div className="mb-2 flex items-center gap-2 text-sm font-medium"><ServerCog className="size-4" /> Onboard the <code className="font-mono">{role}</code> node</div>
      <p className="mb-3 text-xs text-muted-foreground">
        Provision a fresh Debian server, authorize the platform enrollment key on <code>root</code> (or the <code>majnet</code> user on a re-run),
        then enter its IP/host. The setup service SSHes in, runs bootstrap, brings up WireGuard, and registers it in <code>nodes.yaml</code>. Takes a few minutes.
      </p>
      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5"><Label className="text-xs">SSH host</Label>
          <Input value={host} onChange={(e) => setHost(e.target.value)} placeholder="203.0.113.9 or node.example.com" /></div>
        <Button disabled={m.isPending || !host.trim()} onClick={() => m.mutate(() => sendForm(urls.setupEnroll, { role, ssh_host: host.trim() }))}>
          {m.isPending ? 'Enrolling…' : 'Enroll'}
        </Button>
      </div>
    </div>
  )
}
