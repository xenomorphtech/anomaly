use super::ChatApp;
use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::RichText;

impl ChatApp {
    pub(super) fn conversation_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Conversations");
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} most recent", self.conversations.len()))
                    .small()
                    .color(Color32::from_rgb(135, 143, 158)),
            );
            if ui.small_button("Refresh").clicked() {
                self.refresh_conversations();
            }
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        let selected_path = self.path.as_deref();
        let mut selected = None;
        egui::ScrollArea::vertical()
            .id_salt("conversation-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for conversation in &self.conversations {
                    let working = conversation.activity.active;
                    let mut lines = vec![format!(
                        "{}  {}",
                        if working { "●" } else { "○" },
                        compact(&conversation.title, 74)
                    )];
                    if let Some(cwd) = &conversation.cwd {
                        lines.push(format!(
                            "Folder: {}",
                            cwd.file_name().unwrap_or(cwd.as_os_str()).to_string_lossy()
                        ));
                    }
                    let updated_at = conversation
                        .updated_at
                        .as_deref()
                        .map(short_timestamp)
                        .unwrap_or_default();
                    if working {
                        lines.push(format!(
                            "{} tool calls since last user{}",
                            conversation.activity.tool_calls_since_user,
                            if updated_at.is_empty() {
                                String::new()
                            } else {
                                format!("  ·  {updated_at}")
                            }
                        ));
                    } else if !updated_at.is_empty() {
                        lines.push(updated_at);
                    }
                    let response = ui.add_sized(
                        [ui.available_width(), 78.0],
                        egui::Button::new((
                            RichText::new(lines.join("\n")).color(if working {
                                Color32::from_rgb(123, 220, 164)
                            } else {
                                Color32::from_rgb(184, 190, 202)
                            }),
                            egui::Atom::grow(),
                        ))
                        .fill(if selected_path == Some(conversation.path.as_path()) {
                            Color32::from_rgb(38, 49, 65)
                        } else {
                            Color32::from_rgb(24, 28, 37)
                        })
                        .wrap(),
                    );
                    if response.clicked() {
                        selected = Some(conversation.path.clone());
                    }
                    ui.add_space(5.0);
                }
            });
        if let Some(path) = selected {
            self.load(path);
            self.composer.clear_status();
        }
    }
}

fn compact(text: &str, limit: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= limit {
        one_line
    } else {
        format!("{}…", one_line.chars().take(limit).collect::<String>())
    }
}

fn short_timestamp(timestamp: &str) -> String {
    timestamp.get(..16).unwrap_or(timestamp).replace('T', " ")
}
