-- In-flight rename freeze (data-preserving app/project rename). A row means a
-- rename of (project, old_app) → new_app in `class` is underway: convergence
-- and GC skip both names so the timer can't create an empty new stack or remove
-- the old one before the data migration (volume copy + DB rename) completes.
-- The row is deleted once the migration is done, unfreezing normal convergence.
CREATE TABLE IF NOT EXISTS renames (
    project TEXT NOT NULL,
    old_app TEXT NOT NULL,
    new_app TEXT NOT NULL,
    class   TEXT NOT NULL,
    PRIMARY KEY (project, old_app, class)
);
