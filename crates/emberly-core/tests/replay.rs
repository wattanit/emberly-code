//! Replay tests over **committed** transcript fixtures (Phase 5 group 10;
//! Requirements A-2, §8.2; Tech Spec §3.3, §14.2). Each `.jsonl` under
//! `tests/fixtures/transcripts/` is a recorded session; these tests read it
//! back the way `emberly resume` does and assert the rebuilt conversation view,
//! the interrupted/clean classification, and — for the forward-compat fixture —
//! that an unknown event type or a newer schema is **warned and skipped, never
//! a crash**.
//!
//! Fixtures are ground-truth regression anchors: if the schema or the rebuild
//! logic changes in a way that breaks reading an older transcript, one of these
//! fails. No `.unwrap()`/`.expect()` — setup `panic!`s with context.

use std::path::PathBuf;

use emberly_core::resume::{
    interrupted, read_records, rebuild_conversation, session_id, session_meta, Loaded,
};
use emberly_core::Message;
use emberly_providers::{ContentBlock, Role};

/// Load a fixture by file stem from `tests/fixtures/transcripts/`.
fn load(name: &str) -> Loaded {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts")
        .join(format!("{name}.jsonl"));
    match read_records(&path) {
        Ok(loaded) => loaded,
        Err(error) => panic!("read fixture {}: {error}", path.display()),
    }
}

/// The assistant text of a message, if it leads with a text block.
fn assistant_text(message: &Message) -> Option<&str> {
    match message.content.first() {
        Some(ContentBlock::Text { text }) => Some(text.as_str()),
        _ => None,
    }
}

#[test]
fn normal_session_rebuilds_the_full_conversation() {
    let loaded = load("normal");
    assert!(loaded.warnings.is_empty(), "a clean fixture warns nothing");
    assert!(!interrupted(&loaded.records), "it ended cleanly");
    assert_eq!(
        session_meta(&loaded.records),
        Some(("anthropic".into(), "claude-opus-4-8".into()))
    );
    assert!(session_id(&loaded.records).is_some());

    let view = rebuild_conversation(&loaded.records);
    // user, assistant(text+tool_use), tool_result, assistant(text+tool_use),
    // tool_result, assistant — session_start/title/end are not model-visible.
    assert_eq!(view.len(), 6, "six model-visible turns");
    assert_eq!(view[0].role, Role::User);
    assert_eq!(view[1].role, Role::Assistant);
    assert_eq!(view[1].content.len(), 2, "assistant text + one tool_use");
    assert!(matches!(
        &view[1].content[1],
        ContentBlock::ToolUse { name, .. } if name == "bash"
    ));
    assert_eq!(view[2].role, Role::Tool);
    assert_eq!(view[3].role, Role::Assistant);
    assert!(matches!(
        &view[3].content[1],
        ContentBlock::ToolUse { name, .. } if name == "read_file"
    ));
    assert_eq!(view[4].role, Role::Tool);
    assert_eq!(
        assistant_text(&view[5]),
        Some("Done — it's a package named proj.")
    );
}

#[test]
fn compacted_session_collapses_the_middle_and_keeps_the_tail() {
    let loaded = load("compacted");
    assert!(loaded.warnings.is_empty());
    assert!(!interrupted(&loaded.records));

    let view = rebuild_conversation(&loaded.records);
    // [task] [summary] [third/recent] [continue] [continuing]
    assert_eq!(view.len(), 5);
    assert_eq!(assistant_text(&view[0]), Some("Refactor the parser module"));
    assert_eq!(
        assistant_text(&view[1]),
        Some("Summary so far: surveyed the parser and extracted the tokenizer.")
    );
    assert_eq!(
        assistant_text(&view[2]),
        Some("Third pass: the most recent change.")
    );
    assert_eq!(
        assistant_text(&view[3]),
        Some("Continue with the error types")
    );
    assert_eq!(
        assistant_text(&view[4]),
        Some("Continuing with the error types.")
    );
    // The two summarized-away middle turns must not survive in the view.
    assert!(
        !view
            .iter()
            .any(|m| assistant_text(m) == Some("First pass: surveyed the parser.")),
        "the compacted middle is gone from the rebuilt view"
    );
}

#[test]
fn abnormally_exited_session_is_interrupted_but_still_rebuilds() {
    let loaded = load("abnormal_exit");
    assert!(loaded.warnings.is_empty());
    assert!(
        interrupted(&loaded.records),
        "an abnormal_exit (no clean session_end) offers resume"
    );

    let view = rebuild_conversation(&loaded.records);
    // user, assistant(text+tool_use), tool_result(failing build), assistant.
    assert_eq!(view.len(), 4);
    assert_eq!(view[0].role, Role::User);
    assert_eq!(view[1].role, Role::Assistant);
    assert_eq!(view[2].role, Role::Tool);
    assert!(matches!(
        &view[2].content.first(),
        Some(ContentBlock::ToolResult { is_error, .. }) if *is_error
    ));
    assert_eq!(
        assistant_text(&view[3]),
        Some("The build failed; investigating the unresolved import.")
    );
}

#[test]
fn forward_compat_fixture_warns_and_skips_never_crashes() {
    let loaded = load("forward_compat");
    // The two unreadable lines (newer schema + unknown type) are warned...
    assert_eq!(
        loaded.warnings.len(),
        2,
        "both bad lines warned: {:?}",
        loaded.warnings
    );
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("newer transcript schema")),
        "the v9999 line is reported as a newer schema: {:?}",
        loaded.warnings
    );
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("unreadable record")),
        "the unknown-type line is reported as unreadable: {:?}",
        loaded.warnings
    );
    // ...and the good records around them survive and rebuild.
    let view = rebuild_conversation(&loaded.records);
    assert_eq!(view.len(), 2, "user + assistant survive the skipped lines");
    assert_eq!(view[0].role, Role::User);
    assert_eq!(
        assistant_text(&view[1]),
        Some("I skipped what I could not understand and kept going.")
    );
}
