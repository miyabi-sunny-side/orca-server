//! A real browser for the page tests: chrome-headless-shell of Chrome for Testing, driven
//! through the chromedriver of the same version over HTTP, by the commands of W3C
//! `WebDriver`, and by the commands of the Chrome `DevTools` Protocol chromedriver passes on
//! (`goog/cdp/execute`) for what `WebDriver` has no command for: the size and the theme of
//! the window, the wheel and the keys as a person works them, and scripts every document of
//! the page runs first.
//! `bin/browser` fetches both programs, of the version it pins, into the cache.
use serde_json::{Value, json};
use std::{
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::server::{TRIES, free_port};

/// How long [`Page::until`] waits for what it waits for.
const PATIENCE: Duration = Duration::from_secs(5);
/// How often it looks again.
const AGAIN: Duration = Duration::from_millis(10);

/// One browser with one page, for every test of a run, one test after another.
pub struct Page {
    driver: Child,
    /// The URL of the session, which the commands are sent under.
    session: String,
    http: reqwest::Client,
    /// Whether chromedriver and what it started have ended.
    ended: bool,
    /// The temporary directory chromedriver and the browser are started with (`TMPDIR`): the
    /// profile of the session lies in it, and goes with it once they have ended (a field is
    /// dropped after `drop`).
    _scratch: tempfile::TempDir,
}

/// Where `bin/browser` keeps the programs, which it fetches when they are not there yet.
fn fetched() -> PathBuf {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/bin/browser");
    let ran = Command::new("python3")
        .arg("-B")
        .arg(script)
        .stderr(Stdio::inherit())
        .output()
        .unwrap_or_else(|error| panic!("{script} could not be run: {error}"));
    assert!(
        ran.status.success(),
        "{script} did not fetch the browser ({})",
        ran.status
    );
    PathBuf::from(String::from_utf8(ran.stdout).unwrap().trim())
}

/// Whether a JSON value is one JavaScript takes for true.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(yes) => *yes,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

impl Page {
    /// Starts chromedriver, and a session with a browser that runs `scripts` first in every
    /// document it opens.
    pub async fn start(scripts: &[&str]) -> Self {
        let place = fetched();
        let scratch = tempfile::tempdir().unwrap();
        let http = reqwest::Client::new();
        let mut tries = 0..TRIES;
        // a port that was free may be taken by the time chromedriver binds it
        let (driver, base) = loop {
            assert!(tries.next().is_some(), "chromedriver did not start");
            let port = free_port();
            let mut driver = Command::new(place.join("chromedriver-linux64/chromedriver"))
                .arg(format!("--port={port}"))
                .env("TMPDIR", scratch.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                // its own group, so that what it started ends with it
                .process_group(0)
                .spawn()
                .unwrap();
            let base = format!("http://127.0.0.1:{port}");
            if Self::answers(&http, &base, &mut driver).await {
                break (driver, base);
            }
            let _ = driver.wait();
        };
        let mut page = Self {
            driver,
            session: String::new(),
            http,
            ended: false,
            _scratch: scratch,
        };
        let binary = place.join("chrome-headless-shell-linux64/chrome-headless-shell");
        let started = page
            .send(
                reqwest::Method::POST,
                &format!("{base}/session"),
                &json!({"capabilities": {"alwaysMatch": {
                    "browserName": "chrome",
                    "goog:chromeOptions": {
                        "binary": binary,
                        // without the scroll bars hidden a narrow window loses 15px to one
                        "args": ["--window-size=1280,800", "--hide-scrollbars"],
                    },
                }}}),
            )
            .await;
        let id = started["sessionId"].as_str().unwrap();
        page.session = format!("{base}/session/{id}");
        for script in scripts {
            page.cdp(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": script}),
            )
            .await;
        }
        page
    }

    /// Waits until chromedriver answers. `false` when it ended instead.
    async fn answers(http: &reqwest::Client, base: &str, driver: &mut Child) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(answer) = http.get(format!("{base}/status")).send().await {
                return answer.status().is_success();
            }
            if driver.try_wait().unwrap().is_some() {
                return false;
            }
            assert!(Instant::now() < deadline, "chromedriver did not answer");
            tokio::time::sleep(AGAIN).await;
        }
    }

    /// Ends the session and chromedriver, and waits until they are gone.
    pub async fn close(mut self) {
        let _ = self
            .http
            .delete(&self.session)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        self.end();
    }

    /// Ends chromedriver and what it started, and waits until it is gone.
    fn end(&mut self) {
        let group = format!("-{}", self.driver.id());
        let _ = Command::new("kill")
            .args(["-TERM", "--", &group])
            .stderr(Stdio::null())
            .status();
        let _ = self.driver.wait();
        // a browser that outlives chromedriver for a moment is in the same group
        let deadline = Instant::now() + Duration::from_secs(5);
        while Command::new("kill")
            .args(["-0", "--", &group])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
            && Instant::now() < deadline
        {
            std::thread::sleep(AGAIN);
        }
        let _ = Command::new("kill")
            .args(["-KILL", "--", &group])
            .stderr(Stdio::null())
            .status();
        self.ended = true;
    }

    /// Sends a command of `WebDriver` and answers with its value. A command that fails fails
    /// the test, with what the browser said.
    async fn send(&self, method: reqwest::Method, url: &str, body: &Value) -> Value {
        let mut request = self.http.request(method, url);
        if !body.is_null() {
            request = request.json(body);
        }
        let answer = request
            .send()
            .await
            .unwrap_or_else(|error| panic!("{url}: {error}"));
        let status = answer.status();
        let said: Value = answer.json().await.unwrap_or(Value::Null);
        assert!(status.is_success(), "{url}: {status} {said}");
        said["value"].clone()
    }

    /// Sends a command of the Chrome `DevTools` Protocol to the page.
    async fn cdp(&self, command: &str, params: Value) -> Value {
        let url = format!("{}/goog/cdp/execute", self.session);
        self.send(
            reqwest::Method::POST,
            &url,
            &json!({"cmd": command, "params": params}),
        )
        .await
    }

    /// Opens `url`, and returns once the browser has loaded it.
    pub async fn open(&self, url: &str) {
        let at = format!("{}/url", self.session);
        self.send(reqwest::Method::POST, &at, &json!({"url": url}))
            .await;
    }

    /// Runs `script` as the body of a function on the page, which is given `args` as
    /// `arguments`, and answers with what it returns (a promise is waited for).
    pub async fn js(&self, script: &str, args: Value) -> Value {
        let url = format!("{}/execute/sync", self.session);
        let args = match args {
            Value::Array(_) => args,
            Value::Null => json!([]),
            one => json!([one]),
        };
        let asked = json!({"script": script, "args": args});
        let answer = self
            .http
            .post(&url)
            .json(&asked)
            .send()
            .await
            .unwrap_or_else(|error| panic!("{url}: {error}"));
        let status = answer.status();
        let said: Value = answer.json().await.unwrap_or(Value::Null);
        assert!(
            status.is_success(),
            "the page could not run `{script}`: {}",
            said["value"]["message"]
                .as_str()
                .unwrap_or(&said.to_string())
        );
        said["value"].clone()
    }

    /// Runs `script` (as [`Page::js`] does) every 10ms until it returns what JavaScript takes
    /// for true, and answers with that. After 5 seconds the test fails, saying `what` was
    /// waited for and what came back last.
    pub async fn until(&self, what: &str, script: &str, args: Value) -> Value {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let value = self.js(script, args.clone()).await;
            if truthy(&value) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: not so after {PATIENCE:?} (last: {value})"
            );
            tokio::time::sleep(AGAIN).await;
        }
    }

    /// Has the page shown in `theme` ("dark" or "light", as the system would prefer it) in a
    /// window `width` by `height`, and waits until the page has taken the size.
    pub async fn look(&self, theme: &str, width: u32, height: u32) {
        self.cdp(
            "Emulation.setEmulatedMedia",
            json!({"features": [{"name": "prefers-color-scheme", "value": theme}]}),
        )
        .await;
        self.cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
        )
        .await;
        self.until(
            &format!("the window is {width} by {height} in the {theme}"),
            "const [width, height, theme] = arguments; \
             return innerWidth === width && innerHeight === height \
                 && matchMedia(`(prefers-color-scheme: ${theme})`).matches;",
            json!([width, height, theme]),
        )
        .await;
    }

    /// What the window shows inside `x`, `y`, `width`, `height`, as a PNG in base64: two
    /// shots of the same pixels are the same text.
    pub async fn shot(&self, x: f64, y: f64, width: f64, height: f64) -> String {
        let taken = self
            .cdp(
                "Page.captureScreenshot",
                json!({"format": "png", "clip": {"x": x, "y": y, "width": width, "height": height, "scale": 1}}),
            )
            .await;
        taken["data"].as_str().unwrap().to_owned()
    }

    /// The wheel of the mouse turned at `x`, `y` of the window, by `dy` pixels down.
    pub async fn wheel(&self, x: f64, y: f64, dy: f64) {
        self.cdp(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": dy}),
        )
        .await;
    }

    /// The mouse `pressed`, `moved` or `released` at `x`, `y` of the window, with its left
    /// button held down from `pressed` until `released`.
    pub async fn mouse(&self, kind: &str, x: f64, y: f64) {
        let (kind, clicks, held) = match kind {
            "pressed" => ("mousePressed", 1, 1),
            "moved" => ("mouseMoved", 0, 1),
            "hovered" => ("mouseMoved", 0, 0),
            "released" => ("mouseReleased", 1, 0),
            other => panic!("the mouse is not {other}"),
        };
        let button = if kind == "mouseMoved" && held == 0 {
            "none"
        } else {
            "left"
        };
        self.cdp(
            "Input.dispatchMouseEvent",
            json!({"type": kind, "x": x, "y": y, "button": button, "buttons": held, "clickCount": clicks}),
        )
        .await;
    }

    /// Has the page take a finger as a phone does (`true`), or the mouse again.
    pub async fn touches(&self, on: bool) {
        self.cdp(
            "Emulation.setTouchEmulationEnabled",
            json!({"enabled": on, "maxTouchPoints": 1}),
        )
        .await;
    }

    /// A finger put down (`touchStart`), moved (`touchMove`) or lifted (`touchEnd`) at `x`, `y`.
    pub async fn touch(&self, kind: &str, x: f64, y: f64) {
        let points = if kind == "touchEnd" {
            json!([])
        } else {
            json!([{"x": x, "y": y}])
        };
        self.cdp(
            "Input.dispatchTouchEvent",
            json!({"type": kind, "touchPoints": points}),
        )
        .await;
    }

    /// A key pressed and let go, as a person presses it: `Tab` or `ArrowRight`. The browser
    /// does what the key does there: it moves the focus, or scrolls what has it.
    pub async fn key(&self, key: &str) {
        let code = match key {
            "Tab" => 9,
            "Enter" => 13,
            "Escape" => 27,
            " " => 32,
            "ArrowRight" => 39,
            "ArrowDown" => 40,
            other => panic!("the key {other} is not known here"),
        };
        for kind in ["rawKeyDown", "keyUp"] {
            self.cdp(
                "Input.dispatchKeyEvent",
                json!({"type": kind, "key": key, "code": key, "windowsVirtualKeyCode": code}),
            )
            .await;
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        if !self.ended {
            self.end();
        }
    }
}
