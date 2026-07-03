//! Manifest rendering (§9, §11.5).
//!
//! On every `ops` `main` push: merge `base.yaml` ⊕ class overlay per app,
//! validate strictly, open/update a render PR per affected `env/<class>`
//! branch containing the final manifests. Secrets pass through encrypted —
//! rendering never decrypts.
//!
//! Merge policy: `stable`/`ephemeral` render PRs auto-merge; `env/production`
//! waits for admin review — that review *is* the production gate.

// TODO(phase-2): schema v1 rendering + validation.

