use super::*;
use codex_rollout::RolloutRecorder;
use pretty_assertions::assert_eq;
use std::fs;
use std::time::UNIX_EPOCH;

#[test]
fn loads_chat_without_tool_bodies_and_reuses_the_cached_document() {
    let path = temporary_rollout_path();
    fs::write(
        &path,
        concat!(
            "{\"timestamp\":\"2026-07-25T10:00:00Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}}\n",
            "{\"timestamp\":\"2026-07-25T10:00:01Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"custom_tool_call_output\",\"call_id\":\"call-1\",",
            "\"output\":\"hidden tool body\"}}\n"
        ),
    )
    .unwrap();
    let _cleanup = RemoveFile(path.clone());
    let mut loader = RolloutLoader::new(egui::Context::default());

    let pending_key = match loader.request(path.clone(), NodeSet::Chat) {
        LoadStart::Pending(key) => key,
        LoadStart::Ready { .. } | LoadStart::Failed { .. } => panic!("expected pending load"),
    };
    let completed = wait_for_completion(&mut loader);
    assert_eq!(completed.key, pending_key);
    let loaded = completed.result.unwrap();
    assert!(matches!(loaded.origin, LoadOrigin::Disk(_)));
    assert_eq!(
        (
            loaded.nodes.as_ref(),
            loaded.tool_count,
            loaded.parse_errors
        ),
        (
            &[ChatNode {
                kind: crate::model::NodeKind::Assistant,
                title: "Assistant".to_string(),
                summary: String::new(),
                body: "done".to_string(),
            }][..],
            1,
            0,
        )
    );

    let cached = match loader.request(path, NodeSet::Chat) {
        LoadStart::Ready { key, rollout } => {
            assert_eq!(key, pending_key);
            rollout
        }
        LoadStart::Pending(_) | LoadStart::Failed { .. } => panic!("expected cache hit"),
    };
    assert_eq!(
        cached,
        LoadedRollout {
            nodes: loaded.nodes,
            tool_count: 1,
            parse_errors: 0,
            origin: LoadOrigin::Cache,
        }
    );
}

fn wait_for_completion(loader: &mut RolloutLoader) -> LoadCompletion {
    let started = Instant::now();
    loop {
        if let Some(completed) = loader.poll().into_iter().next() {
            return completed;
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "rollout load timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn temporary_rollout_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "codex-egui-chat-{}-{nonce}.jsonl",
        std::process::id()
    ))
}

struct RemoveFile(PathBuf);

impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "manual benchmark over a real rollout"]
fn benchmark_rollout_selection() {
    let path = std::env::var_os("CODEX_EGUI_BENCH_ROLLOUT")
        .map(PathBuf::from)
        .expect("set CODEX_EGUI_BENCH_ROLLOUT");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let baseline_started = Instant::now();
    let (items, _, baseline_parse_errors) = runtime
        .block_on(RolloutRecorder::load_rollout_items(&path))
        .expect("load rollout");
    let baseline_parsed = Instant::now();
    let all_nodes = nodes_from_rollout(&items);
    let baseline_finished = Instant::now();
    let expected_tool_count = all_nodes.iter().filter(|node| node.kind.is_tool()).count();
    let expected_nodes = all_nodes
        .into_iter()
        .filter(|node| !node.kind.is_tool())
        .collect::<Vec<_>>();

    let optimized_started = Instant::now();
    let optimized = runtime
        .block_on(load_chat_nodes(&path))
        .expect("load visible nodes");
    let optimized_finished = Instant::now();

    assert_eq!(
        (
            optimized.nodes.as_ref(),
            optimized.tool_count,
            optimized.parse_errors,
        ),
        (
            expected_nodes.as_slice(),
            expected_tool_count,
            baseline_parse_errors,
        )
    );
    let mut loader = RolloutLoader::new(egui::Context::default());
    let key = RolloutKey {
        path: path.clone(),
        stamp: FileStamp::read(&path).expect("file stamp"),
        node_set: NodeSet::Chat,
    };
    loader.insert(key, optimized);
    let cache_started = Instant::now();
    let cached = loader.request(path.clone(), NodeSet::Chat);
    let cache_finished = Instant::now();
    assert!(matches!(
        cached,
        LoadStart::Ready {
            rollout: LoadedRollout {
                origin: LoadOrigin::Cache,
                ..
            },
            ..
        }
    ));
    eprintln!(
        "{} bytes: baseline parse={:?}, nodes={:?}, total={:?}; optimized={:?}; cache={:?}",
        path.metadata().expect("metadata").len(),
        baseline_parsed.duration_since(baseline_started),
        baseline_finished.duration_since(baseline_parsed),
        baseline_finished.duration_since(baseline_started),
        optimized_finished.duration_since(optimized_started),
        cache_finished.duration_since(cache_started),
    );
}
