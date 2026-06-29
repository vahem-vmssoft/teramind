ALTER TABLE agents DROP CONSTRAINT agents_kind_version_key;
ALTER TABLE agents ADD CONSTRAINT agents_kind_version_key UNIQUE NULLS NOT DISTINCT (kind, version);
