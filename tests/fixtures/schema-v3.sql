-- OrcaServer v0.1.16 schema, before product/color separation.
CREATE TABLE printers (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL,
                    serial TEXT NOT NULL UNIQUE, access_code TEXT NOT NULL, tls_certificate TEXT NOT NULL,
                    machine_profile_key TEXT NOT NULL, default_process_profile_key TEXT NOT NULL,
                    bed_type TEXT NOT NULL, nozzle_material TEXT NOT NULL CHECK(nozzle_material IN ('stainless_steel','hardened_steel','unknown')),
                    mqtt_port INTEGER NOT NULL CHECK(mqtt_port BETWEEN 1 AND 65535),
                    ftps_port INTEGER NOT NULL CHECK(ftps_port BETWEEN 1 AND 65535),
                    start_timeout_secs INTEGER NOT NULL CHECK(start_timeout_secs BETWEEN 1 AND 3600)
                );
CREATE TABLE filaments (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, vendor TEXT NOT NULL,
                material TEXT NOT NULL, color TEXT NOT NULL, bambu_filament_id TEXT
            );
            CREATE TABLE filament_settings (
                id TEXT PRIMARY KEY, filament_id TEXT NOT NULL REFERENCES filaments(id) ON DELETE CASCADE,
                machine_profile_key TEXT NOT NULL, base_profile_key TEXT NOT NULL,
                overrides_json TEXT NOT NULL, UNIQUE(filament_id,machine_profile_key)
            );
            CREATE TABLE ams_slots (
                id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                ams_id INTEGER NOT NULL, slot_index INTEGER NOT NULL,
                filament_id TEXT REFERENCES filaments(id), mapping_source TEXT NOT NULL DEFAULT 'unassigned',
                reported_tag_uid TEXT, reported_profile_id TEXT, reported_type TEXT, reported_color TEXT,
                reported_brand TEXT, reported_temp_min INTEGER, reported_temp_max INTEGER,
                present INTEGER, remaining_percent INTEGER, detect_on_insert INTEGER, detect_on_power_up INTEGER,
                last_seen_at INTEGER, revision INTEGER NOT NULL DEFAULT 1,
                UNIQUE(printer_id,ams_id,slot_index),
                CHECK(ams_id BETWEEN 0 AND 255), CHECK(slot_index BETWEEN 0 AND 3),
                CHECK(mapping_source IN ('unassigned','manual','automatic'))
            );
            PRAGMA user_version=2;
ALTER TABLE printers ADD COLUMN queue_generation INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE printers ADD COLUMN queue_request TEXT;
                CREATE TABLE plates (id TEXT PRIMARY KEY, name TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1 CHECK(version>0));
                CREATE TABLE plate_items (
                    id TEXT PRIMARY KEY, plate_id TEXT NOT NULL REFERENCES plates(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL, name TEXT NOT NULL, source_kind TEXT NOT NULL CHECK(source_kind IN ('scad','upload')),
                    model_key TEXT, original BLOB, quantity INTEGER NOT NULL CHECK(quantity BETWEEN 1 AND 64),
                    UNIQUE(plate_id,position),
                    CHECK((source_kind='scad' AND model_key IS NOT NULL AND original IS NULL) OR
                          (source_kind='upload' AND model_key IS NULL AND original IS NOT NULL))
                );
                CREATE UNIQUE INDEX ams_printer_slot ON ams_slots(printer_id,id);
                CREATE TABLE print_jobs (
                    id TEXT PRIMARY KEY, printer_id TEXT NOT NULL REFERENCES printers(id),
                    plate_id TEXT NOT NULL REFERENCES plates(id), name TEXT NOT NULL,
                    ams_slot_id TEXT NOT NULL, filament_id TEXT NOT NULL REFERENCES filaments(id),
                    required_machine_profile_key TEXT NOT NULL, process_profile_key TEXT NOT NULL, bed_type TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('queued','preparing','printing','awaiting_removal','completed','needs_attention','cancelled')),
                    position INTEGER NOT NULL, attempt_id TEXT, artifact_path TEXT, execution_json TEXT, attempt_json TEXT, last_error TEXT,
                    FOREIGN KEY(printer_id,ams_slot_id) REFERENCES ams_slots(printer_id,id),
                    FOREIGN KEY(filament_id,required_machine_profile_key) REFERENCES filament_settings(filament_id,machine_profile_key)
                );
                CREATE UNIQUE INDEX one_active_job ON print_jobs(printer_id)
                    WHERE state IN ('preparing','printing','awaiting_removal','needs_attention');
PRAGMA user_version=3;
