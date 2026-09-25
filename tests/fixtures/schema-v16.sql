CREATE TABLE printers (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL,
                    serial TEXT NOT NULL UNIQUE, access_code TEXT NOT NULL, tls_certificate TEXT NOT NULL,
                    machine_profile_key TEXT NOT NULL, default_process_profile_key TEXT NOT NULL,
                    bed_type TEXT NOT NULL, nozzle_material TEXT NOT NULL CHECK(nozzle_material IN ('stainless_steel','hardened_steel','unknown')),
                    mqtt_port INTEGER NOT NULL CHECK(mqtt_port BETWEEN 1 AND 65535),
                    ftps_port INTEGER NOT NULL CHECK(ftps_port BETWEEN 1 AND 65535),
                    start_timeout_secs INTEGER NOT NULL CHECK(start_timeout_secs BETWEEN 1 AND 3600)
                , queue_generation INTEGER NOT NULL DEFAULT 0, queue_request TEXT);
CREATE TABLE ams_slots (
                id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                ams_id INTEGER NOT NULL, slot_index INTEGER NOT NULL,
                filament_id TEXT REFERENCES filaments(id), mapping_source TEXT NOT NULL DEFAULT 'unassigned',
                reported_tag_uid TEXT, reported_profile_id TEXT, reported_type TEXT, reported_color TEXT,
                reported_brand TEXT, reported_temp_min INTEGER, reported_temp_max INTEGER,
                present INTEGER, remaining_percent INTEGER, detect_on_insert INTEGER, detect_on_power_up INTEGER,
                last_seen_at INTEGER, revision INTEGER NOT NULL DEFAULT 1, load_order INTEGER CHECK(load_order>0), priority_order INTEGER NOT NULL DEFAULT 0 CHECK(priority_order>=0),
                UNIQUE(printer_id,ams_id,slot_index),
                CHECK(ams_id BETWEEN 0 AND 255), CHECK(slot_index BETWEEN 0 AND 3),
                CHECK(mapping_source IN ('unassigned','manual','automatic'))
            );
CREATE TABLE plates (id TEXT PRIMARY KEY, name TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1 CHECK(version>0), required_machine_profile_key TEXT, filament_id TEXT REFERENCES filaments(id), process_profile_key TEXT, bed_type TEXT, sparse_infill_pattern TEXT, sparse_infill_density REAL CHECK(sparse_infill_density BETWEEN 0 AND 100), wall_loops INTEGER CHECK(wall_loops BETWEEN 0 AND 1000 AND wall_loops=CAST(wall_loops AS INTEGER)), deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)), brim_enabled INTEGER NOT NULL DEFAULT 0 CHECK(brim_enabled IN (0,1)), support_enabled INTEGER NOT NULL DEFAULT 0 CHECK(support_enabled IN (0,1)), support_interface_filament_id TEXT REFERENCES filaments(id), secondary_filament_id TEXT REFERENCES filaments(id));
CREATE TABLE plate_items (
                    id TEXT PRIMARY KEY, plate_id TEXT NOT NULL REFERENCES plates(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL, name TEXT NOT NULL, source_kind TEXT NOT NULL CHECK(source_kind IN ('scad','upload')),
                    model_key TEXT, original BLOB, quantity INTEGER NOT NULL CHECK(quantity BETWEEN 1 AND 64), roles_json TEXT NOT NULL DEFAULT '["primary"]' CHECK(json_valid(roles_json)),
                    UNIQUE(plate_id,position),
                    CHECK((source_kind='scad' AND model_key IS NOT NULL AND original IS NULL) OR
                          (source_kind='upload' AND model_key IS NULL AND original IS NOT NULL))
                );
CREATE TABLE filament_products (id TEXT PRIMARY KEY, name TEXT NOT NULL, vendor TEXT NOT NULL, material TEXT NOT NULL, bambu_filament_id TEXT);
CREATE TABLE "filaments" (id TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES filament_products(id) ON DELETE CASCADE, name TEXT NOT NULL, color TEXT NOT NULL);
CREATE TABLE "filament_settings" (id TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES filament_products(id) ON DELETE CASCADE, machine_profile_key TEXT NOT NULL, base_profile_key TEXT NOT NULL, overrides_json TEXT NOT NULL, UNIQUE(product_id,machine_profile_key));
CREATE TABLE print_jobs (
                    id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id),
                    plate_id TEXT NOT NULL REFERENCES plates(id), name TEXT NOT NULL,
                    ams_slot_id TEXT NOT NULL, filament_id TEXT NOT NULL REFERENCES filaments(id),
                    required_machine_profile_key TEXT NOT NULL, process_profile_key TEXT NOT NULL, bed_type TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('queued','preparing','printing','awaiting_removal','completed','needs_attention','cancelled')),
                    position INTEGER NOT NULL, attempt_id TEXT, artifact_path TEXT, execution_json TEXT, attempt_json TEXT, last_error TEXT, estimate_json TEXT CHECK(estimate_json IS NULL OR json_valid(estimate_json)),
                    FOREIGN KEY(printer_id,ams_slot_id) REFERENCES ams_slots(printer_id,id)
                );
CREATE TABLE default_settings (
                id INTEGER PRIMARY KEY CHECK(id=1),
                default_printer_id TEXT REFERENCES printers(id) ON DELETE SET NULL
            , sparse_infill_pattern TEXT NOT NULL DEFAULT 'adaptivecubic', sparse_infill_density REAL NOT NULL DEFAULT 15 CHECK(sparse_infill_density BETWEEN 0 AND 100), wall_loops INTEGER NOT NULL DEFAULT 2 CHECK(wall_loops BETWEEN 0 AND 1000 AND wall_loops=CAST(wall_loops AS INTEGER)));
CREATE TABLE print_notifications (
        attempt_id TEXT PRIMARY KEY, job_id TEXT NOT NULL, printer_id TEXT NOT NULL,
        printer_name TEXT NOT NULL, plate_name TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','unknown','sent','failed')),
        tries INTEGER NOT NULL DEFAULT 0 CHECK(tries BETWEEN 0 AND 3), next_at INTEGER NOT NULL DEFAULT 0,
        result TEXT, message_id TEXT
    );
CREATE TABLE plate_imports (
                plate_id TEXT PRIMARY KEY REFERENCES plates(id) ON DELETE CASCADE,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                original BLOB NOT NULL CHECK(length(original) BETWEEN 1 AND 67108864)
            );
CREATE TABLE print_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                attempt_id TEXT NOT NULL UNIQUE, job_id TEXT NOT NULL,
                plate_id TEXT NOT NULL, printer_id TEXT NOT NULL, name TEXT NOT NULL,
                completed_at INTEGER NOT NULL CHECK(completed_at >= 0)
            );
CREATE UNIQUE INDEX ams_printer_slot ON ams_slots(printer_id,id);
CREATE UNIQUE INDEX one_active_job ON print_jobs(printer_id)
                    WHERE state IN ('preparing','printing','awaiting_removal','needs_attention');
CREATE INDEX print_history_completion ON print_history(completed_at DESC,id DESC);
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
PRAGMA user_version=16;
