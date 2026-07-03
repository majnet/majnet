# App repo templates

Templates the bot uses to materialize app repos declared in a project's `project.yaml` (e.g. `template: rust-service`, `template: web-app`). Each template ships:

- a GHA workflow: test → build → push image to GHCR (org-scoped, by digest) → webhook the bot `(org, app, digest)`
- branch protection config for `main`
- standard labels

These are *developed* here and *deployed* to `majksa-platform/platform/repo-templates/`, which is what the bot actually reads (design doc §10).

Planned:

```
rust-service/
web-app/
```
