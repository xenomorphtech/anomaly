use super::*;
use pretty_assertions::assert_eq;
use std::io::Cursor;

#[test]
fn tracks_work_and_tool_calls_since_the_latest_user_message() {
    let lines = [
        r#"{"type":"event_msg","payload":{"type":"user_message","message":"first"}}"#,
        r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
        r#"{"type":"response_item","payload":{"type":"function_call"}}"#,
        r#"{"type":"response_item","payload":{"type":"web_search_call"}}"#,
    ];
    let mut summary = ActivitySummary::default();
    let mut saw_user = false;
    for line in lines {
        observe_forward(
            &mut summary,
            &mut saw_user,
            activity_event(line).expect("activity event"),
        );
    }
    assert_eq!(
        summary,
        ActivitySummary {
            active: true,
            tool_calls_since_user: 2,
        }
    );

    observe_forward(
        &mut summary,
        &mut saw_user,
        activity_event(r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#)
            .expect("turn complete"),
    );
    assert_eq!(
        summary,
        ActivitySummary {
            active: false,
            tool_calls_since_user: 2,
        }
    );
}

#[test]
fn a_new_user_message_resets_the_tool_count() {
    let mut summary = ActivitySummary {
        active: false,
        tool_calls_since_user: 7,
    };
    let mut saw_user = true;
    observe_forward(
        &mut summary,
        &mut saw_user,
        activity_event(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[]}}"#,
        )
        .expect("user message"),
    );
    assert_eq!(
        summary,
        ActivitySummary {
            active: true,
            tool_calls_since_user: 0,
        }
    );
}

#[test]
fn reverse_lines_reads_newest_records_first() {
    let mut lines = ReverseLines::new(Cursor::new(b"first\nsecond\n".to_vec())).expect("scanner");
    assert_eq!(lines.next_line().expect("line"), Some(b"second".to_vec()));
    assert_eq!(lines.next_line().expect("line"), Some(b"first".to_vec()));
    assert_eq!(lines.next_line().expect("end"), None);
}
