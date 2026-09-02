use crate::model::ChatNode;
use crate::model::VisibleLine;
use crate::model::nodes_from_rollout;
use crate::model::visible_line;
use codex_rollout::RolloutRecorder;
use codex_rollout::open_rollout_line_reader;
use eframe::egui;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

const CACHE_ENTRY_LIMIT: usize = 8;
const CACHE_BYTE_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum NodeSet {
    Chat,
    ChatAndTools,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileStamp {
    modified_at: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            modified_at: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RolloutKey {
    path: PathBuf,
    stamp: FileStamp,
    node_set: NodeSet,
}

impl RolloutKey {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn stamp(&self) -> FileStamp {
        self.stamp
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LoadOrigin {
    Cache,
    Disk(Duration),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadedRollout {
    pub(crate) nodes: Arc<[ChatNode]>,
    pub(crate) tool_count: usize,
    pub(crate) parse_errors: usize,
    pub(crate) origin: LoadOrigin,
}

pub(crate) enum LoadStart {
    Ready {
        key: RolloutKey,
        rollout: LoadedRollout,
    },
    Pending(RolloutKey),
    Failed {
        path: PathBuf,
        error: String,
    },
}

pub(crate) struct LoadCompletion {
    pub(crate) key: RolloutKey,
    pub(crate) result: Result<LoadedRollout, String>,
}

pub(crate) struct RolloutLoader {
    requests: mpsc::Sender<LoadRequest>,
    results: mpsc::Receiver<WorkerResult>,
    pending: HashSet<RolloutKey>,
    cache: VecDeque<CacheEntry>,
    cache_bytes: usize,
    startup_error: Option<String>,
}

impl RolloutLoader {
    pub(crate) fn new(repaint: egui::Context) -> Self {
        let (request_sender, request_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let startup_error = std::thread::Builder::new()
            .name("codex-egui-rollout-loader".to_string())
            .spawn(move || worker(request_receiver, result_sender, repaint))
            .err()
            .map(|error| error.to_string());
        Self {
            requests: request_sender,
            results: result_receiver,
            pending: HashSet::new(),
            cache: VecDeque::new(),
            cache_bytes: 0,
            startup_error,
        }
    }

    pub(crate) fn request(&mut self, path: PathBuf, node_set: NodeSet) -> LoadStart {
        if let Some(error) = &self.startup_error {
            return LoadStart::Failed {
                path,
                error: error.clone(),
            };
        }
        let stamp = match FileStamp::read(&path) {
            Ok(stamp) => stamp,
            Err(error) => {
                return LoadStart::Failed {
                    path,
                    error: error.to_string(),
                };
            }
        };
        let key = RolloutKey {
            path,
            stamp,
            node_set,
        };
        if let Some(document) = self.cached(&key) {
            return LoadStart::Ready {
                key,
                rollout: document.loaded(LoadOrigin::Cache),
            };
        }
        if self.pending.contains(&key) {
            return LoadStart::Pending(key);
        }
        if let Err(error) = self.requests.send(LoadRequest { key: key.clone() }) {
            return LoadStart::Failed {
                path: key.path,
                error: error.to_string(),
            };
        }
        self.pending.insert(key.clone());
        LoadStart::Pending(key)
    }

    pub(crate) fn poll(&mut self) -> Vec<LoadCompletion> {
        let results = self.results.try_iter().collect::<Vec<_>>();
        let mut completed = Vec::new();
        for result in results {
            match result {
                WorkerResult::Superseded(key) => {
                    self.pending.remove(&key);
                }
                WorkerResult::Finished {
                    key,
                    result,
                    elapsed,
                } => {
                    self.pending.remove(&key);
                    let result = result.map(|document| {
                        self.insert(key.clone(), document.clone());
                        document.loaded(LoadOrigin::Disk(elapsed))
                    });
                    completed.push(LoadCompletion {
                        key,
                        result: result.map_err(|error| error.to_string()),
                    });
                }
            }
        }
        completed
    }

    pub(crate) fn invalidate(&mut self, path: &Path) {
        self.evict_where(|entry| entry.key.path == path);
    }

    fn cached(&mut self, key: &RolloutKey) -> Option<LoadedDocument> {
        let index = self.cache.iter().position(|entry| &entry.key == key)?;
        let entry = self.cache.remove(index)?;
        let document = entry.document.clone();
        self.cache.push_front(entry);
        Some(document)
    }

    fn insert(&mut self, key: RolloutKey, document: LoadedDocument) {
        self.evict_where(|entry| {
            entry.key.path == key.path
                && entry.key.node_set == key.node_set
                && entry.key.stamp != key.stamp
        });
        if document.estimated_bytes > CACHE_BYTE_LIMIT {
            return;
        }
        while self.cache.len() >= CACHE_ENTRY_LIMIT
            || self.cache_bytes.saturating_add(document.estimated_bytes) > CACHE_BYTE_LIMIT
        {
            let Some(entry) = self.cache.pop_back() else {
                break;
            };
            self.cache_bytes = self
                .cache_bytes
                .saturating_sub(entry.document.estimated_bytes);
        }
        self.cache_bytes = self.cache_bytes.saturating_add(document.estimated_bytes);
        self.cache.push_front(CacheEntry { key, document });
    }

    fn evict_where(&mut self, mut should_evict: impl FnMut(&CacheEntry) -> bool) {
        let mut retained = VecDeque::with_capacity(self.cache.len());
        while let Some(entry) = self.cache.pop_front() {
            if should_evict(&entry) {
                self.cache_bytes = self
                    .cache_bytes
                    .saturating_sub(entry.document.estimated_bytes);
            } else {
                retained.push_back(entry);
            }
        }
        self.cache = retained;
    }
}

#[derive(Clone)]
struct LoadedDocument {
    nodes: Arc<[ChatNode]>,
    tool_count: usize,
    parse_errors: usize,
    estimated_bytes: usize,
}

impl LoadedDocument {
    fn new(nodes: Vec<ChatNode>, tool_count: usize, parse_errors: usize) -> Self {
        let estimated_bytes = nodes.iter().fold(
            std::mem::size_of::<ChatNode>() * nodes.len(),
            |bytes, node| {
                bytes
                    .saturating_add(node.title.len())
                    .saturating_add(node.summary.len())
                    .saturating_add(node.body.len())
            },
        );
        Self {
            nodes: nodes.into(),
            tool_count,
            parse_errors,
            estimated_bytes,
        }
    }

    fn loaded(&self, origin: LoadOrigin) -> LoadedRollout {
        LoadedRollout {
            nodes: Arc::clone(&self.nodes),
            tool_count: self.tool_count,
            parse_errors: self.parse_errors,
            origin,
        }
    }
}

struct CacheEntry {
    key: RolloutKey,
    document: LoadedDocument,
}

struct LoadRequest {
    key: RolloutKey,
}

enum WorkerResult {
    Superseded(RolloutKey),
    Finished {
        key: RolloutKey,
        result: io::Result<LoadedDocument>,
        elapsed: Duration,
    },
}

fn worker(
    requests: mpsc::Receiver<LoadRequest>,
    results: mpsc::Sender<WorkerResult>,
    repaint: egui::Context,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            for request in requests {
                let _ = results.send(WorkerResult::Finished {
                    key: request.key,
                    result: Err(io::Error::other(error.to_string())),
                    elapsed: Duration::ZERO,
                });
            }
            repaint.request_repaint();
            return;
        }
    };
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            if results.send(WorkerResult::Superseded(request.key)).is_err() {
                return;
            }
            request = newer;
        }
        let started = Instant::now();
        let result = runtime.block_on(load_document(&request.key));
        if results
            .send(WorkerResult::Finished {
                key: request.key,
                result,
                elapsed: started.elapsed(),
            })
            .is_err()
        {
            return;
        }
        repaint.request_repaint();
    }
}

async fn load_document(key: &RolloutKey) -> io::Result<LoadedDocument> {
    match key.node_set {
        NodeSet::Chat => load_chat_nodes(&key.path).await,
        NodeSet::ChatAndTools => {
            let (items, _, parse_errors) = RolloutRecorder::load_rollout_items(&key.path).await?;
            let nodes = nodes_from_rollout(&items);
            let tool_count = nodes.iter().filter(|node| node.kind.is_tool()).count();
            Ok(LoadedDocument::new(nodes, tool_count, parse_errors))
        }
    }
}

async fn load_chat_nodes(path: &Path) -> io::Result<LoadedDocument> {
    let mut reader = open_rollout_line_reader(path).await?;
    let mut nodes = Vec::new();
    let mut tool_count = 0usize;
    let mut parse_errors = 0usize;
    let mut saw_non_empty_line = false;
    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        saw_non_empty_line = true;
        match visible_line(&line) {
            Ok(VisibleLine::Node(node)) => nodes.push(node),
            Ok(VisibleLine::Tool) => tool_count = tool_count.saturating_add(1),
            Ok(VisibleLine::Ignored) => {}
            Err(_) => parse_errors = parse_errors.saturating_add(1),
        }
    }
    if !saw_non_empty_line {
        return Err(io::Error::other("empty session file"));
    }
    Ok(LoadedDocument::new(nodes, tool_count, parse_errors))
}

#[cfg(test)]
#[path = "rollout_loader_tests.rs"]
mod tests;
