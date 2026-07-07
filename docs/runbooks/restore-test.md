# Weekly restore test

Backups you haven't restored are hopes, not backups (§15: weekly restore test).

```sh
ssh majnet@<node>
source /etc/majnet/restic.env

restic snapshots --tag majnet | tail -5          # 1. snapshots exist + recent
restic check                                      # 2. repository integrity

# 3. restore into a scratch dir
restic restore latest --target /tmp/restore-test --include /var/backups/majnet
cd /tmp/restore-test/var/backups/majnet
```

Then load each engine's dump into a throwaway container (formats per
`bootstrap/steps/60-backups.sh`); only test the engines the node runs.

## postgres (`postgres.sql.gz`, pg_dumpall)

```sh
docker run -d --name restore-test -e POSTGRES_HOST_AUTH_METHOD=trust postgres:17
zcat postgres.sql.gz | docker exec -i restore-test psql -U postgres
docker exec restore-test psql -U postgres -lqt
docker exec restore-test psql -U postgres -d <project>_<app>_production -c 'select count(*) from <important_table>;'
docker rm -f restore-test
```

## mariadb (`mariadb.sql.gz`, mariadb-dump --all-databases)

```sh
docker run -d --name restore-test -e MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=1 mariadb:11
zcat mariadb.sql.gz | docker exec -i restore-test mariadb -uroot
docker exec restore-test mariadb -uroot -e 'show databases; select count(*) from <db>.<important_table>;'
docker rm -f restore-test
```

## mongodb (`mongodb.archive.gz`, mongodump --archive)

```sh
docker run -d --name restore-test mongo:8      # throwaway: no auth
zcat mongodb.archive.gz | docker exec -i restore-test mongorestore --archive
docker exec restore-test mongosh --quiet --eval 'db.getMongo().getDBNames()'
docker exec restore-test mongosh --quiet <db> --eval 'db.<important_collection>.countDocuments()'
docker rm -f restore-test
```

## valkey (`valkey.rdb.gz`, valkey-cli --rdb)

An RDB is loaded at startup from the data dir, not replayed through a client
— mount it as `dump.rdb` (append-only mode off, or the RDB is ignored):

```sh
gunzip -k valkey.rdb.gz && mv valkey.rdb dump.rdb
docker run -d --name restore-test -v "$PWD/dump.rdb:/data/dump.rdb" valkey/valkey:8
docker exec restore-test valkey-cli dbsize        # > 0
docker exec restore-test valkey-cli --scan | head
docker rm -f restore-test
```

## Wrap up

```sh
rm -rf /tmp/restore-test
```

Record date + snapshot ID + result. If step 1 shows stale snapshots, check `systemctl status majnet-backup.timer` first — the most common failure is the timer silently never firing after a rebuild.
