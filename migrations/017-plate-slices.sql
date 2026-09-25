CREATE TABLE plate_slices (
    plate_id TEXT PRIMARY KEY REFERENCES plates(id) ON DELETE CASCADE,
    plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
    input_key TEXT,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    project BLOB, gcode BLOB,
    checked_at INTEGER NOT NULL DEFAULT 0,
    generated_at INTEGER
);

-- An execution survives queue-row deletion. Cleanup retires its files explicitly.
CREATE TABLE print_executions (
    id TEXT PRIMARY KEY, job_id TEXT NOT NULL, printer_id TEXT NOT NULL,
    plate_id TEXT NOT NULL, name TEXT NOT NULL,
    ams_slot_id TEXT NOT NULL, filament_id TEXT NOT NULL REFERENCES filaments(id),
    required_machine_profile_key TEXT NOT NULL, process_profile_key TEXT NOT NULL, bed_type TEXT NOT NULL,
    artifact_path TEXT, execution_json TEXT, attempt_json TEXT, estimate_json TEXT
);
INSERT INTO print_executions
    SELECT attempt_id,id,printer_id,plate_id,name,ams_slot_id,filament_id,
           required_machine_profile_key,process_profile_key,bed_type,
           artifact_path,execution_json,attempt_json,estimate_json
    FROM print_jobs WHERE attempt_id IS NOT NULL;
CREATE INDEX execution_job ON print_executions(job_id);
CREATE TABLE waiting_jobs (
    id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id),
    plate_id TEXT NOT NULL REFERENCES plates(id),
    state TEXT NOT NULL CHECK(state IN ('queued','preparing','printing','awaiting_removal','completed','needs_attention','cancelled')),
    position INTEGER NOT NULL, attempt_id TEXT REFERENCES print_executions(id), last_error TEXT
);
INSERT INTO waiting_jobs SELECT id,printer_id,plate_id,state,position,attempt_id,last_error FROM print_jobs;
DROP TRIGGER referenced_material_setting_delete;
DROP TRIGGER referenced_material_setting_update;
DROP TABLE print_jobs;
ALTER TABLE waiting_jobs RENAME TO print_jobs;
CREATE UNIQUE INDEX one_active_job ON print_jobs(printer_id)
    WHERE state IN ('preparing','printing','awaiting_removal','needs_attention');
CREATE TRIGGER referenced_material_setting_delete BEFORE DELETE ON filament_settings
WHEN EXISTS(SELECT 1 FROM print_executions e JOIN filaments f ON f.id=e.filament_id
    WHERE f.product_id=OLD.product_id AND e.required_machine_profile_key=OLD.machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is referenced'); END;
CREATE TRIGGER referenced_material_setting_update BEFORE UPDATE OF product_id,machine_profile_key ON filament_settings
WHEN (NEW.product_id!=OLD.product_id OR NEW.machine_profile_key!=OLD.machine_profile_key)
    AND EXISTS(SELECT 1 FROM print_executions e JOIN filaments f ON f.id=e.filament_id
    WHERE f.product_id=OLD.product_id AND e.required_machine_profile_key=OLD.machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is referenced'); END;
PRAGMA user_version=17;
