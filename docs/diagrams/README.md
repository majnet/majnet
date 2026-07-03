# Diagrams

Sources in PlantUML (`.puml`) and Mermaid (`.mmd`); the Mermaid versions are also embedded in [design.md](../design.md).

| File | Shows |
|---|---|
| `architecture.puml` / `architecture.mmd` | Deployment view: root/project orgs, main/prod/private nodes, Cloudflare, Tailscale, backup flows |
| `cicd-sequence.puml` / `cicd-sequence.mmd` | Stable auto-deploy, gated production promotion, ephemeral PR previews |
| `lifecycles.puml` / `ephemeral-lifecycle.mmd` | Ephemeral env lifecycle + blue-green deploy state machine |

Render locally:

```sh
plantuml docs/diagrams/*.puml          # → PNG next to sources
mmdc -i docs/diagrams/architecture.mmd -o architecture.svg
```
