//! Real process termination after a persistence acknowledgement. The helper deliberately
//! retains its live SQLite connection; unlike an orderly drop, SIGKILL cannot checkpoint
//! the connection or run Rust destructors.
use booknook_core::model::{InlineStyle, State};
use booknook_native::Desk;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const TEXT: &str = "A café 🦊 日本語\nThe latest acknowledged draft.";
const ACK: &str = "BOOKNOOK_WRITER_ACK";

fn styles() -> Vec<InlineStyle> {
    vec![InlineStyle {
        start_utf16: 7,
        length_utf16: 2,
        bold: true,
        italic: true,
        underline: false,
    }]
}

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("booknook-kill-{}", booknook_core::new_id())))
    }
    fn open(&self) -> Desk {
        Desk::new(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Writer(Child);
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "subprocess fixture; invoked explicitly by the two crash recovery tests"]
fn acknowledged_writer_process_helper() {
    let directory = std::env::var("BOOKNOOK_CRASH_TEST_DIRECTORY").unwrap();
    let phase = std::env::var("BOOKNOOK_CRASH_TEST_PHASE").unwrap();
    assert!(phase == "staged" || phase == "flushed");
    let desk = Desk::new(directory).unwrap();
    desk.execute(
        serde_json::json!({"op":"add_book", "title":"Crash fixture", "author":"Fixture", "at":1})
            .to_string(),
    )
    .unwrap();
    let initial: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    let notebook = initial.notebooks.values().next().unwrap();
    for (sequence, text) in [(1, "superseded draft"), (2, TEXT)] {
        desk.stage_text_edit(
            serde_json::json!({
                "notebook_id":notebook.id, "block_id":notebook.blocks[0].id,
                "session_id":"killed-writer", "sequence":sequence,
                "base_update":notebook.blocks[0].update, "text":text,
                "styles":if sequence == 2 { styles() } else { vec![] }, "at":sequence+1
            })
            .to_string(),
        )
        .unwrap();
    }
    if phase == "flushed" {
        desk.flush_text_edits().unwrap();
    }
    // The parent only kills us after the durable API call and this explicit flush.
    println!("{ACK}");
    std::io::stdout().flush().unwrap();
    let mut byte = [0u8; 1];
    std::io::stdin().read_exact(&mut byte).unwrap();
    panic!("The parent must kill this live writer, not allow it to exit normally");
}

fn terminate_acknowledged_writer(phase: &str) {
    let root = Root::new();
    let mut writer = Writer(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "acknowledged_writer_process_helper",
                "--nocapture",
            ])
            .env("BOOKNOOK_CRASH_TEST_DIRECTORY", &root.0)
            .env("BOOKNOOK_CRASH_TEST_PHASE", phase)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let output = writer.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if line.unwrap() == ACK {
                send.send(()).unwrap();
                return;
            }
        }
    });
    receive
        .recv_timeout(Duration::from_secs(20))
        .expect("writer must acknowledge the actual stage/flush operation");
    assert!(writer.0.try_wait().unwrap().is_none());
    writer.0.kill().unwrap();
    let status = writer.0.wait().unwrap();
    assert!(!status.success());
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "writer must die by SIGKILL");
    }

    // Inspect before Desk::new can recover. This distinguishes the two actual crash phases.
    let sql = rusqlite::Connection::open(root.0.join("booknook.sqlite")).unwrap();
    let pending: i64 = sql
        .query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r.get(0))
        .unwrap();
    let raw: String = sql
        .query_row("SELECT json FROM desk WHERE singleton=1", [], |r| r.get(0))
        .unwrap();
    let before: State = serde_json::from_str(&raw).unwrap();
    let block = &before.notebooks.values().next().unwrap().blocks[0];
    assert_eq!(pending, i64::from(phase == "staged"));
    assert_eq!(block.text, if phase == "staged" { "" } else { TEXT });
    drop(sql);

    let recovered = root.open();
    let after: State = serde_json::from_str(&recovered.snapshot().unwrap()).unwrap();
    let notebook = after.notebooks.values().next().unwrap();
    assert_eq!(notebook.blocks[0].text, TEXT);
    assert_eq!(notebook.blocks[0].styles, styles());
    assert_eq!(
        notebook
            .revisions
            .iter()
            .filter(|r| booknook_core::notebook::text(&r.update).unwrap() == TEXT)
            .count(),
        1,
        "recovery records the latest acknowledged edit exactly once"
    );
    drop(recovered);
    let reopened = root.open();
    assert_eq!(
        serde_json::from_str::<State>(&reopened.snapshot().unwrap()).unwrap(),
        after,
        "opening again cannot replay the recovered intent"
    );
    let sql = rusqlite::Connection::open(root.0.join("booknook.sqlite")).unwrap();
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn sigkill_after_journal_ack_recovers_latest_unicode_text_and_styles_once() {
    terminate_acknowledged_writer("staged");
}

#[test]
fn sigkill_after_flush_ack_retains_committed_unicode_text_and_styles_once() {
    terminate_acknowledged_writer("flushed");
}
