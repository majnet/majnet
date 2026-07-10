import { Link, Outlet } from '@tanstack/react-router'
import { useWhoami } from './api'

const IconProjects = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" /></svg>
)
const IconActivity = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M4 6h16M4 12h16M4 18h10" /></svg>
)
const IconNodes = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="3" y="4" width="18" height="6" rx="1" /><rect x="3" y="14" width="18" height="6" rx="1" /></svg>
)

export function Shell() {
  const { data: me } = useWhoami()
  const initials = (me?.login || 'infra').slice(0, 1).toUpperCase()
  return (
    <div className="app">
      <nav className="side">
        <div className="brand"><div className="logo" /><b>MajNet</b></div>
        <Link to="/" className="nav" activeProps={{ className: 'nav on' }} activeOptions={{ exact: true }}>{IconProjects}Projects</Link>
        <Link to="/activity" className="nav" activeProps={{ className: 'nav on' }}>{IconActivity}Activity</Link>
        <Link to="/nodes" className="nav" activeProps={{ className: 'nav on' }}>{IconNodes}Nodes</Link>
        <div className="spacer" />
        <div className="who">
          <span className="avatar">{initials}</span>
          <div>
            <div>{me?.login || 'infra'}</div>
            <div className="role">{me?.admin ? 'admin' : 'member'}</div>
          </div>
        </div>
      </nav>
      <main className="main"><Outlet /></main>
    </div>
  )
}
