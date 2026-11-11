//! `list_events`: what happened that could wake an owner's agent, read back
//! (ADR-0061). Every such event is logged before any delivery is tried, and stays
//! whether or not it was delivered, so this is what answers "what did I miss".
//!
//! An event says what happened and when. It holds no thesis and no note: the agent
//! reads the setup or the alert as it is now with their own tools.

use chartgen::scripting::persistence::{self, LoggedEvent, Store};
use serde_json::{json, Value};

use crate::mcp::{tool_err, tool_ok};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 100;

/// Is there a log to read? Where alerts or setups run, and never on a replay server,
/// which has no log and must answer the same on every run.
pub fn enabled() -> bool {
    crate::replay::global().is_none()
        && (crate::server_alerts::global().is_some() || crate::setup_service::global().is_some())
}

fn view(e: &LoggedEvent) -> Value {
    let mut v = json!({
        "seq": e.seq,
        "id": e.id,
        "type": e.kind,
        "at": e.at,
    });
    if let Some(alert) = &e.alert_id {
        v["alert_id"] = json!(alert);
    }
    if let Some(setup) = &e.setup_id {
        v["setup_id"] = json!(setup);
    }
    v["what"] = serde_json::from_str(&e.body).unwrap_or(Value::Null);
    v["delivery"] = json!(e.delivery);
    v
}

/// `owner` is the caller's user, or with sign-in off its OAuth client. Blocking.
pub fn call_tool(id: Option<Value>, args: &Value, owner: Option<&str>) -> Value {
    if crate::replay::global().is_some() {
        return tool_err(
            id,
            "list_events is not available on a replay server: a replay keeps no event log",
        );
    }
    let Some(owner) = owner else {
        return tool_err(id, "The event log needs an authenticated OAuth client");
    };
    let Some(store) = persistence::global().filter(|_| enabled()) else {
        return tool_err(id, "The event log is off on this server");
    };
    match answer(store, owner, args) {
        Ok(payload) => tool_ok(id, payload),
        Err(message) => tool_err(id, &message),
    }
}

/// The events of `owner` after the cursor in `args`, oldest first.
fn answer(store: &Store, owner: &str, args: &Value) -> Result<Value, String> {
    let number = |name: &str, default: i64, max: i64| match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_i64() {
            Some(n) if (0..=max).contains(&n) => Ok(n),
            _ => Err(format!("{name} must be a whole number from 0 to {max}")),
        },
    };
    let after = number("after", 0, i64::MAX)?;
    let limit = number("limit", DEFAULT_LIMIT, MAX_LIMIT)?.max(1);
    // One more than asked, to say whether more wait.
    let (mut events, kept, last) = store
        .events_after(owner, after, limit + 1)
        .and_then(|(events, _)| {
            Ok((
                events,
                store.events_kept_after(owner, after)?,
                store.last_event_seq(owner)?,
            ))
        })
        .map_err(|e| {
            eprintln!("[events] cannot read the log: {e}");
            "The event log is unavailable".to_string()
        })?;
    let more = events.len() as i64 > limit;
    events.truncate(limit as usize);
    // Fewer kept after the cursor than were numbered after it: the reader missed some
    // for good. Counted, since a wake-up that still waits is kept past its neighbours.
    let truncated = after < last && kept < last - after;
    // A cursor ahead of the log, from another server or a typo, comes back as the log's
    // end: the next event is then not skipped.
    let cursor = events.last().map_or(after.min(last), |e| e.seq);
    Ok(json!({
        "events": events.iter().map(view).collect::<Vec<_>>(),
        "cursor": cursor,
        "more": more,
        "truncated": truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chartgen::scripting::persistence::NewEvent;

    fn log(store: &Store, owner: &str, n: std::ops::RangeInclusive<usize>) {
        let events: Vec<NewEvent> = n
            .map(|n| NewEvent {
                id: format!("{owner}-{n}"),
                owner: owner.to_string(),
                kind: "setup.status".to_string(),
                at: 1_000 + n as i64,
                alert_id: None,
                setup_id: Some("s1".to_string()),
                body: r#"{"from":"watching","to":"triggered","gap":false}"#.to_string(),
                deliver: false,
            })
            .collect();
        store.save_setups_logging(&[], &events).unwrap();
    }

    fn seqs(answer: &Value) -> Vec<i64> {
        let events = answer["events"].as_array().unwrap();
        events.iter().map(|e| e["seq"].as_i64().unwrap()).collect()
    }

    #[test]
    fn events_come_oldest_first_in_pages_that_follow_each_other() {
        let store = Store::in_memory().unwrap();
        log(&store, "me", 1..=5);
        log(&store, "someone-else", 1..=3);
        let first = answer(&store, "me", &json!({ "limit": 2 })).unwrap();
        assert_eq!(seqs(&first), [1, 2]);
        assert_eq!(
            (first["cursor"].clone(), first["more"].clone()),
            (json!(2), json!(true))
        );
        assert_eq!(first["truncated"], false);
        let event = &first["events"][0];
        assert_eq!(event["type"], "setup.status");
        assert_eq!(event["at"], 1_001);
        assert_eq!(event["setup_id"], "s1");
        assert_eq!(event["what"]["to"], "triggered");
        assert_eq!(event["delivery"], "none");
        assert!(event.get("alert_id").is_none());

        let second = answer(&store, "me", &json!({ "after": 2, "limit": 2 })).unwrap();
        assert_eq!(seqs(&second), [3, 4]);
        let third = answer(&store, "me", &json!({ "after": 4 })).unwrap();
        assert_eq!(seqs(&third), [5]);
        assert_eq!(
            (third["cursor"].clone(), third["more"].clone()),
            (json!(5), json!(false))
        );
        // Nothing new: the cursor stays, so the next call asks the same.
        let none = answer(&store, "me", &json!({ "after": 5 })).unwrap();
        assert_eq!((seqs(&none), none["cursor"].clone()), (vec![], json!(5)));
        // Another owner sees its own, numbered by itself, and nothing of these.
        assert_eq!(
            seqs(&answer(&store, "someone-else", &json!({})).unwrap()),
            [1, 2, 3]
        );
        assert_eq!(
            seqs(&answer(&store, "a-stranger", &json!({})).unwrap()),
            Vec::<i64>::new()
        );
    }

    /// A reader whose cursor is older than what is kept is told that it missed events
    /// for good, and gets what there is from the oldest kept one.
    #[test]
    fn a_cursor_older_than_what_is_kept_says_truncated() {
        let store = Store::in_memory().unwrap();
        log(
            &store,
            "me",
            1..=persistence::EVENTS_PER_OWNER as usize + 10,
        );
        let late = answer(&store, "me", &json!({ "after": 3, "limit": 2 })).unwrap();
        assert_eq!(seqs(&late), [11, 12]);
        assert_eq!(late["truncated"], true);
        let in_time = answer(&store, "me", &json!({ "after": 10, "limit": 2 })).unwrap();
        assert_eq!(
            (seqs(&in_time), in_time["truncated"].clone()),
            (vec![11, 12], json!(false))
        );
        // Everything aged out: still truncated for a reader behind, and no number reused.
        let later = chrono::Utc::now().timestamp() + persistence::EVENT_TTL_SECS + 1;
        store.sweep_events(later).unwrap();
        let behind = answer(&store, "me", &json!({ "after": 3 })).unwrap();
        assert_eq!(
            (seqs(&behind), behind["truncated"].clone()),
            (vec![], json!(true))
        );
        let last = persistence::EVENTS_PER_OWNER + 10;
        let caught_up = answer(&store, "me", &json!({ "after": last })).unwrap();
        assert_eq!(
            (caught_up["truncated"].clone(), caught_up["cursor"].clone()),
            (json!(false), json!(last))
        );
        // A cursor ahead of the log comes back as the log's end.
        assert_eq!(
            answer(&store, "me", &json!({ "after": last + 500 })).unwrap()["cursor"],
            last
        );
    }

    /// A wake-up that still waits is kept past the others of its age. A reader behind
    /// it and one just after it are both told that events between are gone.
    #[test]
    fn a_kept_waiting_row_does_not_hide_what_was_dropped_after_it() {
        let store = Store::in_memory().unwrap();
        let waiting = NewEvent {
            id: "me-waiting".to_string(),
            owner: "me".to_string(),
            kind: "setup.status".to_string(),
            at: 1_000,
            alert_id: None,
            setup_id: None,
            body: "{}".to_string(),
            deliver: true,
        };
        store.save_setups_logging(&[], &[waiting]).unwrap();
        log(
            &store,
            "me",
            2..=persistence::EVENTS_PER_OWNER as usize + 10,
        );
        let from_start = answer(&store, "me", &json!({ "limit": 2 })).unwrap();
        assert_eq!(seqs(&from_start), [1, 11]);
        assert_eq!(from_start["truncated"], true);
        let after_it = answer(&store, "me", &json!({ "after": 1, "limit": 2 })).unwrap();
        assert_eq!(
            (seqs(&after_it), after_it["truncated"].clone()),
            (vec![11, 12], json!(true))
        );
        let in_time = answer(&store, "me", &json!({ "after": 10, "limit": 2 })).unwrap();
        assert_eq!(in_time["truncated"], false);
    }

    #[test]
    fn the_answer_is_bounded() {
        let store = Store::in_memory().unwrap();
        log(&store, "me", 1..=300);
        assert_eq!(seqs(&answer(&store, "me", &json!({})).unwrap()).len(), 50);
        assert_eq!(
            seqs(&answer(&store, "me", &json!({ "limit": 100 })).unwrap()).len(),
            100
        );
        for bad in [
            json!({ "limit": 101 }),
            json!({ "limit": -1 }),
            json!({ "after": -1 }),
            json!({ "after": "x" }),
        ] {
            assert!(answer(&store, "me", &bad).is_err(), "{bad}");
        }
        let largest = answer(&store, "me", &json!({ "limit": 100 }))
            .unwrap()
            .to_string()
            .len();
        assert!(largest < 64 * 1024, "{largest} bytes");
    }
}
