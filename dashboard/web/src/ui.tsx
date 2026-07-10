import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import type { Event } from './api'

// ── toast ────────────────────────────────────────────────────────────────────
type Toast = { msg: string; bad: boolean } | null
const ToastCtx = createContext<(msg: string, bad?: boolean) => void>(() => {})
export const useToast = () => useContext(ToastCtx)

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<Toast>(null)
  const timer = useRef<number>(0)
  const show = useCallback((msg: string, bad = false) => {
    setToast({ msg, bad })
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setToast(null), bad ? 9000 : 4500)
  }, [])
  useEffect(() => () => window.clearTimeout(timer.current), [])
  return (
    <ToastCtx.Provider value={show}>
      {children}
      {toast && (
        <div id="toast" className={toast.bad ? 'bad' : ''}>
          <pre>{toast.msg}</pre>
          <button className="btn ghost sm" style={{ marginLeft: 'auto' }} onClick={() => setToast(null)}>✕</button>
        </div>
      )}
    </ToastCtx.Provider>
  )
}

// ── pills + status ───────────────────────────────────────────────────────────
export type PillKind = 'ok' | 'dep' | 'err' | 'cls' | 'dim'
export function Pill({ kind, dot, title, children }: { kind: PillKind; dot?: boolean; title?: string; children: ReactNode }) {
  return (
    <span className={`pill ${kind}`} title={title}>
      {dot && <span className="dot" />}
      {children}
    </span>
  )
}

export const short = (img: string | null | undefined) =>
  String(img ?? '').replace(/(@sha256:[0-9a-f]{8})[0-9a-f]+/, '$1…')

export const latestEventFor = (events: Event[] | undefined, project: string, app: string) =>
  (events ?? []).find((e) => e.project === project && e.action.trim().split(/\s+/).pop() === app)

export function DeployStatus({ ev }: { ev: Event | undefined }) {
  if (!ev) return <Pill kind="dim">no deploys</Pill>
  const r = ev.result || ''
  const act = ev.action.trim().split(/\s+/)[0] ?? ''
  const title = `${ev.action} → ${r}  ·  ${ev.at}  ·  ${ev.commit.slice(0, 12)}`
  if (r.startsWith('FAILED')) return <Pill kind="err" title={title}>failed</Pill>
  if (act === 'gc') return <Pill kind="dim" title={title}>removed</Pill>
  if (r.startsWith('deployed')) return <Pill kind="ok" dot title={title}>deployed</Pill>
  if (r === 'in sync') return <Pill kind="ok" dot title={title}>healthy</Pill>
  return <Pill kind="dim" title={title}>{(r || act).slice(0, 20)}</Pill>
}

// ── query-state wrapper ──────────────────────────────────────────────────────
export function QueryState({
  isLoading, error, children,
}: { isLoading: boolean; error: unknown; children: ReactNode }) {
  if (isLoading) return <div className="spin">Loading…</div>
  if (error) return <div className="empty">Failed to load: {String((error as Error).message)}</div>
  return <>{children}</>
}
