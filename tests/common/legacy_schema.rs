//! Build the historical queue layout before exercising the older migration fixtures.
pub fn queue_v16(c: &rusqlite::Connection) {
    plate_conditions_v23(c);
    c.execute_batch(
        "PRAGMA foreign_keys=OFF;
        ALTER TABLE printers DROP COLUMN recovery_attempt;
        DROP TRIGGER referenced_material_setting_delete;
        DROP TRIGGER referenced_material_setting_update;
        DROP INDEX one_active_job;
        ALTER TABLE print_jobs RENAME TO jobs_v17;",
    )
    .unwrap();
    c.execute_batch(include_str!("../../migrations/004-print-jobs.sql"))
        .unwrap();
    c.execute_batch("ALTER TABLE print_jobs ADD COLUMN estimate_json TEXT;
        INSERT INTO print_jobs(id,printer_id,plate_id,name,ams_slot_id,filament_id,required_machine_profile_key,process_profile_key,bed_type,state,position,attempt_id,artifact_path,execution_json,attempt_json,last_error,estimate_json)
        SELECT j.id,j.printer_id,j.plate_id,coalesce(e.name,p.name),
          coalesce(e.ams_slot_id,(SELECT id FROM ams_slots WHERE printer_id=j.printer_id AND filament_id=p.filament_id LIMIT 1),''),
          coalesce(e.filament_id,p.filament_id,''),coalesce(e.required_machine_profile_key,p.required_machine_profile_key,''),
          coalesce(e.process_profile_key,p.process_profile_key,''),coalesce(e.bed_type,p.bed_type,''),
          j.state,j.position,j.attempt_id,e.artifact_path,e.execution_json,e.attempt_json,j.last_error,e.estimate_json
        FROM jobs_v17 j JOIN plates p ON p.id=j.plate_id LEFT JOIN print_executions e ON e.id=j.attempt_id;
        DROP TABLE jobs_v17; DROP TABLE print_executions; DROP TABLE plate_slices;
        PRAGMA user_version=16; PRAGMA foreign_keys=ON;").unwrap();
}

/// Return a current database to schema 23: plates own the machine, process and bed, and
/// one slice result per plate. Plates with a material get the first printer's conditions,
/// as plate creation copied them from the printer, and keep that printer's result.
pub fn plate_conditions_v23(c: &rusqlite::Connection) {
    c.execute_batch(
        "ALTER TABLE plates ADD COLUMN required_machine_profile_key TEXT;
        ALTER TABLE plates ADD COLUMN process_profile_key TEXT;
        ALTER TABLE plates ADD COLUMN bed_type TEXT;
        UPDATE plates SET (required_machine_profile_key,process_profile_key,bed_type)=
            (SELECT machine_profile_key,default_process_profile_key,bed_type FROM printers ORDER BY id LIMIT 1)
            WHERE filament_id IS NOT NULL;
        CREATE TABLE plate_slices_v23 (
            plate_id TEXT PRIMARY KEY REFERENCES plates(id) ON DELETE CASCADE,
            plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
            input_key TEXT,
            record_json TEXT NOT NULL CHECK(json_valid(record_json)),
            project BLOB, gcode BLOB,
            checked_at INTEGER NOT NULL DEFAULT 0,
            generated_at INTEGER
        );
        INSERT INTO plate_slices_v23 SELECT plate_id,plan_json,input_key,record_json,project,gcode,checked_at,generated_at
            FROM plate_slices WHERE printer_id=(SELECT id FROM printers ORDER BY id LIMIT 1);
        DROP TABLE plate_slices;
        ALTER TABLE plate_slices_v23 RENAME TO plate_slices;
        PRAGMA user_version=23;",
    )
    .unwrap();
}
