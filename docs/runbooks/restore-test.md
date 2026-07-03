# Weekly restore test

Backups you haven't restored are hopes, not backups (§15: weekly restore test).

```sh
ssh majnet@<node>
source /etc/majnet/restic.env

restic snapshots --tag majnet | tail -5          # 1. snapshots exist + recent
restic check                                      # 2. repository integrity

# 3. restore into a scratch dir and load into a throwaway engine
restic restore latest --target /tmp/restore-test --include /var/backups/majnet
docker run -d --name restore-test -e POSTGRES_HOST_AUTH_METHOD=trust postgres:17
zcat /tmp/restore-test/var/backups/majnet/postgres.sql.gz | docker exec -i restore-test psql -U postgres

# 4. sanity: databases + a table count from a real app DB
docker exec restore-test psql -U postgres -lqt
docker exec restore-test psql -U postgres -d <project>_<app>_production -c 'select count(*) from <important_table>;'

docker rm -f restore-test && rm -rf /tmp/restore-test
```

Record date + snapshot ID + result. If step 1 shows stale snapshots, check `systemctl status majnet-backup.timer` first — the most common failure is the timer silently never firing after a rebuild.
