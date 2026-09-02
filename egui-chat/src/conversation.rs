use codex_rollout::open_rollout_line_reader;
use serde::Deserialize;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;

const READ_CHUNK_SIZE: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ActivitySummary {
    pub(crate) active: bool,
    pub(crate) tool_calls_since_user: usize,
}

pub(crate) async fn summarize_rollout(path: &Path) -> io::Result<ActivitySummary> {
    if path.extension().is_some_and(|extension| extension == "zst") {
        summarize_forward(path).await
    } else {
        summarize_reverse(path)
    }
}

async fn summarize_forward(path: &Path) -> io::Result<ActivitySummary> {
    let mut reader = open_rollout_line_reader(path).await?;
    let mut summary = ActivitySummary::default();
    let mut saw_user = false;
    while let Some(line) = reader.next_line().await? {
        if let Some(event) = activity_event(&line) {
            observe_forward(&mut summary, &mut saw_user, event);
        }
    }
    Ok(summary)
}

fn summarize_reverse(path: &Path) -> io::Result<ActivitySummary> {
    let mut scanner = ReverseLines::new(File::open(path)?)?;
    let mut summary = ActivitySummary::default();
    let mut latest_active = None;
    let mut saw_user = false;
    while let Some(line) = scanner.next_line()? {
        match serde_json::from_slice::<MinimalRolloutLine>(&line)
            .ok()
            .map(|line| line.item.event())
        {
            Some(ActivityEvent::User) => {
                saw_user = true;
                break;
            }
            Some(ActivityEvent::ToolCall) => summary.tool_calls_since_user += 1,
            Some(ActivityEvent::Started) => {
                latest_active.get_or_insert(true);
            }
            Some(ActivityEvent::Finished) => {
                latest_active.get_or_insert(false);
            }
            Some(ActivityEvent::Other) | None => {}
        }
    }
    summary.active = latest_active.unwrap_or(saw_user);
    Ok(summary)
}

fn activity_event(line: &str) -> Option<ActivityEvent> {
    serde_json::from_str::<MinimalRolloutLine>(line)
        .ok()
        .map(|line| line.item.event())
}

fn observe_forward(summary: &mut ActivitySummary, saw_user: &mut bool, event: ActivityEvent) {
    match event {
        ActivityEvent::User => {
            *saw_user = true;
            summary.active = true;
            summary.tool_calls_since_user = 0;
        }
        ActivityEvent::ToolCall if *saw_user => summary.tool_calls_since_user += 1,
        ActivityEvent::Started => summary.active = true,
        ActivityEvent::Finished => summary.active = false,
        ActivityEvent::ToolCall | ActivityEvent::Other => {}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityEvent {
    User,
    ToolCall,
    Started,
    Finished,
    Other,
}

#[derive(Deserialize)]
struct MinimalRolloutLine {
    #[serde(flatten)]
    item: MinimalRolloutItem,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum MinimalRolloutItem {
    ResponseItem(MinimalResponseItem),
    EventMsg(MinimalEvent),
    #[serde(other)]
    Other,
}

impl MinimalRolloutItem {
    fn event(self) -> ActivityEvent {
        match self {
            Self::ResponseItem(item) => item.event(),
            Self::EventMsg(event) => event.event(),
            Self::Other => ActivityEvent::Other,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MinimalResponseItem {
    Message {
        role: String,
    },
    LocalShellCall {},
    FunctionCall {},
    ToolSearchCall {},
    CustomToolCall {},
    WebSearchCall {},
    ImageGenerationCall {},
    #[serde(other)]
    Other,
}

impl MinimalResponseItem {
    fn event(self) -> ActivityEvent {
        match self {
            Self::Message { role } if role == "user" => ActivityEvent::User,
            Self::LocalShellCall {}
            | Self::FunctionCall {}
            | Self::ToolSearchCall {}
            | Self::CustomToolCall {}
            | Self::WebSearchCall {}
            | Self::ImageGenerationCall {} => ActivityEvent::ToolCall,
            Self::Message { .. } | Self::Other => ActivityEvent::Other,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MinimalEvent {
    #[serde(rename = "task_started", alias = "turn_started")]
    TurnStarted,
    #[serde(rename = "task_complete", alias = "turn_complete")]
    TurnComplete,
    TurnAborted,
    UserMessage,
    #[serde(other)]
    Other,
}

impl MinimalEvent {
    fn event(self) -> ActivityEvent {
        match self {
            Self::TurnStarted => ActivityEvent::Started,
            Self::TurnComplete | Self::TurnAborted => ActivityEvent::Finished,
            Self::UserMessage => ActivityEvent::User,
            Self::Other => ActivityEvent::Other,
        }
    }
}

struct ReverseLines<R> {
    reader: R,
    next_chunk_end: u64,
    chunk: Vec<u8>,
    position: usize,
    reversed_line: Vec<u8>,
}

impl<R: Read + Seek> ReverseLines<R> {
    fn new(mut reader: R) -> io::Result<Self> {
        let next_chunk_end = reader.seek(SeekFrom::End(0))?;
        Ok(Self {
            reader,
            next_chunk_end,
            chunk: Vec::new(),
            position: 0,
            reversed_line: Vec::new(),
        })
    }

    fn next_line(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            let Some(byte) = self.previous_byte()? else {
                return Ok(self.finish_line());
            };
            if byte == b'\n' {
                if let Some(line) = self.finish_line() {
                    return Ok(Some(line));
                }
            } else {
                self.reversed_line.push(byte);
            }
        }
    }

    fn previous_byte(&mut self) -> io::Result<Option<u8>> {
        if self.position == 0 {
            if self.next_chunk_end == 0 {
                return Ok(None);
            }
            let start = self.next_chunk_end.saturating_sub(READ_CHUNK_SIZE);
            self.reader.seek(SeekFrom::Start(start))?;
            self.chunk.resize((self.next_chunk_end - start) as usize, 0);
            self.reader.read_exact(&mut self.chunk)?;
            self.next_chunk_end = start;
            self.position = self.chunk.len();
        }
        self.position -= 1;
        Ok(Some(self.chunk[self.position]))
    }

    fn finish_line(&mut self) -> Option<Vec<u8>> {
        if self.reversed_line.is_empty() {
            return None;
        }
        self.reversed_line.reverse();
        Some(std::mem::take(&mut self.reversed_line))
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
