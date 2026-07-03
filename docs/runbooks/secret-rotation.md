# Secret rotation

## An app secret (routine)
1. Edit `apps/<app>/secrets.<class>.yaml` in the project ops repo: `sops apps/api/secrets.stable.yaml` (your age key must be a recipient — project admins are, per `.sops.yaml`).
2. Commit to `main` → render PR → (auto-)merge → reconciler blue-greens the app with the new tmpfs files. Rotation is just a deploy.

## A person's age key (admin joined/left/lost key)
1. Update recipients in the project's `.sops.yaml`.
2. `sops updatekeys apps/*/secrets.*.yaml`, commit. **Removing** a recipient re-encrypts new versions only — an ex-admin can still read git history. If the values themselves must die, rotate them (see above), not just the recipients.

## A platform class key (`age-stable` / `age-production`)
1. `age-keygen -o age-<class>.key.new` on the main node.
2. Add the new public key as a recipient in **every** project's `.sops.yaml` + `sops updatekeys`, commit everywhere (the bot's org loop can help find projects: registry in `projects.yaml`).
3. Swap the file in `MAJNET_AGE_KEY_DIR`, restart the reconciler, verify a converge succeeds.
4. Remove the old recipient, `updatekeys` again.

## The DB master key
Don't, unless compromised — every derived password changes. If you must: replace `db-master.key`, then for each app with a `database:` the reconciler re-provisions the *user* password on next converge (ALTER ROLE/USER runs every cycle), and the new config hash redeploys apps with fresh `DATABASE_URL`s. Expect one blue-green wave across the fleet.
