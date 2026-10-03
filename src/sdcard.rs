//! Files on the printer's storage (SD card) over the same FTPS login as print uploads.

use serde_json::{Value, json};

/// Largest file streamed back through the server.
pub(crate) const MAX_DOWNLOAD: usize = 512 * 1024 * 1024;

/// A path the printer's FTPS server may be asked about: absolute, no `..`, plain characters.
pub(crate) fn path(input: &str) -> Result<String, &'static str> {
    let invalid = "Use an absolute storage path without '..' or control characters";
    let trimmed = if input.len() > 1 {
        input.trim_end_matches('/')
    } else {
        input
    };
    if !trimmed.starts_with('/')
        || trimmed.len() > 255
        || trimmed.contains("//")
        || trimmed.split('/').any(|part| part == "..")
        || trimmed.chars().any(|c| c.is_control() || c == '\\')
    {
        return Err(invalid);
    }
    Ok(trimmed.to_owned())
}

/// One `LIST` line as a file entry under `dir`, or `None` for lines that are not entries.
pub(crate) fn entry(line: &str, dir: &str) -> Option<Value> {
    if line.starts_with("total ") {
        return None;
    }
    let file: suppaftp::list::File = line.parse().ok()?;
    let name = file.name();
    if matches!(name, "." | "..") || name.contains('/') {
        return None;
    }
    let modified = file
        .modified()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs());
    Some(json!({
        "name": name,
        "path": format!("{}/{name}", dir.trim_end_matches('/')),
        "size": file.size(),
        "directory": file.is_directory(),
        "modified": modified,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_plain_and_absolute() {
        assert_eq!(path("/").unwrap(), "/");
        assert_eq!(path("/timelapse/").unwrap(), "/timelapse");
        assert_eq!(
            path("/cache/orca-1.gcode.3mf").unwrap(),
            "/cache/orca-1.gcode.3mf"
        );
        assert_eq!(
            path("/model/My part (2).3mf").unwrap(),
            "/model/My part (2).3mf"
        );
        for bad in [
            "",
            "timelapse",
            "/../etc",
            "/a/../b",
            "/a//b",
            "/a\nb",
            "/a\\b",
            &format!("/{}", "x".repeat(256)),
        ] {
            assert!(path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn list_lines_become_entries() {
        let file = entry(
            "-rw-r--r-- 1 root root 1234567 Oct 03 12:30 video_2026-10-03.mp4",
            "/timelapse",
        )
        .unwrap();
        assert_eq!(file["name"], "video_2026-10-03.mp4");
        assert_eq!(file["path"], "/timelapse/video_2026-10-03.mp4");
        assert_eq!(file["size"], 1_234_567);
        assert_eq!(file["directory"], false);
        assert!(file["modified"].is_u64());
        let dir = entry("drwxr-xr-x 2 root root 4096 Jan 01 2024 cache", "/").unwrap();
        assert_eq!(
            (&dir["path"], &dir["directory"]),
            (&json!("/cache"), &json!(true))
        );
        assert!(entry("total 12", "/").is_none());
        assert!(entry("drwxr-xr-x 2 root root 4096 Jan 01 2024 ..", "/").is_none());
    }
}
