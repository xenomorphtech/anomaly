mod composer;
mod conversation;
mod conversation_panel;
mod model;
mod rollout_loader;
mod rollout_selection;

use codex_rollout::ThreadItem;
use codex_rollout::ThreadSortKey;
use codex_rollout::get_threads;
use codex_utils_home_dir::find_codex_home;
use composer::Composer;
use composer::SendTarget;
use conversation::ActivitySummary;
use conversation::summarize_rollout;
use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::RichText;
use model::ChatNode;
use model::NodeKind;
use rollout_loader::LoadOrigin;
use rollout_loader::RolloutKey;
use rollout_loader::RolloutLoader;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use tokio::runtime::Runtime;

const CONVERSATION_LIMIT: usize = 50;
const CONVERSATION_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 760.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Codex chat nodes",
        native_options,
        Box::new(move |creation_context| {
            Ok(Box::new(ChatApp::new(
                creation_context,
                runtime,
                initial_path,
            )))
        }),
    )?;
    Ok(())
}

struct ChatApp {
    runtime: Runtime,
    codex_home: Option<PathBuf>,
    path: Option<PathBuf>,
    requested_rollout: Option<RolloutKey>,
    nodes: Arc<[ChatNode]>,
    tool_count: usize,
    rollout_loader: RolloutLoader,
    loading: bool,
    load_origin: Option<LoadOrigin>,
    conversations: Vec<Conversation>,
    activity_cache: HashMap<PathBuf, CachedActivity>,
    last_conversation_refresh: Instant,
    composer: Composer,
    parse_errors: usize,
    error: Option<String>,
    show_tools: bool,
}

struct Conversation {
    path: PathBuf,
    thread_id: Option<String>,
    title: String,
    cwd: Option<PathBuf>,
    updated_at: Option<String>,
    activity: ActivitySummary,
}

#[derive(Clone, Copy)]
struct CachedActivity {
    modified_at: Option<SystemTime>,
    summary: ActivitySummary,
}

impl ChatApp {
    fn new(
        creation_context: &eframe::CreationContext<'_>,
        runtime: Runtime,
        initial_path: Option<PathBuf>,
    ) -> Self {
        creation_context.egui_ctx.set_visuals(egui::Visuals::dark());
        creation_context
            .egui_ctx
            .style_mut_of(egui::Theme::Dark, |style| {
                style.spacing.item_spacing = egui::vec2(8.0, 8.0);
                style.visuals.panel_fill = Color32::from_rgb(17, 20, 27);
                style.visuals.extreme_bg_color = Color32::from_rgb(24, 28, 37);
                style.visuals.selection.bg_fill = Color32::from_rgb(46, 94, 154);
            });
        let codex_home = find_codex_home().ok().map(|path| path.to_path_buf());
        let rollout_loader = RolloutLoader::new(creation_context.egui_ctx.clone());
        let mut app = Self {
            runtime,
            codex_home,
            path: None,
            requested_rollout: None,
            nodes: Arc::default(),
            tool_count: 0,
            rollout_loader,
            loading: false,
            load_origin: None,
            conversations: Vec::new(),
            activity_cache: HashMap::new(),
            last_conversation_refresh: Instant::now(),
            composer: Composer::default(),
            parse_errors: 0,
            error: None,
            show_tools: false,
        };
        app.refresh_conversations();
        match initial_path.or_else(|| {
            app.conversations
                .first()
                .map(|conversation| conversation.path.clone())
        }) {
            Some(path) => app.load(path),
            None => {
                app.error = Some("No Codex rollout found. Drop a rollout file here.".to_string());
            }
        }
        app
    }

    fn refresh_conversations(&mut self) {
        let Some(codex_home) = self.codex_home.as_deref() else {
            return;
        };
        let page = self.runtime.block_on(get_threads(
            codex_home,
            CONVERSATION_LIMIT,
            /*cursor*/ None,
            ThreadSortKey::UpdatedAt,
            &[],
            /*model_providers*/ None,
            /*cwd_filters*/ None,
            "openai",
        ));
        let Ok(page) = page else {
            return;
        };
        let mut conversations = Vec::with_capacity(page.items.len());
        for item in page.items {
            conversations.push(self.conversation(item));
        }
        self.activity_cache.retain(|path, _| {
            conversations
                .iter()
                .any(|conversation| conversation.path == *path)
        });
        self.conversations = conversations;
        self.last_conversation_refresh = Instant::now();
    }

    fn conversation(&mut self, item: ThreadItem) -> Conversation {
        let modified_at = item
            .path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok();
        let activity = self
            .activity_cache
            .get(&item.path)
            .filter(|cached| cached.modified_at == modified_at)
            .map(|cached| cached.summary)
            .unwrap_or_else(|| {
                let summary = self
                    .runtime
                    .block_on(summarize_rollout(&item.path))
                    .unwrap_or_default();
                self.activity_cache.insert(
                    item.path.clone(),
                    CachedActivity {
                        modified_at,
                        summary,
                    },
                );
                summary
            });
        let title = item
            .preview
            .or(item.first_user_message)
            .filter(|title| !title.trim().is_empty())
            .or(item.agent_nickname)
            .unwrap_or_else(|| {
                item.path
                    .file_name()
                    .map_or_else(
                        || "Untitled conversation".into(),
                        |name| name.to_string_lossy(),
                    )
                    .into_owned()
            });
        Conversation {
            path: item.path,
            thread_id: item.thread_id.map(|thread_id| thread_id.to_string()),
            title,
            cwd: item.cwd,
            updated_at: item.updated_at,
            activity,
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let chat_count = self
            .nodes
            .iter()
            .filter(|node| !node.kind.is_tool())
            .count();
        let mut tools_changed = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Reload").clicked()
                && let Some(path) = self.path.clone()
            {
                self.rollout_loader.invalidate(&path);
                self.load(path);
            }
            ui.separator();
            tools_changed = ui
                .checkbox(
                    &mut self.show_tools,
                    format!("Show tools ({})", self.tool_count),
                )
                .changed();
            ui.separator();
            ui.label(
                RichText::new(format!("{chat_count} chat nodes"))
                    .color(Color32::from_rgb(160, 170, 188)),
            );
            if !self.show_tools && self.tool_count > 0 {
                ui.label(
                    RichText::new(format!("{} hidden", self.tool_count))
                        .small()
                        .color(Color32::from_rgb(135, 143, 158)),
                );
            }
            if self.loading {
                ui.spinner();
                ui.label(
                    RichText::new("Loading…")
                        .small()
                        .color(Color32::from_rgb(135, 143, 158)),
                );
            } else if let Some(origin) = &self.load_origin {
                let label = match origin {
                    LoadOrigin::Cache => "cached".to_string(),
                    LoadOrigin::Disk(elapsed) => format!("loaded in {elapsed:.0?}"),
                };
                ui.label(
                    RichText::new(label)
                        .small()
                        .color(Color32::from_rgb(135, 143, 158)),
                );
            }
            if self.parse_errors > 0 {
                ui.separator();
                ui.label(
                    RichText::new(format!("{} skipped lines", self.parse_errors))
                        .color(Color32::from_rgb(235, 179, 96)),
                );
            }
        });
        if tools_changed && let Some(path) = self.path.clone() {
            self.load(path);
        }
        if let Some(path) = &self.path {
            ui.label(
                RichText::new(path.display().to_string())
                    .small()
                    .color(Color32::from_rgb(135, 143, 158)),
            );
        }
    }

    fn composer(&mut self, ui: &mut egui::Ui) {
        let Self {
            path,
            conversations,
            composer,
            ..
        } = self;
        let target = path.as_deref().and_then(|path| {
            conversations
                .iter()
                .find(|conversation| conversation.path == path)
                .map(|conversation| SendTarget {
                    thread_id: conversation.thread_id.as_deref(),
                    cwd: conversation.cwd.as_deref(),
                    active: conversation.activity.active,
                })
        });
        composer.ui(ui, target);
    }

    fn timeline(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for (index, node) in self.nodes.iter().enumerate() {
                    if node.kind.is_tool() && !self.show_tools {
                        continue;
                    }
                    render_node(ui, index, node);
                    ui.add_space(10.0);
                }
                if self.nodes.is_empty() && self.error.is_none() {
                    ui.centered_and_justified(|ui| {
                        ui.label(if self.loading {
                            "Loading conversation…"
                        } else {
                            "This rollout has no displayable chat nodes."
                        });
                    });
                }
            });
    }
}

impl eframe::App for ChatApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_rollout_loader();
        self.reload_if_changed();
        if self.composer.poll() {
            self.refresh_conversations();
        }
        if self.last_conversation_refresh.elapsed() >= CONVERSATION_REFRESH_INTERVAL {
            self.refresh_conversations();
        }
        context.request_repaint_after(if self.loading {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(1)
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dropped_path = ui.ctx().input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        });
        if let Some(path) = dropped_path {
            self.load(path);
        }

        egui::Panel::left("conversations")
            .default_size(310.0)
            .min_size(250.0)
            .max_size(430.0)
            .resizable(true)
            .show(ui, |ui| self.conversation_panel(ui));
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("Codex chat nodes");
            ui.label(
                RichText::new("Each chat item is a node. Tool activity starts hidden.")
                    .color(Color32::from_rgb(145, 154, 171)),
            );
            ui.add_space(10.0);
            self.toolbar(ui);
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(Color32::from_rgb(255, 185, 185)));
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            egui::Panel::bottom("composer")
                .resizable(false)
                .show(ui, |ui| self.composer(ui));
            self.timeline(ui);
        });
    }
}

fn render_node(ui: &mut egui::Ui, index: usize, node: &ChatNode) {
    let accent = node_color(node.kind);
    ui.horizontal_top(|ui| {
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.add_space(13.0);
            ui.label(RichText::new("●").color(accent).size(13.0));
        });
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            let frame = egui::Frame::new()
                .fill(Color32::from_rgb(27, 31, 40))
                .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.55)))
                .corner_radius(9)
                .inner_margin(12);
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                if node.kind.is_tool() {
                    let header = if node.summary.is_empty() {
                        node.title.clone()
                    } else {
                        format!("{}  ·  {}", node.title, node.summary)
                    };
                    egui::CollapsingHeader::new(RichText::new(header).color(accent).strong())
                        .id_salt(("tool-node", index))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(5.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&node.body)
                                        .monospace()
                                        .color(Color32::from_rgb(207, 211, 220)),
                                )
                                .wrap(),
                            );
                        });
                } else {
                    ui.label(RichText::new(&node.title).color(accent).strong());
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(&node.body).size(15.0)).wrap());
                }
            });
        });
    });
}

fn node_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::User => Color32::from_rgb(105, 166, 255),
        NodeKind::Assistant => Color32::from_rgb(102, 211, 153),
        NodeKind::Agent => Color32::from_rgb(192, 132, 252),
        NodeKind::Reasoning => Color32::from_rgb(154, 164, 181),
        NodeKind::Tool => Color32::from_rgb(235, 179, 96),
        NodeKind::Other => Color32::from_rgb(129, 201, 190),
    }
}
