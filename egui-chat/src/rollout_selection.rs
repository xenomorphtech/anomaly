use super::ChatApp;
use crate::rollout_loader::FileStamp;
use crate::rollout_loader::LoadStart;
use crate::rollout_loader::LoadedRollout;
use crate::rollout_loader::NodeSet;
use crate::rollout_loader::RolloutKey;
use std::path::PathBuf;
use std::sync::Arc;

impl ChatApp {
    pub(super) fn load(&mut self, path: PathBuf) {
        let node_set = if self.show_tools {
            NodeSet::ChatAndTools
        } else {
            NodeSet::Chat
        };
        let changed_conversation = self.path.as_deref() != Some(path.as_path());
        self.path = Some(path.clone());
        match self.rollout_loader.request(path, node_set) {
            LoadStart::Ready { key, rollout } => self.apply_loaded_rollout(key, rollout),
            LoadStart::Pending(key) => {
                if changed_conversation {
                    self.nodes = Arc::default();
                    self.tool_count = 0;
                    self.parse_errors = 0;
                }
                self.requested_rollout = Some(key);
                self.loading = true;
                self.load_origin = None;
                self.error = None;
            }
            LoadStart::Failed { path, error } => {
                self.requested_rollout = None;
                self.loading = false;
                self.error = Some(format!("Could not open {}: {error}", path.display()));
            }
        }
    }

    fn apply_loaded_rollout(&mut self, key: RolloutKey, rollout: LoadedRollout) {
        self.path = Some(key.path().to_path_buf());
        self.requested_rollout = Some(key);
        self.nodes = rollout.nodes;
        self.tool_count = rollout.tool_count;
        self.parse_errors = rollout.parse_errors;
        self.loading = false;
        self.load_origin = Some(rollout.origin);
        self.error = None;
    }

    pub(super) fn poll_rollout_loader(&mut self) {
        for completion in self.rollout_loader.poll() {
            if self.requested_rollout.as_ref() != Some(&completion.key) {
                continue;
            }
            match completion.result {
                Ok(rollout) => self.apply_loaded_rollout(completion.key, rollout),
                Err(error) => {
                    self.loading = false;
                    self.error = Some(format!(
                        "Could not open {}: {error}",
                        completion.key.path().display()
                    ));
                }
            }
        }
    }

    pub(super) fn reload_if_changed(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let Ok(stamp) = FileStamp::read(&path) else {
            return;
        };
        if self
            .requested_rollout
            .as_ref()
            .is_none_or(|requested| requested.stamp() != stamp)
        {
            self.load(path);
        }
    }
}
