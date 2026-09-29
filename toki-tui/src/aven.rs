//! Read-only Aven task lookup for the optional note picker.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const MAX_TASKS: usize = 500; // Keep every redraw bounded without silently hiding tasks.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvenTask {
    pub title: String,
    pub reference: Option<String>, // Picker label only; never inserted into the note.
}

pub async fn open_tasks(cwd: &Path) -> Result<Vec<AvenTask>, String> {
    // Never pass a task title through a shell; do not invoke Aven sync or writes.
    lookup("aven", cwd, LOOKUP_TIMEOUT).await
}

async fn lookup(executable: &str, cwd: &Path, deadline: Duration) -> Result<Vec<AvenTask>, String> {
    let mut child = Command::new(executable)
        .args(["list", "--open", "--json"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // May contain private task details or local paths.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Aven not found (is `aven` in PATH?)".to_string()
            } else {
                "Could not start Aven in the selected directory".to_string()
            }
        })?;

    let stdout = child.stdout.take().expect("piped stdout");
    let result = tokio::time::timeout(deadline, async {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Could not read Aven output".to_string())?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err("Aven task list is too large; no tasks were omitted silently".to_string());
        }
        if !child
            .wait()
            .await
            .map_err(|_| "Aven process failed".to_string())?
            .success()
        {
            return Err("Aven could not list tasks in this workspace".to_string());
        }
        parse_tasks(&bytes)
    })
    .await;

    if !matches!(result, Ok(Ok(_))) {
        // Also handles a child blocked writing an oversized result to its pipe.
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result.unwrap_or_else(|_| Err("Aven task lookup timed out".to_string()))
}

fn parse_tasks(bytes: &[u8]) -> Result<Vec<AvenTask>, String> {
    let tasks: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| "Aven returned invalid task JSON".to_string())?;
    if tasks.len() > MAX_TASKS {
        return Err(
            "Aven workspace has too many open tasks; no tasks were omitted silently".to_string(),
        );
    }
    tasks
        .into_iter()
        .map(|task| {
            let title = task
                .get("title")
                .and_then(|title| title.as_str())
                .ok_or_else(|| "Aven task is missing a title".to_string())?;
            // A title is untrusted text: control characters must not reach a terminal or a note.
            let title: String = title
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            if title.trim().is_empty() || title.chars().count() > 300 {
                return Err("Aven task has an empty or oversized title".to_string());
            }
            let reference = match task.get("ref") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(value)) if value.is_empty() => None,
                Some(serde_json::Value::String(value)) => {
                    if value.chars().count() > 80
                        || value
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '[' | ']'))
                    {
                        return Err("Aven task has an invalid reference".to_string());
                    }
                    Some(value.clone())
                }
                _ => return Err("Aven task has an invalid reference".to_string()),
            };
            Ok(AvenTask { title, reference })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ref_for_display_but_keeps_title_separate() {
        let tasks = parse_tasks(br#"[{"title":"One\nTwo", "ref":"CHL-X30D", "project":"SECRET"},{"title":"Three", "id":"SECRET"}]"#).unwrap();
        assert_eq!(
            tasks,
            [
                AvenTask {
                    title: "One Two".to_string(),
                    reference: Some("CHL-X30D".to_string())
                },
                AvenTask {
                    title: "Three".to_string(),
                    reference: None
                },
            ]
        );
    }

    #[test]
    fn malformed_or_missing_title_fails_closed() {
        assert!(parse_tasks(br#"[{"id":2}]"#).is_err());
        assert!(parse_tasks(b"not-json").is_err());
        assert!(parse_tasks(br#"[{"title":"\u001b[31m"}]"#).is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_fixture_proves_read_only_argv_and_bounded_runtime() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "toki-aven-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let executable = dir.join("fake-aven");
        std::fs::write(
            &executable,
            "#!/bin/sh\n[ \"$1 $2 $3\" = 'list --open --json' ] || exit 7\nprintf '[{\"title\":\"Fixture title\"}]'\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let titles = lookup(executable.to_str().unwrap(), &dir, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            titles,
            [AvenTask {
                title: "Fixture title".to_string(),
                reference: None
            }]
        );
        std::fs::write(&executable, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        let timeout = lookup(
            executable.to_str().unwrap(),
            &dir,
            Duration::from_millis(30),
        )
        .await;
        assert_eq!(timeout.unwrap_err(), "Aven task lookup timed out");
        std::fs::remove_file(&executable).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn oversized_list_fails_without_silently_truncating() {
        let tasks = vec![serde_json::json!({"title":"Fixture"}); MAX_TASKS + 1];
        let result = parse_tasks(&serde_json::to_vec(&tasks).unwrap());
        assert!(result.unwrap_err().contains("too many open tasks"));
    }

    #[tokio::test]
    async fn absent_executable_returns_safe_error() {
        let result = lookup(
            "toki-aven-executable-that-does-not-exist",
            Path::new("."),
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(result.unwrap_err(), "Aven not found (is `aven` in PATH?)");
    }
}
