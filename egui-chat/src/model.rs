use codex_protocol::protocol::RolloutItem;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;

const MAX_BODY_CHARS: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeKind {
    User,
    Assistant,
    Agent,
    Reasoning,
    Tool,
    Other,
}

impl NodeKind {
    pub(crate) fn is_tool(self) -> bool {
        self == Self::Tool
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatNode {
    pub(crate) kind: NodeKind,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) body: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum VisibleLine {
    Node(ChatNode),
    Tool,
    Ignored,
}

pub(crate) fn visible_line(line: &str) -> serde_json::Result<VisibleLine> {
    let line = serde_json::from_str::<VisibleRolloutLine>(line)?;
    Ok(match line {
        VisibleRolloutLine::ResponseItem(item) => visible_response_line(item),
        VisibleRolloutLine::InterAgentCommunication(message) => VisibleLine::Node(ChatNode {
            kind: NodeKind::Agent,
            title: format!("{} → {}", message.author, message.recipient),
            summary: "Agent message".to_string(),
            body: truncate(&message.content),
        }),
        VisibleRolloutLine::SessionMeta(_)
        | VisibleRolloutLine::InterAgentCommunicationMetadata(_)
        | VisibleRolloutLine::Compacted(_)
        | VisibleRolloutLine::TurnContext(_)
        | VisibleRolloutLine::WorldState(_)
        | VisibleRolloutLine::EventMsg(_)
        | VisibleRolloutLine::Other => VisibleLine::Ignored,
    })
}

pub(crate) fn nodes_from_rollout(items: &[RolloutItem]) -> Vec<ChatNode> {
    let call_names = items
        .iter()
        .filter_map(response_value)
        .filter_map(|value| {
            let item = value.as_object()?;
            let call_id = item.get("call_id")?.as_str()?;
            call_name(item).map(|name| (call_id.to_string(), name))
        })
        .collect::<HashMap<_, _>>();

    items
        .iter()
        .filter_map(|item| match item {
            RolloutItem::ResponseItem(_) => {
                response_value(item).and_then(|value| response_node(&value, &call_names))
            }
            RolloutItem::InterAgentCommunication(message) => Some(ChatNode {
                kind: NodeKind::Agent,
                title: format!("{} → {}", message.author, message.recipient),
                summary: "Agent message".to_string(),
                body: truncate(&message.content),
            }),
            RolloutItem::SessionMeta(_)
            | RolloutItem::InterAgentCommunicationMetadata { .. }
            | RolloutItem::Compacted(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::WorldState(_)
            | RolloutItem::EventMsg(_) => None,
        })
        .collect()
}

fn visible_response_line(item: VisibleResponseItem) -> VisibleLine {
    match item {
        VisibleResponseItem::Message { role, content } => VisibleLine::Node(ChatNode {
            kind: match role.as_str() {
                "user" => NodeKind::User,
                "assistant" => NodeKind::Assistant,
                _ => NodeKind::Other,
            },
            title: title_case(&role),
            summary: String::new(),
            body: truncate(&content_text(Some(&content))),
        }),
        VisibleResponseItem::AgentMessage {
            author,
            recipient,
            content,
        } => VisibleLine::Node(ChatNode {
            kind: NodeKind::Agent,
            title: format!("{author} → {recipient}"),
            summary: "Agent message".to_string(),
            body: truncate(&content_text(Some(&content))),
        }),
        VisibleResponseItem::Reasoning { summary, content } => {
            let body = [&summary, &content]
                .into_iter()
                .map(|value| content_text(Some(value)))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            if body.is_empty() {
                VisibleLine::Ignored
            } else {
                VisibleLine::Node(ChatNode {
                    kind: NodeKind::Reasoning,
                    title: "Reasoning".to_string(),
                    summary: single_line(&body),
                    body: truncate(&body),
                })
            }
        }
        VisibleResponseItem::AdditionalTools {}
        | VisibleResponseItem::LocalShellCall {}
        | VisibleResponseItem::FunctionCall {}
        | VisibleResponseItem::ToolSearchCall {}
        | VisibleResponseItem::FunctionCallOutput {}
        | VisibleResponseItem::CustomToolCall {}
        | VisibleResponseItem::CustomToolCallOutput {}
        | VisibleResponseItem::ToolSearchOutput {}
        | VisibleResponseItem::WebSearchCall {}
        | VisibleResponseItem::ImageGenerationCall {} => VisibleLine::Tool,
        VisibleResponseItem::Other => VisibleLine::Ignored,
    }
}

fn response_value(item: &RolloutItem) -> Option<Value> {
    let RolloutItem::ResponseItem(response_item) = item else {
        return None;
    };
    serde_json::to_value(response_item).ok()
}

fn response_node(value: &Value, call_names: &HashMap<String, String>) -> Option<ChatNode> {
    let item = value.as_object()?;
    let item_type = item.get("type")?.as_str()?;
    match item_type {
        "message" => {
            let role = item.get("role")?.as_str()?;
            Some(ChatNode {
                kind: match role {
                    "user" => NodeKind::User,
                    "assistant" => NodeKind::Assistant,
                    _ => NodeKind::Other,
                },
                title: title_case(role),
                summary: String::new(),
                body: truncate(&content_text(item.get("content"))),
            })
        }
        "agent_message" => Some(ChatNode {
            kind: NodeKind::Agent,
            title: format!(
                "{} → {}",
                string_field(item, "author", "agent"),
                string_field(item, "recipient", "agent")
            ),
            summary: "Agent message".to_string(),
            body: truncate(&content_text(item.get("content"))),
        }),
        "reasoning" => {
            let body = [item.get("summary"), item.get("content")]
                .into_iter()
                .map(content_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!body.is_empty()).then(|| ChatNode {
                kind: NodeKind::Reasoning,
                title: "Reasoning".to_string(),
                summary: single_line(&body),
                body: truncate(&body),
            })
        }
        item_type if is_tool_item(item_type) => Some(tool_node(item_type, item, call_names)),
        _ => None,
    }
}

fn tool_node(
    item_type: &str,
    item: &Map<String, Value>,
    call_names: &HashMap<String, String>,
) -> ChatNode {
    let call_id = item.get("call_id").and_then(Value::as_str);
    let is_output = item_type.ends_with("_output");
    let title = if is_output {
        format!(
            "{} result",
            call_id
                .and_then(|call_id| call_names.get(call_id))
                .cloned()
                .unwrap_or_else(|| humanize(item_type.trim_end_matches("_output")))
        )
    } else {
        call_name(item).unwrap_or_else(|| humanize(item_type.trim_end_matches("_call")))
    };
    let summary = if is_output {
        item.get("status")
            .map(value_text)
            .unwrap_or_else(|| "completed".to_string())
    } else {
        item.get("arguments")
            .or_else(|| item.get("input"))
            .or_else(|| item.get("revised_prompt"))
            .or_else(|| item.get("status"))
            .map(value_text)
            .unwrap_or_default()
    };
    let body = item
        .get("output")
        .or_else(|| item.get("input"))
        .or_else(|| item.get("action"))
        .map(value_text)
        .or_else(|| {
            item.get("arguments").map(|arguments| {
                arguments
                    .as_str()
                    .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                    .map_or_else(|| value_text(arguments), |value| format_value(&value))
            })
        })
        .unwrap_or_else(|| {
            let mut safe_item = item.clone();
            if let Some(result) = safe_item.remove("result") {
                safe_item.insert(
                    "result".to_string(),
                    Value::String(format!("<{} byte payload>", value_text(&result).len())),
                );
            }
            format_value(&safe_item)
        });
    ChatNode {
        kind: NodeKind::Tool,
        title,
        summary: single_line(&summary),
        body: truncate(&body),
    }
}

fn call_name(item: &Map<String, Value>) -> Option<String> {
    let item_type = item.get("type")?.as_str()?;
    if let Some(name) = item.get("name").and_then(Value::as_str) {
        return Some(item.get("namespace").and_then(Value::as_str).map_or_else(
            || name.to_string(),
            |namespace| format!("{namespace}.{name}"),
        ));
    }
    match item_type {
        "local_shell_call" => Some("shell".to_string()),
        "tool_search_call" => Some("tool search".to_string()),
        "web_search_call" => Some("web search".to_string()),
        "image_generation_call" => Some("image generation".to_string()),
        _ => None,
    }
}

fn is_tool_item(item_type: &str) -> bool {
    item_type.ends_with("_call")
        || item_type.ends_with("_output")
        || item_type == "additional_tools"
}

fn content_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    item.get("image_url")
                        .and_then(Value::as_str)
                        .map(|value| media_label("image", value))
                })
                .or_else(|| {
                    item.get("audio_url")
                        .and_then(Value::as_str)
                        .map(|value| media_label("audio", value))
                })
                .or_else(|| {
                    item.get("encrypted_content")
                        .map(|_| "<encrypted content>".to_string())
                })
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn string_field<'a>(item: &'a Map<String, Value>, field: &str, fallback: &'a str) -> &'a str {
    item.get(field).and_then(Value::as_str).unwrap_or(fallback)
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| format_value(value), str::to_string)
}

fn format_value(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("<could not format value: {error}>"))
}

fn media_label(kind: &str, value: &str) -> String {
    if value.starts_with("data:") {
        format!("<{kind} data: {} bytes>", value.len())
    } else {
        format!("<{kind}: {}>", truncate_to(value, 512))
    }
}

fn title_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(
        || "Message".to_string(),
        |first| first.to_uppercase().chain(characters).collect(),
    )
}

fn humanize(text: &str) -> String {
    text.replace('_', " ")
}

fn single_line(text: &str) -> String {
    truncate_to(&text.split_whitespace().collect::<Vec<_>>().join(" "), 140)
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_BODY_CHARS {
        text.to_string()
    } else {
        format!(
            "{}\n\n… truncated for display …",
            text.chars().take(MAX_BODY_CHARS).collect::<String>()
        )
    }
}

fn truncate_to(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum VisibleRolloutLine {
    SessionMeta(IgnoredAny),
    ResponseItem(VisibleResponseItem),
    InterAgentCommunication(VisibleInterAgentCommunication),
    InterAgentCommunicationMetadata(IgnoredAny),
    Compacted(IgnoredAny),
    TurnContext(IgnoredAny),
    WorldState(IgnoredAny),
    EventMsg(IgnoredAny),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum VisibleResponseItem {
    Message {
        role: String,
        content: Value,
    },
    AgentMessage {
        author: String,
        recipient: String,
        content: Value,
    },
    Reasoning {
        #[serde(default)]
        summary: Value,
        #[serde(default)]
        content: Value,
    },
    AdditionalTools {},
    LocalShellCall {},
    FunctionCall {},
    ToolSearchCall {},
    FunctionCallOutput {},
    CustomToolCall {},
    CustomToolCallOutput {},
    ToolSearchOutput {},
    WebSearchCall {},
    ImageGenerationCall {},
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct VisibleInterAgentCommunication {
    author: String,
    recipient: String,
    content: String,
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
