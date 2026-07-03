//! Convergence loop (§12) — per registered project:
//! ensure per-project Docker networks, project ingress (Traefik + tailscale
//! sidecar) and DB users on the assigned nodes, then for each rendered
//! manifest (app × class): validate → decrypt → container spec → diff vs
//! node's Docker state → migrations → blue-green converge → record event
//! {commit, project, node, action, result}.

// TODO(phase-2): implement single-app convergence to the private node first.
