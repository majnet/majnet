import { useState, type ReactNode } from 'react'
import { Loader2 } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle, AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { cn } from '@/lib/utils'
import type { Event } from './api'

export const short = (img: string | null | undefined) =>
  String(img ?? '').replace(/(@sha256:[0-9a-f]{8})[0-9a-f]+/, '$1…')

/// A dependency-free sparkline: filled area + line, scaled to `max`. Themed via
/// the primary accent. Renders nothing meaningful until ≥2 points arrive.
export function Sparkline({ values, max = 100, h = 32 }: { values: number[]; max?: number; h?: number }) {
  const w = 100 // viewBox units; SVG stretches to container width
  if (values.length < 2) {
    return (
      <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className="text-muted-foreground/40 block">
        <line x1={0} y1={h - 1} x2={w} y2={h - 1} stroke="currentColor" strokeWidth={1} strokeDasharray="2 3" vectorEffect="non-scaling-stroke" />
      </svg>
    )
  }
  const n = values.length
  const y = (v: number) => h - 1 - (Math.min(Math.max(v, 0), max) / max) * (h - 2)
  const pts = values.map((v, i) => [(i / (n - 1)) * w, y(v)] as const)
  const line = pts.map(([x, yy], i) => `${i ? 'L' : 'M'}${x.toFixed(2)} ${yy.toFixed(2)}`).join(' ')
  const area = `${line} L${w} ${h} L0 ${h} Z`
  return (
    <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" className="block">
      <path d={area} className="fill-primary/15" />
      <path d={line} className="fill-none stroke-primary" strokeWidth={1.5} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

/// An interactive node metric chart (CPU / memory): filled area + line over the
/// rolling window, a recessive 100% limit line + a dashed threshold line, an
/// emphasized endpoint, and a hover crosshair + tooltip showing the value at any
/// point (percent + the absolute via `format`, and how long ago). The header
/// value tracks the hovered sample and goes warning-colored past `threshold`.
export function MetricChart({ label, values, format, threshold = 80, sampleSecs = 10 }: {
  label: string
  values: number[]
  /// Absolute readout for a given percent, e.g. `2.5 of 4 cores`.
  format: (pct: number) => string
  threshold?: number
  sampleSecs?: number
}) {
  const [hover, setHover] = useState<number | null>(null)
  const n = values.length
  const enough = n >= 2
  const idx = hover != null && hover < n ? hover : n - 1
  const pct = n ? (values[idx] ?? 0) : 0
  const W = 240
  const H = 64
  const pad = 3
  const x = (i: number) => (i / Math.max(1, n - 1)) * W
  const y = (v: number) => pad + (1 - Math.min(Math.max(v, 0), 100) / 100) * (H - pad * 2)
  const line = enough ? values.map((v, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join(' ') : ''
  const area = enough ? `${line} L${W} ${H} L0 ${H} Z` : ''
  const ago = (n - 1 - idx) * sampleSecs
  const rel = ago <= 0 ? 'now' : ago < 60 ? `${ago}s ago` : `${Math.round(ago / 60)}m ago`

  return (
    <div className="rounded-md border p-2.5">
      <div className="mb-1 flex items-baseline gap-2">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span className={cn('ml-auto font-mono text-[13px] font-semibold tabular-nums', pct >= threshold && 'text-warning')}>
          {n ? `${pct.toFixed(0)}%` : '—'}
        </span>
      </div>
      <div className="mb-1.5 font-mono text-[11px] text-muted-foreground">{n ? format(pct) : '—'}</div>
      <div className="relative">
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="block h-16 w-full"
          onMouseMove={enough ? (e) => {
            const r = e.currentTarget.getBoundingClientRect()
            const p = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width))
            setHover(Math.round(p * (n - 1)))
          } : undefined}
          onMouseLeave={() => setHover(null)}>
          <line x1={0} y1={y(100)} x2={W} y2={y(100)} className="stroke-border" strokeWidth={1} vectorEffect="non-scaling-stroke" />
          <line x1={0} y1={y(threshold)} x2={W} y2={y(threshold)} className="stroke-warning" strokeWidth={1} strokeDasharray="3 3" opacity={0.7} vectorEffect="non-scaling-stroke" />
          {enough ? (
            <>
              <path d={area} className="fill-primary/15" />
              <path d={line} className="fill-none stroke-primary" strokeWidth={1.75} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
              <circle cx={x(n - 1)} cy={y(values[n - 1] ?? 0)} r={2.5} className="fill-primary" />
              {hover != null && (
                <>
                  <line x1={x(idx)} y1={0} x2={x(idx)} y2={H} className="stroke-muted-foreground" strokeWidth={1} opacity={0.5} vectorEffect="non-scaling-stroke" />
                  <circle cx={x(idx)} cy={y(pct)} r={3} className="fill-primary stroke-card" strokeWidth={1.5} />
                </>
              )}
            </>
          ) : (
            <line x1={0} y1={H - 1} x2={W} y2={H - 1} className="stroke-muted-foreground/40" strokeWidth={1} strokeDasharray="2 3" vectorEffect="non-scaling-stroke" />
          )}
        </svg>
        {hover != null && enough && (
          <div className="pointer-events-none absolute -translate-x-1/2 -translate-y-full rounded-md bg-foreground px-1.5 py-1 text-center font-mono text-[11px] leading-tight text-background"
            style={{ left: `${(idx / (n - 1)) * 100}%`, top: `${(y(pct) / H) * 100 - 6}%` }}>
            {pct.toFixed(0)}% · {format(pct)}
            <div className="opacity-70">{rel}</div>
          </div>
        )}
      </div>
    </div>
  )
}

/// A hostname/URL rendered as a link that opens the site in a new tab. A bare
/// host (`app.example.com`) gets an `https://` scheme; anything already a URL is
/// used as-is. Shows a `↗` affordance.
export function ExtLink({ to, children, className }: { to: string; children?: ReactNode; className?: string }) {
  const href = /^https?:\/\//.test(to) ? to : `https://${to}`
  return (
    <a href={href} target="_blank" rel="noreferrer"
      className={cn('text-primary underline-offset-2 hover:underline', className)}>
      {children ?? to}<span aria-hidden className="ml-0.5 opacity-60">↗</span>
    </a>
  )
}

export const latestEventFor = (events: Event[] | undefined, project: string, app: string) =>
  (events ?? []).find((e) => e.project === project && e.action.trim().split(/\s+/).pop() === app)

// ── status badge ─────────────────────────────────────────────────────────────
const TONES = {
  success: 'border-transparent bg-success/15 text-success',
  warn: 'border-transparent bg-warning/15 text-warning',
  danger: 'border-transparent bg-destructive/15 text-destructive',
  muted: 'border-transparent bg-muted text-muted-foreground',
  accent: 'border-transparent bg-accent text-accent-foreground',
} as const

export function StatusBadge({ tone, dot, title, children }: {
  tone: keyof typeof TONES; dot?: boolean; title?: string; children: ReactNode
}) {
  return (
    <Badge variant="outline" title={title} className={cn('gap-1.5 font-medium', TONES[tone])}>
      {dot && <span className="size-1.5 rounded-full bg-current" />}
      {children}
    </Badge>
  )
}

export function DeployStatus({ ev }: { ev: Event | undefined }) {
  if (!ev) return <StatusBadge tone="muted">no deploys</StatusBadge>
  const r = ev.result || ''
  const act = ev.action.trim().split(/\s+/)[0] ?? ''
  const title = `${ev.action} → ${r}  ·  ${ev.at}  ·  ${ev.commit.slice(0, 12)}`
  if (r.startsWith('FAILED')) return <StatusBadge tone="danger" title={title}>failed</StatusBadge>
  if (act === 'gc') return <StatusBadge tone="muted" title={title}>removed</StatusBadge>
  if (r.startsWith('deployed')) return <StatusBadge tone="success" dot title={title}>deployed</StatusBadge>
  if (r === 'in sync') return <StatusBadge tone="success" dot title={title}>healthy</StatusBadge>
  return <StatusBadge tone="muted" title={title}>{(r || act).slice(0, 20)}</StatusBadge>
}

// ── query state ──────────────────────────────────────────────────────────────
export function QueryState({ isLoading, error, children }: {
  isLoading: boolean; error: unknown; children: ReactNode
}) {
  if (isLoading) return (
    <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
      <Loader2 className="size-4 animate-spin" /> Loading…
    </div>
  )
  if (error) return <div className="py-8 text-sm text-destructive">Failed to load: {String((error as Error).message)}</div>
  return <>{children}</>
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="py-8 text-sm text-muted-foreground">{children}</div>
}

// ── confirm dialog button ────────────────────────────────────────────────────
export function ConfirmButton({
  title, description, confirmText = 'Confirm', onConfirm, children, ...btn
}: React.ComponentProps<typeof Button> & {
  title: string; description?: string; confirmText?: string; onConfirm: () => void
}) {
  const [open, setOpen] = useState(false)
  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogTrigger asChild><Button {...btn}>{children}</Button></AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          {description && <AlertDialogDescription>{description}</AlertDialogDescription>}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm}>{confirmText}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
