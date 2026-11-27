//! The real binary of the service for the page tests, on a port the system chose and nothing
//! else held, run in a temporary directory of its own that also keeps its data and its log.
//! It serves the page; what the page asks of its API the test answers (tests/support/page.js).
use std::{
    fs::File,
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// How often a process that ended at once is started again, on another port.
pub const TRIES: usize = 5;

/// A port the system chose as free. It may be taken again by the time it is bound, so whoever
/// binds it starts again on another when it was.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub struct Server {
    child: Child,
    pub port: u16,
    /// Where it runs and logs to; it goes once the server has ended (a field is dropped after
    /// `drop`).
    _dir: tempfile::TempDir,
}

impl Server {
    /// Starts the binary, and returns once it says that its port is its own.
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        for _ in 0..TRIES {
            let port = free_port();
            let mut child = Command::new(env!("CARGO_BIN_EXE_orca-server"))
                .current_dir(dir.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap())
                .env("PORT", port.to_string())
                .env("PLATES_DIR", dir.path().join("plates"))
                // the official error table is never asked for
                .env("BAMBU_ERROR_TABLE_URL", "off")
                .stdout(File::create(&log).unwrap())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            // that the port answers is no sign, for another run may have got the same one
            let deadline = Instant::now() + Duration::from_secs(20);
            let ended = loop {
                let said = std::fs::read_to_string(&log).unwrap_or_default();
                if said.contains("server listening") {
                    break false;
                }
                if child.try_wait().unwrap().is_some() {
                    break true;
                }
                assert!(Instant::now() < deadline, "the server did not come up");
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            if !ended {
                return Self {
                    child,
                    port,
                    _dir: dir,
                };
            }
        }
        panic!("the server did not start");
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
