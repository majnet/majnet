# App repo templates

Templates the bot uses to materialize app repos declared in a project's `project.yaml` (e.g. `template: rust-service`, `template: web-app`). Each template ships two GHA workflows for the DEV→OPS gradient (ADR 0009):

- **`build.yaml`** — the *build tier*: on `main`/PR, test → build → push image to GHCR by digest. Feeds `testing` (latest main) and `ephemeral` (PR previews). Disposable, continuous.
- **`release.yaml`** — the *release tier*: on tag `vX.Y.Z`, calls the reusable [`app-release.yaml`](../../.github/workflows/app-release.yaml) — builds + pushes `image:vX.Y.Z` to GHCR by digest. That tagged publish *is* the release: the bot records it off the `registry_package` webhook (version→digest) and auto-tracks `stable`; an operator promotes a chosen version to `production`. The migration lives in the ops overlay, not here.

Plus branch protection config for `main` and standard labels.

Each template also ships a **minimal runnable scaffold** — a Dockerfile and a tiny server — so a freshly materialized app is deployable and already serves the platform **standard endpoints** (design doc §16):

- **`/healthz`** — liveness. This is the platform's default `health.path`, so an app's manifest need only declare the `health.port`.
- **`/info`** — build metadata as JSON: `{version, commit, build_time}`. The reconciler scrapes this right after the health gate and shows it per env in the dashboard.

Build metadata flows from CI into the image: `build.yaml`/`release.yaml` pass `VERSION`/`GIT_COMMIT`/`BUILD_TIME` as Docker **build-args**; the Dockerfile bakes them into `ENV`; the server reads them at `/info`. The build tier reports `version: "dev"`; the release tier stamps the `vX.Y.Z` tag. Replace the scaffold's catch-all handler with your real app — keep `/healthz` + `/info`.

```
rust-service/   Cargo.toml · src/main.rs (axum) · Dockerfile · workflows
web-app/        package.json · server.js (node:http) · Dockerfile · workflows
```

These are *developed* here and *deployed* to `majksa-platform/platform/repo-templates/`, which is what the bot actually reads (design doc §10).
