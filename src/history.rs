use crate::{
    plate_api::blocking,
    plates::{Error, Result, Store},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Entry {
    id: i64,
    attempt_id: String,
    job_id: String,
    plate_id: String,
    printer_id: String,
    name: String,
    completed_at: i64,
    available: bool,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Entry>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Search {
    before: Option<String>,
}

pub(crate) fn router(store: Store) -> Router {
    Router::new()
        .route("/api/history", get(list))
        .with_state(store)
}

async fn list(State(store): State<Store>, Query(search): Query<Search>) -> Result<Json<Page>> {
    let (time, id) = match search.before {
        None => (i64::MAX, i64::MAX),
        Some(cursor) => {
            let (time, id) = cursor
                .split_once(':')
                .and_then(|(time, id)| Some((time.parse::<i64>().ok()?, id.parse::<i64>().ok()?)))
                .filter(|(time, id)| *time >= 0 && *id > 0)
                .ok_or(Error::Invalid("Invalid history cursor"))?;
            (time, id)
        }
    };
    blocking(move || {
        let c = store.db.connection()?;
        let mut items = c
            .prepare(
                "SELECT h.id,h.attempt_id,h.job_id,h.plate_id,h.printer_id,h.name,h.completed_at,
            EXISTS(SELECT 1 FROM plates p WHERE p.id=h.plate_id AND p.deleted=0)
            FROM print_history h WHERE (h.completed_at,h.id) < (?1,?2)
            ORDER BY h.completed_at DESC,h.id DESC LIMIT 51",
            )?
            .query_map(rusqlite::params![time, id], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    attempt_id: row.get(1)?,
                    job_id: row.get(2)?,
                    plate_id: row.get(3)?,
                    printer_id: row.get(4)?,
                    name: row.get(5)?,
                    completed_at: row.get(6)?,
                    available: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > 50 {
            items.truncate(50);
            items
                .last()
                .map(|entry| format!("{}:{}", entry.completed_at, entry.id))
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    })
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use crate::plates::Store;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    async fn page(store: &Store, query: &str, status: u16) -> Value {
        let response = crate::app_with_store(store.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/api/history{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn history_pages_keep_names_and_stable_completion_order() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let empty = page(&store, "", 200).await;
        assert_eq!(empty["items"], serde_json::json!([]));
        assert!(empty["next_cursor"].is_null());
        {
            let c = store.db.connection().unwrap();
            c.execute(
                "INSERT INTO plates(id,name) VALUES ('plate','Renamed now')",
                [],
            )
            .unwrap();
            for index in 0..105 {
                c.execute("INSERT INTO print_history(attempt_id,job_id,plate_id,printer_id,name,completed_at) VALUES (?1,?2,'plate','removed-printer','Printed name',?3)",
                    rusqlite::params![format!("attempt-{index}"),format!("job-{index}"),100 + index / 3]).unwrap();
            }
        }
        let first = page(&store, "", 200).await;
        assert_eq!(first["items"].as_array().unwrap().len(), 50);
        let mut entries = first["items"].as_array().unwrap().clone();
        assert_eq!(entries[0]["name"], "Printed name");
        assert_eq!(entries[0]["available"], true);
        assert_eq!(entries[0]["attempt_id"], "attempt-104");
        // A new completion before the current page cannot shift the next page.
        store.db.connection().unwrap().execute("INSERT INTO print_history(attempt_id,job_id,plate_id,printer_id,name,completed_at) VALUES ('new','new','plate','removed-printer','New print',1000)", []).unwrap();
        let second = page(
            &store,
            &format!("?before={}", first["next_cursor"].as_str().unwrap()),
            200,
        )
        .await;
        entries.extend(second["items"].as_array().unwrap().clone());
        let last = page(
            &store,
            &format!("?before={}", second["next_cursor"].as_str().unwrap()),
            200,
        )
        .await;
        entries.extend(last["items"].as_array().unwrap().clone());
        assert!(last["next_cursor"].is_null());
        let attempts: Vec<_> = entries
            .iter()
            .map(|v| v["attempt_id"].as_str().unwrap())
            .collect();
        let expected: Vec<_> = (0..105).rev().map(|i| format!("attempt-{i}")).collect();
        assert_eq!(attempts, expected);
        for pair in entries.windows(2) {
            assert!(pair[0]["completed_at"].as_i64() >= pair[1]["completed_at"].as_i64());
        }
        store
            .db
            .connection()
            .unwrap()
            .execute("UPDATE plates SET deleted=1 WHERE id='plate'", [])
            .unwrap();
        let deleted = page(&store, "", 200).await;
        assert_eq!(deleted["items"][1]["name"], "Printed name");
        assert_eq!(deleted["items"][1]["available"], false);
        store
            .db
            .connection()
            .unwrap()
            .execute("DELETE FROM plates", [])
            .unwrap();
        drop(store);
        let reopened = Store::open(root.path()).unwrap();
        let absent = page(&reopened, "", 200).await;
        assert_eq!(absent["items"][1]["name"], "Printed name");
        assert_eq!(absent["items"][1]["completed_at"], 134);
        assert_eq!(absent["items"][1]["available"], false);
        for invalid in [
            "bad",
            "1",
            "-1:2",
            "1:0",
            "1:2:3",
            "999999999999999999999:1",
        ] {
            assert!(page(&reopened, &format!("?before={invalid}"), 400).await["error"].is_string());
        }
    }
}
