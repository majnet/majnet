# App repo templates

Templates the bot uses to materialize app repos declared in a project's `project.yaml` (e.g. `template: rust-service`, `template: web-app`). Each template ships two GHA workflows for the DEV→OPS gradient (ADR 0009):

- **`build.yaml`** — the *build tier*: on `main`/PR, test → build → push image to GHCR by digest. Feeds `testing` (latest main) and `ephemeral` (PR previews). Disposable, continuous.
- **`release.yaml`** — the *release tier*: on tag `vX.Y.Z`, calls the reusable [`app-release.yaml`](../../.github/workflows/app-release.yaml) — builds + pushes `image:vX.Y.Z` to GHCR by digest. That tagged publish *is* the release: the bot records it off the `registry_package` webhook (version→digest) and auto-tracks `stable`; an operator promotes a chosen version to `production`. The migration lives in the ops overlay, not here.

Plus branch protection config for `main` and standard labels.

These are *developed* here and *deployed* to `majksa-platform/platform/repo-templates/`, which is what the bot actually reads (design doc §10).

Planned:

```
rust-service/
web-app/
```
