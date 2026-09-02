use super::*;
use pretty_assertions::assert_eq;
use std::ffi::OsStr;

#[test]
fn resume_command_targets_the_selected_thread_and_folder() {
    let command = resume_command(
        SendTarget {
            thread_id: Some("thread-id"),
            cwd: Some(Path::new("/workspace/project")),
            active: false,
        },
        "-follow up",
    );
    assert_eq!(command.get_program(), OsStr::new("codex"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "exec",
            "--skip-git-repo-check",
            "-C",
            "/workspace/project",
            "resume",
            "thread-id",
            "--",
            "-follow up",
        ]
        .map(OsStr::new)
    );
}
