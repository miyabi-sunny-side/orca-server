//! Build the historical queue layout before exercising the older migration fixtures.
pub fn queue_v16(c: &rusqlite::Connection) {
    c.execute_batch(
        "PRAGMA foreign_keys=OFF;
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
