# Bad deploy

## Boot/health failures — no action needed
The blue-green gate already handled it: the new container was removed, the old one is still serving, the event log shows `FAILED` with the causing commit (`GET /api/events` or dashboard).

Fix forward: correct the app/manifest, merge; stable redeploys automatically.

## Bad-but-healthy deploy (wrong behavior, passing health check)
1. **Identify the commit**: `git log env/stable` (or `env/production`) in the project's ops repo — each entry is exactly what ran, when.
2. **Rollback** = revert on `main`, never on env branches:
   - dashboard **rollback** button / `curl -X POST http://10.88.0.1:8081/api/rollback/<org>` — reverts the latest ops `main` commit; render PRs propagate it (production still needs the review-merge).
   - Older-than-latest: `git revert <sha>` on ops `main` by hand.
3. **Production**: merging the rollback render PR is the deploy; review the diff as usual.

## Stuck container (healthy per Docker, actually wedged)
Imperative escape hatch: dashboard **restart** or
`curl -X POST http://10.88.0.1:9090/api/restart/<project>/<class>/<app>` — same digest, audit-logged.

Never `docker stop/rm` majnet containers by hand — the reconciler will just recreate them; fix the state in git instead.
