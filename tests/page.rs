//! The page of the real binary in a real browser, for what only a browser's engine shows:
//! where things lie and how big they are, what overflows, the colours, scrolling, the mouse
//! and the finger as a person works them, and the shapes WebGL draws. The rest of the page is
//! tested in jsdom (client/src/**/*.test.ts).
//!
//! One browser and one server: the tests run one after another, in the order they are
//! written, each on a page of no state in a window of 1280 by 720 in the light. What the page
//! asks of the API the test answers (tests/support/page.js). `cargo test --test page --
//! <words>` runs the tests whose titles hold any of the words; `--skip <words>` leaves those
//! out.
#![allow(clippy::unwrap_used)] // helpers outside #[test] fns
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use browser::Page;
use server::Server;

#[path = "support/browser.rs"]
mod browser;
#[path = "support/server.rs"]
mod server;

type Run = Pin<Box<dyn Future<Output = ()> + Send>>;
/// A test: its title, and what it does on the page of the server.
type Test = (&'static str, fn(Arc<Page>, Arc<Server>) -> Run);

macro_rules! test {
    ($title:literal, $test:ident) => {
        ($title, |page, server| Box::pin($test(page, server)))
    };
}

/// The tests, in the order they run.
const TESTS: &[Test] = &[
    test!(
        "the plate list fits each window, with its colours, and 200% text",
        plate_list_fits
    ),
    test!("the license page fits 320px in both themes", license_fits),
    test!(
        "stopped recovery keeps a 44px retry in sight with 200% text",
        recovery_in_sight
    ),
    test!(
        "a saved failure's long reason stays inside the window",
        failure_reason_fits
    ),
    test!(
        "an attempt that never started says why next to the retry, in sight",
        not_started_in_sight
    ),
    test!(
        "a paused printer's error and its help stay in sight with 200% text",
        device_error_in_sight
    ),
    test!(
        "queue rows keep two lines, six in sight on a phone, and 200% text",
        queue_density
    ),
    test!(
        "pointer insertion survives polling; changed generation cancels a stale drag",
        pointer_drag
    ),
    test!(
        "touch scroll starts on the card, drag is confined to the handle",
        touch_drag
    ),
    test!("the plate detail fits a 375px window", detail_fits),
    test!(
        "the plate menu has buttons of one width, a readable delete and an outline",
        plate_menu_looks
    ),
    test!("a pending deletion dims its button", pending_deletion_dims),
    test!(
        "long names and 200% text keep menu and confirmation within the viewport",
        long_names_fit
    ),
    test!(
        "the history and its menu fit each window with 200% text",
        history_fits
    ),
    test!(
        "long press opens the shared menu while vertical touch movement scrolls",
        history_touch
    ),
    test!(
        "adding from deep history preserves the row position and shows the result",
        history_keeps_position
    ),
    test!(
        "the queue menu's three buttons are of one width",
        queue_menu_widths
    ),
    test!("the plate's estimates fit a 375px window", estimates_fit),
];

/// What every document of the page runs first: the stand-in for the API of the server.
const PAGE_JS: &str = include_str!("support/page.js");
/// The queue the jsdom tests use as well: one printer printing and eight jobs waiting.
const QUEUE: &str = include_str!("../client/src/testing/queue.json");

const LOOKS: [&str; 2] = ["dark", "light"];
/// The background of the page in each theme.
fn background(theme: &str) -> &'static str {
    if theme == "dark" {
        "rgb(25, 25, 25)"
    } else {
        "rgb(250, 246, 239)"
    }
}

// ---- helpers ----

/// Has the API answer with `routes` from now on (tests/support/page.js).
async fn lay(page: &Page, routes: &Value) {
    page.js(
        "sessionStorage.setItem('test:api', JSON.stringify(arguments[0]));",
        json!([routes]),
    )
    .await;
}

/// The requests the page sent, as tests/support/page.js wrote them down.
async fn sent(page: &Page) -> Vec<Value> {
    page.js(
        "return JSON.parse(sessionStorage.getItem('test:sent') ?? '[]');",
        Value::Null,
    )
    .await
    .as_array()
    .unwrap()
    .clone()
}

/// The requests sent with `method` to a path that begins with `path`.
async fn sent_to(page: &Page, method: &str, path: &str) -> Vec<Value> {
    sent(page)
        .await
        .into_iter()
        .filter(|r| r["method"] == method && r["path"].as_str().unwrap().starts_with(path))
        .collect()
}

/// Lets the answers held under `name` go.
async fn release(page: &Page, name: &str) {
    page.js(
        "const done = JSON.parse(sessionStorage.getItem('test:released') ?? '[]'); \
         sessionStorage.setItem('test:released', JSON.stringify([...done, arguments[0]]));",
        json!(name),
    )
    .await;
}

/// Waits until the page holds what `selector` finds.
async fn shows(page: &Page, selector: &str) {
    page.until(
        &format!("{selector} is on the page"),
        "return document.querySelector(arguments[0]) !== null;",
        json!(selector),
    )
    .await;
}

/// The page is no wider than the window.
async fn no_overflow(page: &Page, look: &str) {
    let widths = page
        .js(
            "return [document.documentElement.scrollWidth, innerWidth];",
            Value::Null,
        )
        .await;
    assert!(
        widths[0].as_f64() <= widths[1].as_f64(),
        "{look}: the page is wider than the window: {widths}"
    );
}

/// Doubles the text sizes, the fixed ones of the tokens included, in the same window.
async fn bigger_text(page: &Page) {
    page.js(
        "const style = document.createElement('style'); \
         style.textContent = ':root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }'; \
         document.head.append(style);",
        Value::Null,
    )
    .await;
}

/// Finds the element `find` (a script that returns it) after scrolling it into view, and
/// waits until some of it is inside the window (as Playwright's `toBeInViewport`).
async fn in_sight(page: &Page, what: &str, find: &str) {
    page.until(
        &format!("{what} is in sight"),
        &format!(
            "const el = (() => {{ {find} }})(); if (!el) return false; \
             el.scrollIntoView({{block: 'nearest', inline: 'nearest'}}); \
             const r = el.getBoundingClientRect(); \
             return r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0 \
                 && r.top < innerHeight && r.left < innerWidth;"
        ),
        Value::Null,
    )
    .await;
}

/// A script that finds the button named `name`, when there is one, in `within` (a selector).
fn button(within: &str, name: &str) -> String {
    format!(
        "return [...document.querySelectorAll('{within} button')].find((b) => \
         (b.getAttribute('aria-label') ?? b.textContent).trim() === '{name}');"
    )
}

/// The middle of what `find` returns, in the window.
async fn middle(page: &Page, find: &str) -> (f64, f64) {
    let at = page
        .js(
            &format!(
                "const r = (() => {{ {find} }})().getBoundingClientRect(); \
                 return [r.x + r.width / 2, r.y + r.height / 2];"
            ),
            Value::Null,
        )
        .await;
    (at[0].as_f64().unwrap(), at[1].as_f64().unwrap())
}

/// The queue of queue.json with `change` applied to it, and the routes that answer with it.
fn queue(change: impl FnOnce(&mut Value)) -> Value {
    let mut fixture: Value = serde_json::from_str(QUEUE).unwrap();
    change(&mut fixture["queue"]);
    json!({
        "/api/printers": {"json": fixture["printers"]},
        "/api/filaments": {"json": fixture["filaments"]},
        "/api/printers/p1/ams": {"json": fixture["ams"]},
        "/api/queue": {"json": fixture["queue"]},
    })
}

/// Merges the fields of `with` into `into`.
fn set(into: &mut Value, with: &Value) {
    for (key, value) in with.as_object().unwrap() {
        into[key] = value.clone();
    }
}

// ---- the plate list ----

fn plates(n: usize, name: impl Fn(usize) -> String) -> Value {
    (0..n)
        .map(|i| {
            json!({
                "id": format!("10000000-0000-4000-8000-{i:012}"),
                "version": 1,
                "name": name(i),
                "models": [{"id": "model", "name": "box.stl", "source": "box.stl", "quantity": 1}],
                "conditions": {"filament_id": "white"},
            })
        })
        .collect()
}

async fn plate_list_fits(page: Arc<Page>, server: Arc<Server>) {
    let list = plates(100, |i| {
        if i % 3 == 0 {
            format!("机の配線整理・ケーブルホルダー {}", i + 1)
        } else {
            format!("小物ケース {}", i + 1)
        }
    });
    for width in [320, 375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &json!({"/api/plates": {"json": list}})).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/plates")).await;
            page.until(
                &format!("{look}: 100 rows"),
                "return document.querySelectorAll('.plate-row').length === 100;",
                Value::Null,
            )
            .await;
            no_overflow(&page, &look).await;
            let at = page
                .js(
                    "const rows = [...document.querySelectorAll('.plate-row')]; \
                     return { first: rows[0].getBoundingClientRect().y, \
                       visible: rows.filter((r) => r.getBoundingClientRect().bottom <= innerHeight).length, \
                       overflow: getComputedStyle(document.querySelector('.plate-list')).overflowY, \
                       background: getComputedStyle(document.body).backgroundColor, \
                       menu: document.querySelector('header button[aria-label=\"メニュー\"]').getBoundingClientRect().width };",
                    Value::Null,
                )
                .await;
            assert!(at["first"].as_f64().unwrap() <= 210.0, "{look}: {at}");
            if width == 375 {
                assert!(at["visible"].as_u64().unwrap() >= 6, "{look}: {at}");
            }
            assert_eq!(at["overflow"], "visible", "{look}: {at}");
            assert_eq!(at["background"], background(theme), "{look}: {at}");
            assert_eq!(at["menu"], 36, "{look}: {at}");
            bigger_text(&page).await;
            no_overflow(&page, &format!("{look} 200%")).await;
            in_sight(
                &page,
                "新規作成",
                "return [...document.querySelectorAll('a')].find((a) => a.textContent.trim() === '新規作成');",
            )
            .await;
        }
    }
}

async fn license_fits(page: Arc<Page>, server: Arc<Server>) {
    let about = json!({"/api/about": {"json": {"version": "0.1.8", "source_url": "https://example.org/source.tar.gz"}}});
    for theme in LOOKS {
        lay(&page, &about).await;
        page.look(theme, 320, 812).await;
        page.open(&server.url("/about")).await;
        page.until(
            "the version",
            "return document.querySelector('h1, h2')?.textContent.includes('0.1.8');",
            Value::Null,
        )
        .await;
        let color = page
            .js(
                "return getComputedStyle(document.body).backgroundColor;",
                Value::Null,
            )
            .await;
        assert_eq!(color, background(theme), "{theme}");
        no_overflow(&page, theme).await;
    }
}

// ---- the queue ----

async fn recovery_in_sight(page: Arc<Page>, server: Arc<Server>) {
    let stopped = queue(|q| {
        q["current"]["state"] = json!("needs_attention");
        q["printer"]["print"]["state"] = json!("FAILED");
        set(&mut q["allowed"], &json!({"retry": true, "discard": true}));
        q["recovery"] = json!({"retry_reason": null, "discard_reason": null});
    });
    let changed = "Printer report does not match the recovery target";
    let blocked = queue(|q| {
        q["current"]["state"] = json!("needs_attention");
        q["printer"]["print"]["state"] = json!("FAILED");
        q["recovery"] = json!({"retry_reason": changed, "discard_reason": changed});
    });
    let retry = button("", "再印刷");
    for width in [320, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &stopped).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            page.until(
                &look,
                &format!("const b = (() => {{ {retry} }})(); return b && !b.disabled;"),
                Value::Null,
            )
            .await;
            let height = page
                .js(
                    &format!("return (() => {{ {retry} }})().getBoundingClientRect().height;"),
                    Value::Null,
                )
                .await;
            assert!(
                height.as_f64().unwrap() >= 44.0,
                "{look}: the retry is {height}px high"
            );
            lay(&page, &blocked).await;
            page.open(&server.url("/")).await;
            page.until(
                &look,
                &format!("const b = (() => {{ {retry} }})(); return b && b.disabled;"),
                Value::Null,
            )
            .await;
            bigger_text(&page).await;
            in_sight(&page, &format!("{look}: the retry"), &retry).await;
            no_overflow(&page, &look).await;
        }
    }
}

async fn failure_reason_fits(page: Arc<Page>, server: Arc<Server>) {
    let reason = "The printer reported that the storage is full and the uploaded project could not be opened for printing ".repeat(3)
        + "/sdcard/" + &"x".repeat(120);
    let routes = queue(|q| {
        set(
            &mut q["current"],
            &json!({"state": "needs_attention", "last_error": "Printer rejected the start request",
                "failure": {"kind": "rejected", "field": "err_code", "code": "0500-4003", "reason": reason}}),
        );
        set(
            &mut q["printer"]["print"],
            &json!({"state": "IDLE", "error": 0}),
        );
        q["allowed"] = json!({"next": false, "retry": true, "discard": true});
        q["recovery"] = json!({"retry_reason": null, "discard_reason": null});
    });
    for width in [320, 375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &routes).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            shows(&page, ".current-job summary").await;
            page.js(
                "document.querySelector('.current-job summary').click();",
                Value::Null,
            )
            .await;
            let right = page
                .until(
                    &look,
                    "const d = document.querySelector('.current-job .job-details'); \
                     return d && d.getBoundingClientRect().width > 0 && d.getBoundingClientRect().right;",
                    Value::Null,
                )
                .await;
            assert!(
                right.as_f64().unwrap() <= f64::from(width),
                "{look}: the details end at {right}"
            );
            no_overflow(&page, &look).await;
        }
    }
}

async fn not_started_in_sight(page: Arc<Page>, server: Arc<Server>) {
    let routes = queue(|q| {
        set(
            &mut q["current"],
            &json!({"state": "needs_attention", "failure": null,
                "last_error": "Printer status or selected AMS changed during transfer; no start command was sent"}),
        );
        q["printer"]["ready_to_print"] = json!(true);
        set(
            &mut q["printer"]["print"],
            &json!({"state": "IDLE", "error": 0}),
        );
        q["waiting"] = json!([]);
        q["allowed"] = json!({"next": false, "retry": true, "discard": true});
        q["recovery"] = json!({"retry_reason": null, "discard_reason": null});
    });
    let note = "return [...document.querySelectorAll('p, span, div')].find((e) => e.children.length === 0 \
        && e.textContent.trim() === '転送中に機器またはAMSの状態が変わりました。開始命令は送っていません。');";
    for width in [320, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &routes).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            in_sight(&page, &format!("{look}: the note"), note).await;
            in_sight(&page, &format!("{look}: the retry"), &button("", "再印刷")).await;
            page.js(
                "document.querySelector('.current-job > summary').click();",
                Value::Null,
            )
            .await;
            no_overflow(&page, &look).await;
        }
    }
}

async fn device_error_in_sight(page: Arc<Page>, server: Arc<Server>) {
    let previous = "Wait for a matching terminal report for the previous start";
    let routes = queue(|q| {
        set(
            &mut q["current"],
            &json!({"state": "needs_attention", "last_error": "Printer reported an error; inspect the printer",
                "failure": {"kind": "device_error", "field": "print_error", "code": "0300-8010", "state": "PAUSE"}}),
        );
        set(
            &mut q["printer"]["print"],
            &json!({"state": "PAUSE", "error": 0x0300_8010}),
        );
        q["printer"]["descriptions"] =
            json!({"0300-8010": "冷却ファンの回転速度は異常があります。"});
        q["waiting"] = json!([]);
        q["recovery"] = json!({"retry_reason": previous, "discard_reason": previous});
    });
    for width in [320, 375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &routes).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            page.until(
                &look,
                "return document.querySelector('.device-failure') !== null;",
                Value::Null,
            )
            .await;
            let at = page
                .js(
                    "const r = document.querySelector('.device-failure').getBoundingClientRect(); \
                     return r.top >= 0 && r.bottom <= innerHeight && r.right <= innerWidth;",
                    Value::Null,
                )
                .await;
            assert_eq!(at, true, "{look}: the error is out of sight");
            bigger_text(&page).await;
            in_sight(
                &page,
                &format!("{look}: the official help"),
                "return document.querySelector('.device-failure a');",
            )
            .await;
            no_overflow(&page, &look).await;
        }
    }
}

async fn queue_density(page: Arc<Page>, server: Arc<Server>) {
    let routes = queue(|_| {});
    for width in [375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &routes).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            page.until(
                &look,
                "return document.querySelectorAll('.job-summary').length === 9;",
                Value::Null,
            )
            .await;
            let metrics = page
                .js(
                    "const rows = [...document.querySelectorAll('.waiting-job')]; \
                     return { visible: rows.filter((n) => n.getBoundingClientRect().bottom <= innerHeight).length, \
                       lines: rows.map((n) => [...n.querySelector('.job-lines').children].map((c) => ({ \
                         height: c.getBoundingClientRect().height, \
                         lineHeight: parseFloat(getComputedStyle(c).lineHeight), \
                         nowrap: getComputedStyle(c).whiteSpace }))) };",
                    Value::Null,
                )
                .await;
            assert!(
                metrics["visible"].as_u64().unwrap() >= 6,
                "{look}: {metrics}"
            );
            for row in metrics["lines"].as_array().unwrap() {
                assert_eq!(row.as_array().unwrap().len(), 2, "{look}: {row}");
                for line in row.as_array().unwrap() {
                    assert_eq!(line["nowrap"], "nowrap", "{look}: {line}");
                    assert!(
                        line["height"].as_f64().unwrap()
                            <= line["lineHeight"].as_f64().unwrap() + 1.0,
                        "{look}: a line wraps: {line}"
                    );
                }
            }
            page.js(
                "document.querySelectorAll('.waiting-job summary')[1].click();",
                Value::Null,
            )
            .await;
            bigger_text(&page).await;
            in_sight(
                &page,
                &format!("{look}: the remove button"),
                "return [...document.querySelectorAll('.waiting-job')[1].querySelectorAll('button')]\
                 .find((b) => (b.getAttribute('aria-label') ?? b.textContent).includes('キューから削除'));",
            )
            .await;
            no_overflow(&page, &look).await;
        }
    }
}

/// The posted moves of the queue.
async fn moves(page: &Page) -> Vec<Value> {
    sent_to(page, "POST", "/api/queue")
        .await
        .into_iter()
        .map(|r| r["body"]["action"].clone())
        .collect()
}

/// A script that finds the drag handle of the `n`th waiting row.
fn handle(n: usize) -> String {
    format!("return document.querySelectorAll('.waiting-job .drag-handle')[{n}];")
}
/// A script that finds the `n`th waiting row.
fn row(n: usize) -> String {
    format!("return document.querySelectorAll('.waiting-job')[{n}];")
}

async fn pointer_drag(page: Arc<Page>, server: Arc<Server>) {
    let fixture: Value = serde_json::from_str(QUEUE).unwrap();
    let id = fixture["queue"]["waiting"][0]["id"].clone();
    lay(&page, &queue(|_| {})).await;
    page.look("light", 900, 900).await;
    page.open(&server.url("/queue?printer_id=p1")).await;
    page.until(
        "8 rows",
        "return document.querySelectorAll('.waiting-job').length === 8;",
        Value::Null,
    )
    .await;
    let (x, y) = middle(&page, &handle(0)).await;
    let target = page
        .js(
            &format!(
                "const r = (() => {{ {} }})().getBoundingClientRect(); return [r.x, r.bottom];",
                row(2)
            ),
            Value::Null,
        )
        .await;
    let (tx, ty) = (
        target[0].as_f64().unwrap() + 22.0,
        target[1].as_f64().unwrap() - 8.0,
    );
    page.mouse("hovered", x, y).await;
    page.mouse("pressed", x, y).await;
    for step in 1..=10 {
        let f = f64::from(step) / 10.0;
        page.mouse("moved", x + (tx - x) * f, y + (ty - y) * f)
            .await;
    }
    let inserting = format!(
        "return (() => {{ {} }})().classList.contains('insert-after');",
        row(2)
    );
    page.until("the third row marks the insertion", &inserting, Value::Null)
        .await;
    let reads = sent_to(&page, "GET", "/api/queue").await.len();
    page.until(
        "the queue is read again while dragging",
        "return JSON.parse(sessionStorage.getItem('test:sent')).filter((r) => r.method === 'GET' && r.path.startsWith('/api/queue')).length > arguments[0];",
        json!(reads),
    )
    .await;
    assert_eq!(
        page.js(&inserting, Value::Null).await,
        true,
        "polling dropped the insertion mark"
    );
    page.mouse("released", tx, ty).await;
    page.until(
        "the move is sent",
        "return JSON.parse(sessionStorage.getItem('test:sent')).some((r) => r.method === 'POST');",
        Value::Null,
    )
    .await;
    assert_eq!(
        moves(&page).await,
        [json!({"type": "move", "job_id": id, "index": 2})],
        "the moves sent"
    );
    // A drag the queue changes under is cancelled.
    let (x, y) = middle(&page, &handle(0)).await;
    page.mouse("pressed", x, y).await;
    for step in 1..=5 {
        page.mouse("moved", x, y + 18.0 * f64::from(step)).await;
    }
    lay(&page, &queue(|q| q["generation"] = json!(1))).await;
    page.until(
        "the stale drag ends",
        "return document.querySelector('.dragging') === null;",
        Value::Null,
    )
    .await;
    page.mouse("released", x, y + 90.0).await;
    assert_eq!(moves(&page).await.len(), 1, "a stale drag was sent");
    assert_eq!(
        page.js(
            "return document.querySelectorAll('.current-job .drag-handle').length;",
            Value::Null
        )
        .await,
        0,
        "the current job has a drag handle"
    );
}

async fn touch_drag(page: Arc<Page>, server: Arc<Server>) {
    let fixture: Value = serde_json::from_str(QUEUE).unwrap();
    let id = fixture["queue"]["waiting"][0]["id"].clone();
    lay(&page, &queue(|_| {})).await;
    page.look("light", 375, 520).await;
    page.touches(true).await;
    page.open(&server.url("/")).await;
    page.until(
        "8 rows",
        "return document.querySelectorAll('.waiting-job').length === 8;",
        Value::Null,
    )
    .await;
    page.touch("touchStart", 240.0, 450.0).await;
    for y in (130..=420).rev().step_by(30) {
        page.touch("touchMove", 240.0, f64::from(y)).await;
    }
    page.touch("touchEnd", 0.0, 0.0).await;
    page.until(
        "the finger scrolls the page",
        "return scrollY > 60;",
        Value::Null,
    )
    .await;
    assert_eq!(
        page.js(
            "return document.querySelector('.dragging') === null;",
            Value::Null
        )
        .await,
        true,
        "scrolling started a drag"
    );
    assert!(moves(&page).await.is_empty(), "scrolling moved a job");
    settle(&page).await;
    let (x, y) = middle(&page, &handle(0)).await;
    let target = page
        .js(
            &format!(
                "return (() => {{ {} }})().getBoundingClientRect().bottom;",
                row(2)
            ),
            Value::Null,
        )
        .await;
    page.touch("touchStart", x, y).await;
    page.touch("touchMove", x, target.as_f64().unwrap() - 6.0)
        .await;
    page.until(
        "the third row marks the insertion",
        &format!(
            "return (() => {{ {} }})().classList.contains('insert-after');",
            row(2)
        ),
        Value::Null,
    )
    .await;
    page.touch("touchEnd", 0.0, 0.0).await;
    page.until(
        "the move is sent",
        "return JSON.parse(sessionStorage.getItem('test:sent')).some((r) => r.method === 'POST');",
        Value::Null,
    )
    .await;
    assert_eq!(
        moves(&page).await,
        [json!({"type": "move", "job_id": id, "index": 2})],
        "the moves sent"
    );
    page.touches(false).await;
}

/// Waits until the scrolling a finger started has ended, and puts the page back at the top.
async fn settle(page: &Page) {
    page.until(
        "the page stops scrolling",
        "const y = scrollY; return new Promise((done) => setTimeout(() => done(scrollY === y), 200));",
        Value::Null,
    )
    .await;
    page.js("scrollTo(0, 0);", Value::Null).await;
    page.until(
        "the page is back at the top",
        "return scrollY === 0;",
        Value::Null,
    )
    .await;
}

// ---- the plates ----

const PLATE: &str = "11111111-1111-4111-8111-111111111111";

async fn detail_fits(page: Arc<Page>, server: Arc<Server>) {
    let machine = "Bambu Lab P1S 0.4 nozzle";
    let mini = "Bambu Lab A1 mini 0.2 nozzle";
    let mut printers = vec![json!({"id": "p1", "name": "P1S", "machine_profile_key": machine})];
    printers.extend((1..=3).map(|n| json!({"id": format!("a{n}"), "name": format!("A1 mini {n}"), "machine_profile_key": mini})));
    let routes = json!({
        format!("/api/plates/{PLATE}"): {"json": {"id": PLATE, "version": 1, "name": "繰り返し印刷するプレート",
            "models": [{"id": "item", "name": "part.stl", "source": "part.stl", "quantity": 10}],
            "conditions": {"filament_id": "white"}}},
        "/api/printers": {"json": printers},
        "/api/filaments": {"json": [{"id": "white", "name": "PLA 白", "vendor": "Fixture", "material": "PLA"}]},
        "/api/queue": {"json": {"epoch": "epoch", "generation": 1, "request_id": "request-1",
            "waiting": [{"id": "job", "plate_id": PLATE}], "current": null, "allowed": {"next": false},
            "admission": {"allowed": true, "plate_version": 1, "reason": null}}},
    });
    for theme in LOOKS {
        lay(&page, &routes).await;
        page.look(theme, 375, 812).await;
        page.open(&server.url(&format!("/plates/{PLATE}"))).await;
        page.until(
            "the destination",
            "return document.querySelector('select')?.options.length === 4;",
            Value::Null,
        )
        .await;
        no_overflow(&page, theme).await;
    }
}

/// Two saved plates named `name`, one printer, and a queue that admits them unless `held`.
fn menu_routes(name: &str, held: bool) -> Value {
    let list = plates(2, |i| {
        if name.is_empty() {
            format!("保存プレート {}", i + 1)
        } else {
            name.to_owned()
        }
    });
    let reason = if held {
        json!("No confirmed AMS slot contains the plate material")
    } else {
        Value::Null
    };
    json!({
        "/api/plates": {"json": list},
        format!("/api/plates/{}", list[0]["id"].as_str().unwrap()): {"json": list[0]},
        format!("/api/plates/{}", list[1]["id"].as_str().unwrap()): {"json": list[1]},
        "/api/printers": {"json": [{"id": "p1", "name": "P1S", "machine_profile_key": "Bambu Lab P1S 0.4 nozzle"}]},
        "/api/queue": {"json": {"epoch": "e", "generation": 0, "request_id": "request", "waiting": [], "current": null,
            "admission": {"allowed": !held, "plate_version": 1, "reason": reason}}},
    })
}

/// Opens the menu of the first plate row, as a right click does.
async fn open_plate_menu(page: &Page) {
    page.until(
        "two rows",
        "return document.querySelectorAll('.plate-row').length === 2;",
        Value::Null,
    )
    .await;
    page.js(
        "document.querySelector('.plate-row').dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true}));",
        Value::Null,
    )
    .await;
    shows(page, "[role=dialog]").await;
}

/// The delete button `remove` (a script that finds it) keeps a contrast of 4.5 with its
/// background, under the mouse and away from it.
async fn readable(page: &Page, look: &str, remove: &str) {
    for hover in [false, true] {
        if hover {
            let (x, y) = middle(page, remove).await;
            page.mouse("hovered", x, y).await;
        } else {
            page.mouse("hovered", 0.0, 0.0).await;
        }
        let contrast = page
                    .js(
                        &format!(
                            "const style = getComputedStyle((() => {{ {remove} }})()); \
                             const luminance = (rgb) => rgb.match(/[\\d.]+/g).slice(0, 3).map(Number).map((c) => c / 255) \
                               .map((c) => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4) \
                               .reduce((n, c, i) => n + c * [0.2126, 0.7152, 0.0722][i], 0); \
                             const a = luminance(style.color), b = luminance(style.backgroundColor); \
                             return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);"
                        ),
                        Value::Null,
                    )
                    .await;
        assert!(
            contrast.as_f64().unwrap() >= 4.5,
            "{look} hover {hover}: contrast {contrast}"
        );
    }
}

async fn plate_menu_looks(page: Arc<Page>, server: Arc<Server>) {
    let remove = button("[role=dialog] .queue-menu, [role=dialog]", "削除");
    for width in [375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &menu_routes("", false)).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/plates")).await;
            open_plate_menu(&page).await;
            let add = button("[role=dialog]", "キュー追加");
            page.until(
                &look,
                &format!("const b = (() => {{ {add} }})(); return b && !b.disabled;"),
                Value::Null,
            )
            .await;
            let widths = page
                .js(
                    &format!(
                        "return [(() => {{ {add} }})(), (() => {{ {} }})(), (() => {{ {remove} }})()]\
                         .map((b) => b.getBoundingClientRect().width);",
                        button("[role=dialog]", "編集")
                    ),
                    Value::Null,
                )
                .await;
            let widths: Vec<f64> = widths
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_f64().unwrap())
                .collect();
            assert!(
                (widths[0] - widths[1]).abs() < 1.0 && (widths[2] - widths[1]).abs() < 1.0,
                "{look}: {widths:?}"
            );
            let color = page
                .js(
                    &format!("return getComputedStyle((() => {{ {remove} }})()).color;"),
                    Value::Null,
                )
                .await;
            let expected = if theme == "dark" {
                "rgb(255, 107, 107)"
            } else {
                "rgb(156, 43, 29)"
            };
            assert_eq!(color, expected, "{look}: the delete colour");
            readable(&page, &look, &remove).await;
            page.key("Tab").await;
            let outline = page
                .js(&format!("const b = (() => {{ {remove} }})(); b.focus(); return getComputedStyle(b).outlineWidth;"), Value::Null)
                .await;
            assert_eq!(outline, "2px", "{look}: the focus outline");
            lay(&page, &menu_routes("", true)).await;
            page.open(&server.url("/plates")).await;
            open_plate_menu(&page).await;
            page.until(
                &look,
                &format!("const b = (() => {{ {add} }})(); return b && b.disabled;"),
                Value::Null,
            )
            .await;
            let widths = page
                .js(
                    &format!(
                        "return [(() => {{ {add} }})(), (() => {{ {} }})()].map((b) => b.getBoundingClientRect().width);",
                        button("[role=dialog]", "編集")
                    ),
                    Value::Null,
                )
                .await;
            assert!(
                (widths[0].as_f64().unwrap() - widths[1].as_f64().unwrap()).abs() < 1.0,
                "{look}: a disabled add is of another width: {widths}"
            );
        }
    }
}

async fn pending_deletion_dims(page: Arc<Page>, server: Arc<Server>) {
    let mut routes = menu_routes("", false);
    let first = routes["/api/plates"]["json"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    routes[format!("DELETE /api/plates/{first}")] =
        json!({"hold": "delete", "status": 503, "json": {"error": "一時的な保存エラー"}});
    lay(&page, &routes).await;
    page.open(&server.url("/plates")).await;
    open_plate_menu(&page).await;
    page.js(
        &format!(
            "(() => {{ {} }})().click();",
            button("[role=dialog]", "削除")
        ),
        Value::Null,
    )
    .await;
    page.js(
        &format!(
            "(() => {{ {} }})().click();",
            button("[role=dialog]", "削除")
        ),
        Value::Null,
    )
    .await;
    let opacity = page
        .until(
            "the button says it is deleting",
            &format!(
                "const b = (() => {{ {} }})(); return b && getComputedStyle(b).opacity;",
                button("[role=dialog]", "削除中…")
            ),
            Value::Null,
        )
        .await;
    assert_eq!(opacity, "0.5", "the pending deletion is not dimmed");
    release(&page, "delete").await;
    shows(&page, "[role=dialog] [role=alert]").await;
}

async fn long_names_fit(page: Arc<Page>, server: Arc<Server>) {
    lay(&page, &menu_routes(&"baseplate_front_".repeat(10), false)).await;
    page.look("light", 375, 812).await;
    page.open(&server.url("/plates")).await;
    bigger_text(&page).await;
    open_plate_menu(&page).await;
    let fits =
        "const d = document.querySelector('[role=dialog]'); return d.scrollWidth <= d.clientWidth;";
    assert_eq!(page.js(fits, Value::Null).await, true, "the menu overflows");
    page.js(
        &format!(
            "(() => {{ {} }})().click();",
            button("[role=dialog]", "削除")
        ),
        Value::Null,
    )
    .await;
    page.until("the confirmation", "return document.querySelector('[role=dialog]').getAttribute('aria-label') === 'プレートを削除';", Value::Null)
        .await;
    assert_eq!(
        page.js(fits, Value::Null).await,
        true,
        "the confirmation overflows"
    );
}

// ---- the history ----

fn history_routes() -> Value {
    let fixture: Value = serde_json::from_str(QUEUE).unwrap();
    let plate = fixture["queue"]["current"]["plate_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let items: Vec<Value> = (0..50)
        .map(|i| {
            json!({"id": 100 - i, "printer_id": "p1", "plate_id": plate, "completed_at": 1_750_000_000 - i,
                "available": i != 2,
                "name": if i == 0 { "長いプレート名・前面の取り付け部品と追加のケース".to_owned() } else { format!("部品 {i}") }})
        })
        .collect();
    let mut routes = queue(|_| {});
    let mut admitted = fixture["queue"].clone();
    admitted["admission"] = json!({"allowed": true, "plate_version": 7, "reason": null});
    routes["/api/history"] = json!({"json": {"items": items, "next_cursor": "1749999951:51"}});
    routes[format!("/api/plates/{plate}")] = json!({"json": {"id": plate, "name": "現在の名前", "version": 7, "conditions": {"filament_id": null}, "models": []}});
    routes[format!("/api/queue?plate_id={plate}")] = json!({"json": admitted});
    routes[format!("GET /api/queue?printer_id=p1&plate_id={plate}")] = json!({"json": admitted});
    routes["POST /api/queue"] = json!({"json": admitted});
    routes
}

/// Opens the menu of the history row `n`, as a right click does.
async fn open_history_menu(page: &Page, n: usize) {
    page.js(
        "document.querySelectorAll('.history-row')[arguments[0]].dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true}));",
        json!(n),
    )
    .await;
    shows(page, "[role=dialog]").await;
}

async fn history_fits(page: Arc<Page>, server: Arc<Server>) {
    let routes = history_routes();
    for width in [320, 375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &routes).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/history")).await;
            page.until(
                &look,
                "return document.querySelectorAll('.history-row').length === 50;",
                Value::Null,
            )
            .await;
            bigger_text(&page).await;
            no_overflow(&page, &look).await;
            open_history_menu(&page, 0).await;
            no_overflow(&page, &format!("{look} menu")).await;
        }
    }
}

async fn history_touch(page: Arc<Page>, server: Arc<Server>) {
    lay(&page, &history_routes()).await;
    page.look("light", 375, 520).await;
    page.touches(true).await;
    page.open(&server.url("/history")).await;
    page.until(
        "50 rows",
        "return document.querySelectorAll('.history-row').length === 50;",
        Value::Null,
    )
    .await;
    page.touch("touchStart", 180.0, 450.0).await;
    for y in (130..=420).rev().step_by(30) {
        page.touch("touchMove", 180.0, f64::from(y)).await;
    }
    page.touch("touchEnd", 0.0, 0.0).await;
    page.until(
        "the finger scrolls the page",
        "return scrollY > 60;",
        Value::Null,
    )
    .await;
    assert_eq!(
        page.js(
            "return document.querySelector('[role=dialog]') === null;",
            Value::Null
        )
        .await,
        true,
        "scrolling opened the menu"
    );
    settle(&page).await;
    let at = page
        .js("const r = document.querySelector('.history-row').getBoundingClientRect(); return [r.x + 100, r.y + 20];", Value::Null)
        .await;
    let (x, y) = (at[0].as_f64().unwrap(), at[1].as_f64().unwrap());
    page.touch("touchStart", x, y).await;
    shows(&page, "[role=dialog]").await;
    page.touch("touchEnd", 0.0, 0.0).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        page.js(
            "return document.querySelector('[role=dialog]') !== null;",
            Value::Null
        )
        .await,
        true,
        "lifting the finger closed the menu"
    );
    page.touches(false).await;
}

async fn history_keeps_position(page: Arc<Page>, server: Arc<Server>) {
    lay(&page, &history_routes()).await;
    page.look("light", 375, 812).await;
    page.open(&server.url("/history")).await;
    page.until(
        "50 rows",
        "return document.querySelectorAll('.history-row').length === 50;",
        Value::Null,
    )
    .await;
    let row = "document.querySelectorAll('.history-row')[30]";
    let before = page
        .js(&format!("{row}.scrollIntoView({{block: 'nearest'}}); return {row}.getBoundingClientRect().y;"), Value::Null)
        .await;
    open_history_menu(&page, 30).await;
    let add = button("[role=dialog]", "キューに追加");
    page.until(
        "the add is enabled",
        &format!("const b = (() => {{ {add} }})(); return b && !b.disabled;"),
        Value::Null,
    )
    .await;
    page.js(&format!("(() => {{ {add} }})().click();"), Value::Null)
        .await;
    page.until(
        "the menu closes",
        "return document.querySelector('[role=dialog]') === null;",
        Value::Null,
    )
    .await;
    in_sight(
        &page,
        "the result",
        "return [...document.querySelectorAll('[role=status]')].find((s) => s.textContent.includes('キューに追加しました'));",
    )
    .await;
    let after = page
        .js(
            &format!("return [{row}.getBoundingClientRect().y, document.activeElement === {row}];"),
            Value::Null,
        )
        .await;
    assert!(
        (after[0].as_f64().unwrap() - before.as_f64().unwrap()).abs() < 2.0,
        "the row moved from {before} to {after}"
    );
    assert_eq!(after[1], true, "the row lost the focus");
}

async fn queue_menu_widths(page: Arc<Page>, server: Arc<Server>) {
    let fixture: Value = serde_json::from_str(QUEUE).unwrap();
    let current = fixture["queue"]["current"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for width in [375, 900] {
        for theme in LOOKS {
            let look = format!("{width} {theme}");
            lay(&page, &queue(|_| {})).await;
            page.look(theme, width, 812).await;
            page.open(&server.url("/")).await;
            shows(&page, &format!("#job-{current} > summary")).await;
            page.js(
                "document.querySelector(`#job-${arguments[0]} > summary`).dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true}));",
                json!(current),
            )
            .await;
            let widths = page
                .until(
                    &look,
                    "const b = [...document.querySelectorAll('[role=dialog] .queue-menu > button')]; \
                     return b.length === 3 && b.map((n) => n.getBoundingClientRect().width);",
                    Value::Null,
                )
                .await;
            let widths = widths.as_array().unwrap();
            assert!(widths.iter().all(|w| w == &widths[0]), "{look}: {widths:?}");
        }
    }
}

async fn estimates_fit(page: Arc<Page>, server: Arc<Server>) {
    let routes = json!({
        format!("/api/plates/{PLATE}"): {"json": {"id": PLATE, "version": 1, "name": "キューブの保存済みプレート",
            "models": [{"id": "item", "name": "cube.stl", "source": "cube.stl", "quantity": 2}],
            "conditions": {"filament_id": "pla"}}},
        "/api/filaments": {"json": [{"id": "pla", "name": "PLA 青"}]},
        format!("/api/plates/{PLATE}/slice"): {"json": {"printers": [
            {"printer_id": "a1", "printer_name": "A1 mini", "machine_profile_key": "A1 mini 0.4", "state": "failed",
             "seconds": null, "error": "Configure this material for the required machine and nozzle first",
             "reason": "material_setting"},
            {"printer_id": "p1", "printer_name": "P1S", "machine_profile_key": "P1S 0.4", "state": "ready",
             "seconds": 1140, "error": null},
        ]}},
    });
    for theme in LOOKS {
        lay(&page, &routes).await;
        page.look(theme, 375, 812).await;
        page.open(&server.url(&format!("/plates/{PLATE}"))).await;
        page.until(
            "two printers' estimates",
            "return document.querySelectorAll('[aria-label=\"プレートの試算\"] li').length === 2;",
            Value::Null,
        )
        .await;
        page.js(
            "document.querySelector('[aria-label=\"プレートの試算\"] li summary').click();",
            Value::Null,
        )
        .await;
        no_overflow(&page, theme).await;
    }
}

// ---- running them ----

/// Which tests a run takes, as `cargo test` passes its words on: titles that hold any of the
/// words (all, when there are none), and none that hold a word after `--skip`; with
/// `--exact`, a word is a whole title. Other options are left alone.
fn chosen(args: &[String]) -> impl Fn(&str) -> bool {
    let mut words = Vec::new();
    let mut skipped = Vec::new();
    let mut exact = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--skip" => skipped.extend(args.next().cloned()),
            "--exact" => exact = true,
            option if option.starts_with('-') => {}
            word => words.push(word.to_owned()),
        }
    }
    move |title: &str| {
        let holds = |word: &String| {
            if exact {
                title == word
            } else {
                title.contains(word.as_str())
            }
        };
        (words.is_empty() || words.iter().any(holds)) && !skipped.iter().any(holds)
    }
}

/// What a panic said.
fn said(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|&said| said.to_owned()))
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chosen = chosen(&args);
    let tests: Vec<&Test> = TESTS.iter().filter(|test| chosen(test.0)).collect();
    if args.iter().any(|arg| arg == "--list") {
        for (title, _) in &tests {
            println!("{title}: test");
        }
        return std::process::ExitCode::SUCCESS;
    }
    println!("\nrunning {} tests", tests.len());
    if tests.is_empty() {
        return std::process::ExitCode::SUCCESS;
    }
    let began = Instant::now();
    let server = Arc::new(Server::start().await);
    let page = Arc::new(Page::start(&[PAGE_JS]).await);
    let mut failed = Vec::new();
    for (title, test) in &tests {
        // each test begins on a page of no state, in a window of 1280 by 720 in the light
        page.open(&server.url("/healthz")).await;
        page.js("localStorage.clear(); sessionStorage.clear();", Value::Null)
            .await;
        page.look("light", 1280, 720).await;
        let began = Instant::now();
        let ran = tokio::spawn(test(page.clone(), server.clone())).await;
        let took = began.elapsed().as_secs_f64();
        match ran {
            Ok(()) => println!("test {title} ... ok ({took:.2}s)"),
            Err(error) => {
                println!("test {title} ... FAILED: {}", said(&*error.into_panic()));
                failed.push(*title);
            }
        }
    }
    let page = Arc::try_unwrap(page).ok().unwrap();
    page.close().await;
    let seconds = began.elapsed().as_secs_f64();
    if failed.is_empty() {
        println!(
            "\ntest result: ok. {} passed; 0 failed; finished in {seconds:.2}s\n",
            tests.len()
        );
        std::process::ExitCode::SUCCESS
    } else {
        println!("\nfailures:");
        for title in &failed {
            println!("    {title}");
        }
        println!(
            "\ntest result: FAILED. {} passed; {} failed; finished in {seconds:.2}s\n",
            tests.len() - failed.len(),
            failed.len()
        );
        std::process::ExitCode::from(101)
    }
}
