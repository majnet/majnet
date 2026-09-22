-- Node filesystem usage in the metrics history (V7's `metric_samples`).
--
-- V7 recorded CPU, memory and container count. It did not record disk, and
-- neither did anything else: node `private` filled to 100%, took `stable` down
-- with it, and no MajNet surface could have shown the trend beforehand. The
-- charts these rows feed are the difference between noticing at 90% and finding
-- out at 100%.
--
-- Bytes, matching `mem_used`/`mem_total`. DEFAULT 0 backfills the rows written
-- before this migration; 0 reads as "not measured" everywhere downstream
-- (`disk_total = 0` is the same signal a probe timeout produces), never as an
-- empty disk. The pre-migration history therefore charts as a gap, not as a lie.
ALTER TABLE metric_samples ADD COLUMN disk_used  INTEGER NOT NULL DEFAULT 0;
ALTER TABLE metric_samples ADD COLUMN disk_total INTEGER NOT NULL DEFAULT 0;
