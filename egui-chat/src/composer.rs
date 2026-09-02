use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::RichText;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;

#[derive(Clone, Copy)]
pub(crate) struct SendTarget<'a> {
    pub(crate) thread_id: Option<&'a str>,
    pub(crate) cwd: Option<&'a Path>,
    pub(crate) active: bool,
}

#[derive(Default)]
pub(crate) struct Composer {
    text: String,
    process: Option<Child>,
    status: Option<String>,
}

impl Composer {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, target: Option<SendTarget<'_>>) {
        let can_send = target.is_some_and(|target| target.thread_id.is_some() && !target.active)
            && self.process.is_none()
            && !self.text.trim().is_empty();
        let editor = ui.add(
            egui::TextEdit::multiline(&mut self.text)
                .hint_text("Send a follow-up to this conversation…")
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .lock_focus(true),
        );
        let shortcut = editor.has_focus()
            && ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if (ui
                .add_enabled(can_send, egui::Button::new("Send"))
                .clicked()
                || (can_send && shortcut))
                && let Some(target) = target
            {
                self.send(target);
            }
            let status = if self.process.is_some() {
                Some("Running follow-up turn…")
            } else if target.is_some_and(|target| target.active) {
                Some("The selected conversation is still working.")
            } else if target.is_some_and(|target| target.thread_id.is_none()) {
                Some("This rollout has no resumable thread id.")
            } else {
                self.status.as_deref()
            };
            if let Some(status) = status {
                ui.label(
                    RichText::new(status)
                        .small()
                        .color(Color32::from_rgb(145, 154, 171)),
                );
            }
        });
    }

    pub(crate) fn clear_status(&mut self) {
        self.status = None;
    }

    pub(crate) fn poll(&mut self) -> bool {
        let Some(child) = self.process.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.status = Some(if status.success() {
                    "Follow-up turn completed.".to_string()
                } else {
                    format!("Codex exited with {status}.")
                });
                self.process = None;
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.status = Some(format!("Could not check Codex: {error}"));
                self.process = None;
                false
            }
        }
    }

    fn send(&mut self, target: SendTarget<'_>) {
        if target.thread_id.is_none() {
            return;
        }
        match resume_command(target, self.text.trim()).spawn() {
            Ok(child) => {
                self.process = Some(child);
                self.status = None;
                self.text.clear();
            }
            Err(error) => self.status = Some(format!("Could not start Codex: {error}")),
        }
    }
}

fn resume_command(target: SendTarget<'_>, message: &str) -> Command {
    let mut command = Command::new("codex");
    command
        .arg("exec")
        .arg("--skip-git-repo-check")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = target.cwd {
        command.arg("-C").arg(cwd);
    }
    command
        .arg("resume")
        .arg(target.thread_id.unwrap_or_default())
        .arg("--")
        .arg(message);
    command
}

#[cfg(test)]
#[path = "composer_tests.rs"]
mod tests;
