# DB break-glass access

Databases listen on per-project Docker networks only — **no VPN listener anywhere, by design** (§7). Emergency access is SSH + `docker exec`:

```sh
ssh majnet@<node>            # prod node for production DBs, private node for dev
docker exec -it majnet-postgres psql -U postgres
docker exec -it majnet-mariadb sh -c 'mariadb -uroot -p"$MARIADB_ROOT_PASSWORD"'
```

Per-app credentials are deterministic — regenerate a lost `DATABASE_URL` without touching anything:
db/user name is `<project>_<app>_<class>` (`-`→`_`), password = first 16 bytes (hex) of
`HMAC-SHA256(/etc/majnet/age/db-master.key, "Postgres:<project>:<app>:<class>")` — see `reconciler/src/db.rs`.

For a one-off query as the app's user, prefer exec-ing a client container on the project network over exposing any port:

```sh
docker run --rm -it --network proj-<project> postgres:17 psql "<DATABASE_URL>"
```

Log what you did in the incident notes — imperative DB access is invisible to the event log.
