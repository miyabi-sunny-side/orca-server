-- The printer owns the machine, process and bed; a result belongs to one plate and printer.
-- A result moves to every printer whose conditions produced it; others are recalculated.
CREATE TABLE printer_slices (
    plate_id TEXT NOT NULL REFERENCES plates(id) ON DELETE CASCADE,
    printer_id TEXT NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
    plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
    input_key TEXT,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    project BLOB, gcode BLOB,
    checked_at INTEGER NOT NULL DEFAULT 0,
    generated_at INTEGER,
    PRIMARY KEY(plate_id,printer_id)
);
INSERT INTO printer_slices
    SELECT s.plate_id,r.id,s.plan_json,s.input_key,s.record_json,s.project,s.gcode,s.checked_at,s.generated_at
    FROM plate_slices s JOIN plates p ON p.id=s.plate_id
    JOIN printers r ON r.machine_profile_key=p.required_machine_profile_key
        AND r.default_process_profile_key=p.process_profile_key AND r.bed_type=p.bed_type;
DROP TABLE plate_slices;
ALTER TABLE printer_slices RENAME TO plate_slices;
CREATE INDEX plate_slices_printer ON plate_slices(printer_id);
ALTER TABLE plates DROP COLUMN required_machine_profile_key;
ALTER TABLE plates DROP COLUMN process_profile_key;
ALTER TABLE plates DROP COLUMN bed_type;
PRAGMA user_version=24;
