use super::*;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;

#[test]
fn builds_separate_chat_nodes_and_media_placeholders() {
    let items = vec![
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![
                ContentItem::InputText {
                    text: "Show me the changes".to_string(),
                },
                ContentItem::InputImage {
                    image_url: "data:image/png;base64,AAAA".to_string(),
                    detail: None,
                },
            ],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }),
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "Here they are.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }),
    ];

    assert_eq!(
        nodes_from_rollout(&items),
        vec![
            ChatNode {
                kind: NodeKind::User,
                title: "User".to_string(),
                summary: String::new(),
                body: "Show me the changes\n\n<image data: 26 bytes>".to_string(),
            },
            ChatNode {
                kind: NodeKind::Assistant,
                title: "Assistant".to_string(),
                summary: String::new(),
                body: "Here they are.".to_string(),
            },
        ]
    );
}

#[test]
fn labels_tool_results_with_their_call_name() {
    let items = vec![
        RolloutItem::ResponseItem(ResponseItem::FunctionCall {
            id: None,
            name: "exec_command".to_string(),
            namespace: Some("functions".to_string()),
            arguments: r#"{"cmd":"just test"}"#.to_string(),
            call_id: "call-1".to_string(),
            internal_chat_message_metadata_passthrough: None,
        }),
        RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call-1".to_string(),
            output: FunctionCallOutputPayload::from_text("all tests passed".to_string()),
            internal_chat_message_metadata_passthrough: None,
        }),
    ];

    assert_eq!(
        nodes_from_rollout(&items),
        vec![
            ChatNode {
                kind: NodeKind::Tool,
                title: "functions.exec_command".to_string(),
                summary: r#"{"cmd":"just test"}"#.to_string(),
                body: "{\n  \"cmd\": \"just test\"\n}".to_string(),
            },
            ChatNode {
                kind: NodeKind::Tool,
                title: "functions.exec_command result".to_string(),
                summary: "completed".to_string(),
                body: "all tests passed".to_string(),
            },
        ]
    );
}

#[test]
fn parses_visible_nodes_without_materializing_tool_payloads() {
    assert_eq!(
        visible_line(
            r#"{"timestamp":"2026-07-25T10:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#,
        )
        .unwrap(),
        VisibleLine::Node(ChatNode {
            kind: NodeKind::User,
            title: "User".to_string(),
            summary: String::new(),
            body: "hello".to_string(),
        })
    );
    assert_eq!(
        visible_line(
            r#"{"timestamp":"2026-07-25T10:00:01Z","type":"response_item","payload":{"type":"image_generation_call","status":"completed","result":"large payload"}}"#,
        )
        .unwrap(),
        VisibleLine::Tool
    );
}
