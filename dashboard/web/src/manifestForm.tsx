import { type ReactNode } from 'react'

// The manifest draft: optional sections carry an `on` flag so class overlays
// stay sparse (only enabled/non-empty fields are emitted).
export interface ManifestDraft {
  image: string
  ingress: { on: boolean; host: string; port: string; domains: string[] }
  health: { on: boolean; path: string; port: string; retries: string }
  database: { on: boolean; engine: string }
  env: [string, string][]
  secrets: string[]
  migration: { on: boolean; command: string[] }
}

type Rec = Record<string, unknown>
const asRec = (v: unknown): Rec => (v && typeof v === 'object' ? (v as Rec) : {})
const str = (v: unknown, d = '') => (v == null ? d : String(v))

export function fromData(data: unknown): ManifestDraft {
  const d = asRec(data)
  const ing = asRec(d.ingress)
  const hl = asRec(d.health)
  const db = asRec(d.database)
  const mig = asRec(d.migration)
  const env = asRec(d.env)
  return {
    image: str(d.image),
    ingress: { on: !!d.ingress, host: str(ing.host), port: str(ing.port), domains: Array.isArray(ing.domains) ? ing.domains.map(String) : [] },
    health: { on: !!d.health, path: str(hl.path, '/'), port: str(hl.port), retries: str(hl.retries, '5') },
    database: { on: !!d.database, engine: str(db.engine, 'postgres') },
    env: Object.entries(env).map(([k, v]) => [k, String(v)] as [string, string]),
    secrets: Array.isArray(d.secrets) ? d.secrets.map(String) : [],
    migration: { on: !!d.migration, command: Array.isArray(mig.command) ? mig.command.map(String) : [] },
  }
}

export function toManifest(d: ManifestDraft, file: string, app: string): Rec {
  const out: Rec = {}
  if (file === 'base.yaml') out.name = app // identity = directory
  if (d.image.trim()) out.image = d.image.trim()
  if (d.ingress.on) {
    const ing: Rec = { host: d.ingress.host.trim(), port: Number(d.ingress.port) }
    const domains = d.ingress.domains.map((s) => s.trim()).filter(Boolean)
    if (domains.length) ing.domains = domains
    out.ingress = ing
  }
  if (d.health.on) out.health = { path: d.health.path.trim(), port: Number(d.health.port), retries: Number(d.health.retries) }
  if (d.database.on) out.database = { engine: d.database.engine }
  const env = Object.fromEntries(d.env.map(([k, v]) => [k.trim(), v]).filter(([k]) => k))
  if (Object.keys(env).length) out.env = env
  const secrets = d.secrets.map((s) => s.trim()).filter(Boolean)
  if (secrets.length) out.secrets = secrets
  if (d.migration.on) {
    const command = d.migration.command.map((s) => s.trim()).filter(Boolean)
    if (command.length) out.migration = { command }
  }
  return out
}

// ── small editors ────────────────────────────────────────────────────────────
function Section({ label, on, onToggle, children }: { label: string; on: boolean; onToggle: (v: boolean) => void; children: ReactNode }) {
  return (
    <div className="field" style={{ border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', padding: 12, gap: 10 }}>
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontWeight: 560 }}>
        <input type="checkbox" checked={on} onChange={(e) => onToggle(e.target.checked)} style={{ width: 'auto' }} /> {label}
      </label>
      {on && <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>{children}</div>}
    </div>
  )
}

function ListEditor({ values, onChange, placeholder }: { values: string[]; onChange: (v: string[]) => void; placeholder?: string }) {
  const set = (i: number, v: string) => onChange(values.map((x, j) => (j === i ? v : x)))
  return (
    <div className="rows">
      {values.map((v, i) => (
        <div key={i} style={{ display: 'flex', gap: 6 }}>
          <input type="text" value={v} placeholder={placeholder} onChange={(e) => set(i, e.target.value)} />
          <button type="button" className="btn ghost sm" onClick={() => onChange(values.filter((_, j) => j !== i))}>✕</button>
        </div>
      ))}
      <button type="button" className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => onChange([...values, ''])}>+ add</button>
    </div>
  )
}

function KvEditor({ pairs, onChange }: { pairs: [string, string][]; onChange: (v: [string, string][]) => void }) {
  const set = (i: number, k: string, val: string) => onChange(pairs.map((p, j) => (j === i ? [k, val] : p)))
  return (
    <div className="rows">
      {pairs.map(([k, v], i) => (
        <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 6 }}>
          <input type="text" value={k} placeholder="KEY" onChange={(e) => set(i, e.target.value, v)} />
          <input type="text" value={v} placeholder="value" onChange={(e) => set(i, k, e.target.value)} />
          <button type="button" className="btn ghost sm" onClick={() => onChange(pairs.filter((_, j) => j !== i))}>✕</button>
        </div>
      ))}
      <button type="button" className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => onChange([...pairs, ['', '']])}>+ add</button>
    </div>
  )
}

const ENGINES = ['postgres', 'mariadb', 'valkey', 'mongodb']

/** The structured manifest form for one file; lifts its draft to the parent via onChange. */
export function ManifestForm({ file, draft, onChange }: { file: string; draft: ManifestDraft; onChange: (d: ManifestDraft) => void }) {
  const set = <K extends keyof ManifestDraft>(k: K, v: ManifestDraft[K]) => onChange({ ...draft, [k]: v })
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div className="field"><label>Image</label>
        <input type="text" value={draft.image} placeholder="ghcr.io/org/app@sha256:…" onChange={(e) => set('image', e.target.value)} />
        <span className="h">{file === 'base.yaml' ? 'Digest-pinned (required in base.yaml); tags are rejected.' : 'Optional per-class image override (production digests come from Promote).'}</span>
      </div>

      <Section label="Ingress" on={draft.ingress.on} onToggle={(on) => set('ingress', { ...draft.ingress, on })}>
        <div className="row2">
          <div className="field"><label>Host</label><input type="text" value={draft.ingress.host} onChange={(e) => set('ingress', { ...draft.ingress, host: e.target.value })} /></div>
          <div className="field"><label>Container port</label><input type="number" value={draft.ingress.port} onChange={(e) => set('ingress', { ...draft.ingress, port: e.target.value })} /></div>
        </div>
        <div className="field"><label>Additional domains</label>
          <ListEditor values={draft.ingress.domains} placeholder="www.example.com" onChange={(domains) => set('ingress', { ...draft.ingress, domains })} /></div>
      </Section>

      <Section label="Health check" on={draft.health.on} onToggle={(on) => set('health', { ...draft.health, on })}>
        <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr 1fr', gap: 10 }}>
          <div className="field"><label>Path</label><input type="text" value={draft.health.path} onChange={(e) => set('health', { ...draft.health, path: e.target.value })} /></div>
          <div className="field"><label>Port</label><input type="number" value={draft.health.port} onChange={(e) => set('health', { ...draft.health, port: e.target.value })} /></div>
          <div className="field"><label>Retries</label><input type="number" value={draft.health.retries} onChange={(e) => set('health', { ...draft.health, retries: e.target.value })} /></div>
        </div>
      </Section>

      <Section label="Database" on={draft.database.on} onToggle={(on) => set('database', { ...draft.database, on })}>
        <div className="field"><label>Engine</label>
          <select value={draft.database.engine} onChange={(e) => set('database', { ...draft.database, engine: e.target.value })}>
            {ENGINES.map((en) => <option key={en}>{en}</option>)}
          </select></div>
      </Section>

      <div className="field"><label>Environment variables</label>
        <KvEditor pairs={draft.env} onChange={(env) => set('env', env)} />
        <span className="h">Plain (non-secret) values.</span></div>

      <div className="field"><label>Secrets</label>
        <ListEditor values={draft.secrets} placeholder="SECRET_NAME" onChange={(secrets) => set('secrets', secrets)} />
        <span className="h">Names of secret env vars; the values live SOPS-encrypted and are edited outside the UI.</span></div>

      <Section label="Migration" on={draft.migration.on} onToggle={(on) => set('migration', { ...draft.migration, on })}>
        <div className="field"><label>Command</label>
          <ListEditor values={draft.migration.command} placeholder="arg" onChange={(command) => set('migration', { ...draft.migration, command })} /></div>
      </Section>
    </div>
  )
}
