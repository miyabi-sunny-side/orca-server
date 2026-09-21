CREATE TABLE print_jobs (
                    id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id),
                    plate_id TEXT NOT NULL REFERENCES plates(id), name TEXT NOT NULL,
                    ams_slot_id TEXT NOT NULL, filament_id TEXT NOT NULL REFERENCES filaments(id),
                    required_machine_profile_key TEXT NOT NULL, process_profile_key TEXT NOT NULL, bed_type TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('queued','preparing','printing','awaiting_removal','completed','needs_attention','cancelled')),
                    position INTEGER NOT NULL, attempt_id TEXT, artifact_path TEXT, execution_json TEXT, attempt_json TEXT, last_error TEXT,
                    FOREIGN KEY(printer_id,ams_slot_id) REFERENCES ams_slots(printer_id,id)
                );
                CREATE UNIQUE INDEX one_active_job ON print_jobs(printer_id)
                    WHERE state IN ('preparing','printing','awaiting_removal','needs_attention');
CREATE TRIGGER referenced_material_setting_delete BEFORE DELETE ON filament_settings
WHEN EXISTS(SELECT 1 FROM print_jobs j JOIN filaments f ON f.id=j.filament_id WHERE f.product_id=OLD.product_id AND j.required_machine_profile_key=OLD.machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is referenced'); END;
CREATE TRIGGER referenced_material_setting_update BEFORE UPDATE OF product_id,machine_profile_key ON filament_settings
WHEN (NEW.product_id!=OLD.product_id OR NEW.machine_profile_key!=OLD.machine_profile_key) AND EXISTS(SELECT 1 FROM print_jobs j JOIN filaments f ON f.id=j.filament_id WHERE f.product_id=OLD.product_id AND j.required_machine_profile_key=OLD.machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is referenced'); END;
CREATE TRIGGER job_material_setting_insert BEFORE INSERT ON print_jobs
WHEN NOT EXISTS(SELECT 1 FROM filaments f JOIN filament_settings s ON s.product_id=f.product_id WHERE f.id=NEW.filament_id AND s.machine_profile_key=NEW.required_machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is missing'); END;
CREATE TRIGGER job_material_setting_update BEFORE UPDATE OF filament_id,required_machine_profile_key ON print_jobs
WHEN NOT EXISTS(SELECT 1 FROM filaments f JOIN filament_settings s ON s.product_id=f.product_id WHERE f.id=NEW.filament_id AND s.machine_profile_key=NEW.required_machine_profile_key)
BEGIN SELECT RAISE(ABORT,'material setting is missing'); END;
