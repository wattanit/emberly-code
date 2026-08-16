//! Tests for the `App` view model.

use super::*;
use emberly_core::ToolCallId;

fn app() -> App {
    App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        vec!["anthropic".into(), "openai".into(), "zai".into()],
        "# test config\n".to_string(),
        test_provider_writer(),
    )
}

#[test]
fn slash_model_switches_provider_and_model() {
    let mut a = app();
    assert_eq!(
        a.run_slash("model zai glm-4.6"),
        Action::Command(Command::SwitchModel {
            profile: "zai".into(),
            model: Some("glm-4.6".into()),
        })
    );
    // Profile only → keep-current-model (None).
    assert_eq!(
        a.run_slash("model openai"),
        Action::Command(Command::SwitchModel {
            profile: "openai".into(),
            model: None,
        })
    );
}

#[test]
fn slash_model_without_args_opens_the_picker() {
    let mut a = app();
    assert_eq!(a.run_slash("model"), Action::None);
    assert!(
        matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Choices {
                kind: ChoiceKind::Model,
                ..
            })
        ),
        "no-arg /model opens the model picker"
    );
}

#[test]
fn model_picker_marks_current_and_enter_switches() {
    let mut a = app();
    a.session.provider = "anthropic".into();
    a.open_model_picker();
    // The active profile is preselected and marked current.
    let Some(OverlayContent::Choices { rows, selected, .. }) =
        a.overlays.last().map(|o| &o.content)
    else {
        panic!("expected a choices overlay");
    };
    assert!(rows[*selected].current && rows[*selected].label == "anthropic");
    // Move to a different profile and press Enter → SwitchModel (keep model).
    let down = KeyEvent::from(KeyCode::Down);
    let _ = a.on_choice_picker_key(down);
    let enter = KeyEvent::from(KeyCode::Enter);
    assert!(matches!(
        a.on_choice_picker_key(enter),
        Action::Command(Command::SwitchModel { model: None, .. })
    ));
}

#[test]
fn model_picker_offers_add_provider_even_with_no_profiles() {
    // Requirements C-7: the picker is the entry point for guided setup, so
    // it must still open (with just the trailing row) when nothing is
    // configured yet — the exact moment a user most needs it.
    let mut a = App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        Vec::new(),
        String::new(),
        test_provider_writer(),
    );
    a.open_model_picker();
    let Some(Overlay {
        content: OverlayContent::Choices { rows, .. },
        ..
    }) = a.overlays.last()
    else {
        panic!("expected a choices overlay");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, ADD_PROVIDER_ROW);
}

/// Type each character of `text` into the active wizard step's editor.
fn type_str(a: &mut App, text: &str) {
    for c in text.chars() {
        let _ = a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
}

#[test]
fn provider_wizard_happy_path_writes_and_fires_reload() {
    let mut a = App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        vec!["anthropic".into()],
        String::new(),
        test_provider_writer(),
    );
    a.open_model_picker();
    // rows: [anthropic, "+ add new provider…"] — move to the trailing row.
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Down));
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
    assert!(a.wizard.pending.is_some());

    type_str(&mut a, "deepseek");
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // Name -> Adapter
    let _ = a.on_key(KeyEvent::from(KeyCode::Down)); // anthropic -> openai
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // Adapter -> Endpoint
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // accept the default endpoint
    type_str(&mut a, "deepseek-chat");
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // ModelId -> ApiKey
    type_str(&mut a, "sk-test");
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // ApiKey -> Summary
    let action = a.on_key(KeyEvent::from(KeyCode::Enter)); // write

    assert!(matches!(action, Action::Command(Command::ReloadConfig)));
    assert!(a.wizard.pending.is_none());
    assert_eq!(
        a.wizard.created_models.get("deepseek").map(String::as_str),
        Some("deepseek-chat")
    );
}

#[test]
fn provider_wizard_rejects_empty_name_and_stays_on_step() {
    let mut a = App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        Vec::new(),
        String::new(),
        test_provider_writer(),
    );
    a.open_model_picker();
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::None);
    let Some(wizard) = a.wizard.pending.as_ref() else {
        panic!("wizard should still be open");
    };
    assert_eq!(wizard.step, WizardStep::Name);
}

#[test]
fn provider_wizard_esc_steps_back_without_losing_the_value() {
    let mut a = App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        Vec::new(),
        String::new(),
        test_provider_writer(),
    );
    a.open_model_picker();
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
    type_str(&mut a, "zai2");
    let _ = a.on_key(KeyEvent::from(KeyCode::Enter)); // Name -> Adapter
    let Some(wizard) = a.wizard.pending.as_ref() else {
        panic!("wizard should still be open");
    };
    assert_eq!(wizard.step, WizardStep::Adapter);
    let _ = a.on_key(KeyEvent::from(KeyCode::Esc)); // Adapter -> Name
    let Some(wizard) = a.wizard.pending.as_ref() else {
        panic!("wizard should still be open");
    };
    assert_eq!(wizard.step, WizardStep::Name);
    assert_eq!(wizard.editor.text(), "zai2");
}

#[test]
fn provider_wizard_rejects_a_name_already_in_use() {
    let mut a = App::new(
        SessionInfo::default(),
        std::env::temp_dir(),
        vec!["anthropic".into()],
        String::new(),
        test_provider_writer(),
    );
    a.open_model_picker();
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Down));
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
    type_str(&mut a, "anthropic");
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::None);
    let Some(wizard) = a.wizard.pending.as_ref() else {
        panic!("wizard should still be open");
    };
    assert_eq!(wizard.step, WizardStep::Name);
    assert!(wizard.error.is_some());
}

#[test]
fn slash_modelx_is_not_the_model_command() {
    // A command whose name merely starts with "model" is not `/model`.
    let mut a = app();
    assert_eq!(a.run_slash("modelx"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown command"))));
}

#[test]
fn every_registry_name_is_reachable_as_a_slash_command() {
    // The rich half of the parity check (line mode has the other): every
    // name in the registry resolves, so no command is palette-only by
    // accident. Both frontends parse through `commands::parse_slash`.
    for spec in commands::COMMANDS {
        let mut a = app();
        a.run_slash(spec.name);
        assert!(
            !a.timeline
                .items
                .iter()
                .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown command"))),
            "/{} was not recognized",
            spec.name
        );
    }
}

#[test]
fn slash_mode_without_args_opens_the_picker() {
    let mut a = app();
    assert_eq!(a.run_slash("mode"), Action::None);
    assert!(
        matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Choices {
                kind: ChoiceKind::Mode,
                ..
            })
        ),
        "no-arg /mode opens the mode picker"
    );
}

#[test]
fn mode_picker_marks_current_and_marks_auto_unavailable_without_sandbox() {
    let mut a = app(); // sandbox: None → auto tiers unavailable
    a.open_mode_picker();
    let Some(OverlayContent::Choices { rows, selected, .. }) =
        a.overlays.last().map(|o| &o.content)
    else {
        panic!("expected a choices overlay");
    };
    // Normal is current (the default) and preselected.
    assert!(rows[*selected].current);
    assert_eq!(rows[*selected].label, "normal");
    // Auto tiers are annotated as needing confinement.
    assert!(rows
        .iter()
        .any(|r| r.label.starts_with("auto-accept-edits") && r.label.contains("OS confinement")));
    assert!(rows
        .iter()
        .any(|r| r.label == "auto  (needs OS confinement)" || r.label.starts_with("auto  (")));
}

#[test]
fn mode_picker_switches_to_normal_on_enter() {
    let mut a = app();
    a.mode = Mode::Auto;
    a.sandbox = Some(SandboxStatus::Confined {
        backend: "landlock".into(),
    });
    a.open_mode_picker();
    // Current is Auto; move up to AutoAcceptEdits, then up to Normal.
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Up));
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Up));
    assert!(matches!(
        a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter)),
        Action::Command(Command::SetMode { mode: Mode::Normal })
    ));
}

#[test]
fn mode_picker_refuses_auto_without_confinement() {
    let mut a = app(); // sandbox: None
    a.open_mode_picker();
    // The auto tier row carries the unavailability suffix; selecting it and
    // pressing Enter declines with a notice rather than emitting SetMode.
    // Move down to the auto-accept-edits row (index 1).
    let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Down));
    let action = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::None);
    assert!(a.timeline.items.iter().any(|i| matches!(
        i,
        ConvItem::Notice(n) if n.contains("unavailable without OS confinement")
    )));
}

#[test]
fn slash_mode_arg_sets_directly_and_validates() {
    let mut a = app();
    a.sandbox = Some(SandboxStatus::Confined {
        backend: "landlock".into(),
    });
    // A direct arg with confinement active → SetMode.
    assert_eq!(
        a.run_slash("mode auto"),
        Action::Command(Command::SetMode { mode: Mode::Auto })
    );
    // Normal is always allowed.
    assert_eq!(
        a.run_slash("mode normal"),
        Action::Command(Command::SetMode { mode: Mode::Normal })
    );
    // Unknown mode → a notice, not a command.
    assert_eq!(a.run_slash("mode bogus"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown mode"))));
}

#[test]
fn slash_mode_arg_refuses_auto_without_confinement() {
    let mut a = app(); // sandbox: None
    assert_eq!(a.run_slash("mode auto-accept-edits"), Action::None);
    assert!(a.timeline.items.iter().any(|i| matches!(
        i,
        ConvItem::Notice(n) if n.contains("unavailable without OS confinement")
    )));
    // Normal is still allowed without confinement.
    assert_eq!(
        a.run_slash("mode normal"),
        Action::Command(Command::SetMode { mode: Mode::Normal })
    );
}

#[test]
fn slash_config_seeds_then_edits_without_clobbering() {
    let root = std::env::temp_dir().join(format!("emberly-cfgedit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sessions = root.join(".agents").join("sessions");
    std::fs::create_dir_all(&sessions).expect("mkdir");
    let mut a = App::new(
        SessionInfo::default(),
        sessions,
        Vec::new(),
        "# seeded config\n".to_string(),
        test_provider_writer(),
    );

    let cfg = root.join(".agents").join("config.toml");
    let action = a.run_slash("config");
    assert!(cfg.exists(), "config.toml is seeded when absent");
    assert!(matches!(action, Action::EditFile(ref p) if *p == cfg));

    // A second /config must not overwrite the (now user-edited) file.
    std::fs::write(&cfg, "user edits").expect("write");
    let _ = a.run_slash("config");
    assert_eq!(std::fs::read_to_string(&cfg).expect("read"), "user edits");
}

#[test]
fn slash_prompt_seeds_from_default_and_rejects_unknown() {
    let root = std::env::temp_dir().join(format!("emberly-promptedit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sessions = root.join(".agents").join("sessions");
    std::fs::create_dir_all(&sessions).expect("mkdir");
    let mut a = App::new(
        SessionInfo::default(),
        sessions,
        Vec::new(),
        String::new(),
        test_provider_writer(),
    );

    let p = root.join(".agents").join("prompts").join("system.md");
    let action = a.run_slash("prompt system");
    assert!(p.exists(), "prompt seeded from the baked-in default");
    assert!(matches!(action, Action::EditFile(ref pp) if *pp == p));
    assert!(!std::fs::read_to_string(&p).expect("read").is_empty());

    // An unknown prompt name is a notice, not an edit.
    assert_eq!(a.run_slash("prompt bogus"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown prompt"))));
}

#[test]
fn edit_provenance_distinguishes_new_override_from_existing() {
    let root = std::env::temp_dir().join(format!("emberly-prov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sessions = root.join(".agents").join("sessions");
    std::fs::create_dir_all(&sessions).expect("mkdir");
    let mut a = App::new(
        SessionInfo::default(),
        sessions,
        Vec::new(),
        "# t\n".to_string(),
        test_provider_writer(),
    );

    // First edit: no project config yet → seeded, told it overrides defaults.
    a.run_slash("config");
    assert!(matches!(
        a.timeline.items.last(),
        Some(ConvItem::Notice(n)) if n.contains("no project config yet")
    ));
    // Second edit: the file exists → editing an existing project value.
    a.run_slash("config");
    assert!(matches!(
        a.timeline.items.last(),
        Some(ConvItem::Notice(n)) if n.contains("editing your project config")
    ));
}

#[test]
fn note_edit_reports_no_editor() {
    let mut a = app();
    a.note_edit(
        Path::new("/x/config.toml"),
        crate::edit::EditStatus::NoEditor,
    );
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("no editor configured"))));
}

#[test]
fn model_changed_updates_the_sidebar() {
    let mut a = app();
    a.apply_event(UiEvent::ModelChanged {
        provider: "zai".into(),
        model: "glm-4.6".into(),
    });
    assert_eq!(a.session.provider, "zai");
    assert_eq!(a.session.model, "glm-4.6");
}

#[test]
fn assistant_deltas_accumulate_into_one_item() {
    let mut a = app();
    a.apply_event(UiEvent::AssistantDelta { text: "Hel".into() });
    a.apply_event(UiEvent::AssistantDelta { text: "lo".into() });
    a.apply_event(UiEvent::AssistantDone);
    assert_eq!(a.timeline.items, vec![ConvItem::Assistant("Hello".into())]);
    assert!(!a.timeline.streaming);
}

#[test]
fn tool_finished_marks_the_matching_start() {
    let mut a = app();
    let id = ToolCallId::new("c1");
    a.apply_event(UiEvent::ToolStarted {
        call_id: id.clone(),
        tool: "bash".into(),
        summary: "run: ls".into(),
        explanation: None,
    });
    a.apply_event(UiEvent::ToolFinished {
        call_id: id.clone(),
        ok: true,
        summary: "exit 0".into(),
        preview: "hello\nworld".into(),
        untrusted: false,
    });
    match &a.timeline.items[0] {
        ConvItem::Tool {
            done,
            summary,
            result,
            preview,
            ..
        } => {
            assert_eq!(*done, Some(true));
            // The descriptive label is kept; result + preview are separate.
            assert_eq!(summary, "run: ls");
            assert_eq!(result.as_deref(), Some("exit 0"));
            assert_eq!(preview.as_deref(), Some("hello\nworld"));
        }
        other => panic!("expected a tool item, got {other:?}"),
    }
}

#[test]
fn tool_started_carries_the_explanation_onto_the_item() {
    let mut a = app();
    a.apply_event(UiEvent::ToolStarted {
        call_id: ToolCallId::new("c1"),
        tool: "bash".into(),
        summary: "run: sed …".into(),
        explanation: Some("raise the log level".into()),
    });
    match &a.timeline.items[0] {
        ConvItem::Tool { explanation, .. } => {
            assert_eq!(explanation.as_deref(), Some("raise the log level"));
        }
        other => panic!("expected a tool item, got {other:?}"),
    }
}

#[test]
fn file_diff_shows_inline_and_opens_overlay() {
    let mut a = app();
    a.apply_event(UiEvent::FileModified {
        path: "a.rs".into(),
        adds: 1,
        dels: 0,
    });
    a.apply_event(UiEvent::FileDiff {
        path: "a.rs".into(),
        unified: "--- a/a.rs\n+++ b/a.rs\n+x".into(),
    });
    // Inline diff item recorded.
    assert!(matches!(
        a.timeline.items.last(),
        Some(ConvItem::Diff { .. })
    ));
    assert_eq!(a.files.last.as_deref(), Some("a.rs"));
    // Ctrl+O opens the overlay for the most-recent file.
    a.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(a.overlays.len(), 1);
    assert!(matches!(
        a.active_overlay().map(|o| &o.content),
        Some(OverlayContent::Diff(_))
    ));
}

#[test]
fn busy_spinner_spans_the_turn_and_respects_gates() {
    let mut a = app();
    // Submitting a message enters the busy/working state.
    a.on_key(KeyEvent::from(KeyCode::Char('h')));
    a.on_key(KeyEvent::from(KeyCode::Enter));
    assert!(a.anim.busy);
    assert!(a.is_animating(), "spinner runs while working");
    // No elapsed time before the threshold; the glyph cycles on tick.
    assert_eq!(a.spinner_elapsed(), None);
    for _ in 0..ANIM_FPS * ELAPSED_AFTER_SECS {
        a.tick();
    }
    assert_eq!(a.spinner_elapsed(), Some(ELAPSED_AFTER_SECS));
    // A permission prompt freezes motion (that screen is perfectly still).
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(1),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: x".into(),
            detail: "x".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "asks".into(),
            on_behalf_of: None,
        },
    });
    assert!(!a.is_animating(), "no motion during a permission prompt");
    // Answering resumes; TurnEnded stops the spinner.
    a.on_key(KeyEvent::from(KeyCode::Enter)); // deny
    assert!(a.is_animating());
    a.apply_event(UiEvent::TurnEnded);
    assert!(!a.anim.busy);
    assert!(!a.is_animating());
}

#[test]
fn esc_cancels_an_in_flight_turn() {
    let mut a = app();
    // Idle: Esc has nothing to dismiss and no modal is open.
    assert!(matches!(
        a.on_key(KeyEvent::from(KeyCode::Esc)),
        Action::None
    ));
    assert!(!a.anim.busy);

    a.on_key(KeyEvent::from(KeyCode::Char('h')));
    a.on_key(KeyEvent::from(KeyCode::Enter));
    assert!(a.anim.busy);
    let action = a.on_key(KeyEvent::from(KeyCode::Esc));
    assert!(matches!(action, Action::Command(Command::Cancel)));
}

#[test]
fn ctrl_c_cancels_an_in_flight_turn_but_quits_when_idle() {
    let mut a = app();
    // Idle + empty input: unchanged behavior, Ctrl+C quits.
    assert!(matches!(
        a.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Action::Quit
    ));

    a.on_key(KeyEvent::from(KeyCode::Char('h')));
    a.on_key(KeyEvent::from(KeyCode::Enter));
    assert!(a.anim.busy);
    let action = a.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(matches!(action, Action::Command(Command::Cancel)));
}

#[test]
fn motion_off_disables_animation() {
    let mut a = app();
    a.anim.active = false;
    a.anim.busy = true;
    assert!(!a.is_animating(), "motion=false is the off switch");
}

#[test]
fn overlay_ease_animates_then_settles() {
    let mut a = app();
    a.open_text_overlay("t", "body");
    // Idle, but the ease-in makes it animate briefly, and start scaled down.
    assert!(a.is_animating());
    assert!(!a.is_working(), "ease-in is not the working spinner");
    assert!(a.overlay_ease_progress() < 1.0);
    for _ in 0..EASE_FRAMES {
        a.tick();
    }
    assert!((a.overlay_ease_progress() - 1.0).abs() < f32::EPSILON);
    assert!(!a.is_animating(), "settles to a still screen when idle");
}

#[test]
fn sidebar_settle_highlights_only_new_entries() {
    let mut a = app();
    a.apply_event(UiEvent::FileModified {
        path: "a.rs".into(),
        adds: 1,
        dels: 0,
    });
    assert!(a.sidebar_settling(), "a new entry settles in");
    for _ in 0..SETTLE_FRAMES {
        a.tick();
    }
    assert!(!a.sidebar_settling());
    // An update to an existing entry does not re-trigger the settle.
    a.apply_event(UiEvent::FileModified {
        path: "a.rs".into(),
        adds: 2,
        dels: 1,
    });
    assert!(
        !a.sidebar_settling(),
        "updates don't settle, only new entries"
    );
}

#[test]
fn motion_off_skips_transient_effects() {
    let mut a = app();
    a.anim.active = false;
    a.open_text_overlay("t", "body");
    assert!(!a.is_animating());
    assert!(
        (a.overlay_ease_progress() - 1.0).abs() < f32::EPSILON,
        "no ease-in scaling with motion off"
    );
}

#[test]
fn ctrl_j_inserts_a_newline() {
    let mut a = app();
    a.on_key(KeyEvent::from(KeyCode::Char('a')));
    a.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    a.on_key(KeyEvent::from(KeyCode::Char('b')));
    assert_eq!(a.editor.text(), "a\nb");
    assert_eq!(a.editor.line_count(), 2);
}

#[test]
fn shift_tab_cycles_the_auto_accept_mode() {
    let mut a = app();
    // The view starts in Normal; Shift+Tab (BackTab) proposes the next tier.
    // The frontend only proposes — the engine gates it against confinement —
    // so we assert the emitted command, then advance the view as the engine
    // would (ModeChanged) to check the full cycle.
    let step = |a: &mut App, from: emberly_core::Mode, to: emberly_core::Mode| {
        a.mode = from;
        let action = a.on_key(KeyEvent::from(KeyCode::BackTab));
        assert_eq!(action, Action::Command(Command::SetMode { mode: to }));
    };
    step(
        &mut a,
        emberly_core::Mode::Normal,
        emberly_core::Mode::AutoAcceptEdits,
    );
    step(
        &mut a,
        emberly_core::Mode::AutoAcceptEdits,
        emberly_core::Mode::Auto,
    );
    step(&mut a, emberly_core::Mode::Auto, emberly_core::Mode::Normal);
}

#[test]
fn shift_tab_cycles_via_the_cycle_mode_command() {
    // The registry wires `Shift-Tab` to `CycleMode`. `/mode` (slash and
    // palette) is intercepted earlier in `run_slash` to open the picker, so
    // this registry entry now serves only the quick-toggle keybinding.
    assert_eq!(
        crate::commands::by_name("mode").map(|spec| spec.cmd),
        Some(crate::commands::AppCommand::CycleMode)
    );
}

#[test]
fn shift_and_alt_enter_insert_newlines() {
    // Both modifiers newline (where the terminal reports them); plain Enter
    // still submits. Shift is enabled by the kitty protocol at runtime.
    for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
        let mut a = app();
        a.on_key(KeyEvent::from(KeyCode::Char('a')));
        a.on_key(KeyEvent::new(KeyCode::Enter, modifier));
        a.on_key(KeyEvent::from(KeyCode::Char('b')));
        assert_eq!(a.editor.text(), "a\nb", "{modifier:?}+Enter should newline");
    }
}

#[test]
fn session_usage_is_stored() {
    let mut a = app();
    a.apply_event(UiEvent::SessionUsage {
        usage: TokenUsage {
            input: 1200,
            output: 340,
        },
    });
    assert_eq!(a.usage.tokens.input, 1200);
    assert_eq!(a.usage.tokens.output, 340);
}

#[test]
fn wheel_scrolls_conversation_and_routes_to_overlay() {
    let mut a = app();
    a.on_scroll(true); // wheel up → into history
    assert!(a.timeline.scroll > 0);
    a.on_scroll(false);
    assert_eq!(a.timeline.scroll, 0);
    // With an overlay open, the wheel scrolls the overlay, not the history.
    a.open_text_overlay("t", "x");
    a.on_scroll(false);
    assert_eq!(
        a.timeline.scroll, 0,
        "conversation untouched while overlay is up"
    );
    assert!(a.active_overlay().is_some_and(|o| o.scroll > 0));
}

#[test]
fn wheel_routes_to_the_open_palette() {
    let mut a = app();
    // Open the palette (Ctrl+P) — it is modal and takes the wheel first.
    a.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(a.palette.is_some());
    // Wheel down moves the selection down, exactly like the Down key…
    a.on_scroll(false);
    assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(1));
    // …and wheel up moves it back, clamped at the top.
    a.on_scroll(true);
    assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(0));
    a.on_scroll(true);
    assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(0));
    // The wheel never leaks to the conversation while the palette is up.
    assert_eq!(a.timeline.scroll, 0);
}

#[test]
fn click_on_a_palette_row_is_focus_plus_enter() {
    // A click resolves via the hit-map to a row, then does exactly what
    // "arrow to that row + Enter" does — no separate authority (§3.4).
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };

    let mut by_click = app();
    by_click.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    by_click.hit_map.push(region, ClickTarget::PaletteRow(1));
    let click_action = by_click.on_click(0, 0);

    let mut by_key = app();
    by_key.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    by_key.on_key(KeyEvent::from(KeyCode::Down)); // focus row 1
    let key_action = by_key.on_key(KeyEvent::from(KeyCode::Enter));

    assert_eq!(click_action, key_action, "a click is focus + Enter");
    assert!(
        by_click.palette.is_none(),
        "activating a row closes the palette, like Enter"
    );
    assert!(by_key.palette.is_none());
}

#[test]
fn click_on_a_choice_row_is_focus_plus_enter() {
    // Same parity for the model/effort/mode picker rows.
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };

    let mut by_click = app();
    by_click.open_mode_picker(); // rows: Normal(current) / AutoAcceptEdits / Auto
    by_click.hit_map.push(region, ClickTarget::ChoiceRow(2));
    let click_action = by_click.on_click(0, 0);

    let mut by_key = app();
    by_key.open_mode_picker();
    by_key.on_key(KeyEvent::from(KeyCode::Down));
    by_key.on_key(KeyEvent::from(KeyCode::Down)); // focus row 2
    let key_action = by_key.on_key(KeyEvent::from(KeyCode::Enter));

    assert_eq!(
        click_action, key_action,
        "clicking a choice == arrow + Enter"
    );
    assert!(
        by_click.overlays.is_empty(),
        "confirming a choice closes the picker, like Enter"
    );
    assert!(by_key.overlays.is_empty());
}

#[test]
fn a_click_on_nothing_interactive_is_inert() {
    // An empty hit-map (nothing rendered clickable) → no action, no panic.
    let mut a = app();
    assert_eq!(a.on_click(5, 5), Action::None);
}

#[test]
fn click_reasoning_toggle_matches_ctrl_r() {
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };
    let mut a = app();
    a.apply_event(UiEvent::ReasoningDelta { text: "hmm".into() });
    a.apply_event(UiEvent::AssistantDelta { text: "a".into() }); // settle → collapsed
    a.hit_map.push(region, ClickTarget::ReasoningToggle);
    // A click expands the trail — exactly Ctrl+R.
    assert_eq!(a.on_click(0, 0), Action::None);
    assert!(matches!(
        a.timeline.items.first(),
        Some(ConvItem::Reasoning { expanded: true, .. })
    ));
    // A second click collapses it (parity with a second Ctrl+R).
    a.on_click(0, 0);
    assert!(matches!(
        a.timeline.items.first(),
        Some(ConvItem::Reasoning {
            expanded: false,
            ..
        })
    ));
}

#[test]
fn click_memory_row_is_focus_plus_enter() {
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };
    let entries = || {
        (
            vec![mem_summary("Alpha", "a", MemoryScope::User)],
            vec![mem_summary("Proj", "p", MemoryScope::Project)],
        )
    };

    let mut by_click = app();
    let (user, project) = entries();
    by_click.apply_event(UiEvent::MemoryEntries { user, project });
    by_click.hit_map.push(region, ClickTarget::MemoryRow(1)); // project entry
    let click_action = by_click.on_click(0, 0);

    let mut by_key = app();
    let (user, project) = entries();
    by_key.apply_event(UiEvent::MemoryEntries { user, project });
    by_key.on_key(key(KeyCode::Down)); // focus flattened row 1
    let key_action = by_key.on_key(key(KeyCode::Enter));

    assert_eq!(
        click_action, key_action,
        "clicking a memory entry == arrow + Enter"
    );
}

#[test]
fn click_sidebar_sections_match_their_keyboard_actions() {
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };

    // Memory section → the `/memory` command (palette twin, §3.3).
    let mut a = app();
    a.hit_map.push(region, ClickTarget::OpenMemoryInspector);
    let click = a.on_click(0, 0);
    assert_eq!(click, app().run_command(AppCommand::Memory));

    // Skills section → the `/skills` command.
    let mut a = app();
    a.hit_map.push(region, ClickTarget::OpenSkillsInspector);
    let click = a.on_click(0, 0);
    assert_eq!(click, app().run_command(AppCommand::Skills));

    // Modified-files section → Ctrl+O (open the diff overlay). With no diff
    // recorded both are inert, and both open the same overlay when one is.
    let mut by_click = app();
    by_click.hit_map.push(region, ClickTarget::OpenDiff);
    let click = by_click.on_click(0, 0);
    let mut by_key = app();
    let key = by_key.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(click, key, "clicking modified files == Ctrl+O");
    assert_eq!(by_click.overlays.len(), by_key.overlays.len());
}

fn pending_permission_app() -> App {
    let mut a = app();
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(7),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: x".into(),
            detail: "x".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash asks".into(),
            on_behalf_of: None,
        },
    });
    a
}

#[test]
fn click_permission_affordances_match_their_keys() {
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };
    for (choice, code, decision) in [
        (
            PermissionChoice::Allow,
            KeyCode::Char('y'),
            PermissionDecision::AllowOnce,
        ),
        (
            PermissionChoice::Session,
            KeyCode::Char('s'),
            PermissionDecision::AllowForSession,
        ),
        (
            PermissionChoice::Deny,
            KeyCode::Enter,
            PermissionDecision::Deny,
        ),
    ] {
        let mut by_click = pending_permission_app();
        by_click
            .hit_map
            .push(region, ClickTarget::PermissionChoice(choice));
        let click = by_click.on_click(0, 0);

        let mut by_key = pending_permission_app();
        let keyed = by_key.on_key(key(code));

        assert_eq!(click, keyed, "click on {choice:?} == its key");
        assert!(matches!(
            click,
            Action::Command(Command::PermissionAnswer { decision: d, .. }) if d == decision
        ));
    }
}

#[test]
fn help_documents_the_mouse_and_shift_passthrough() {
    let help = help_text();
    assert!(help.contains("Mouse"), "help has a mouse section");
    assert!(
        help.contains("Shift"),
        "help documents Shift for native selection (Design §3.4)"
    );
    assert!(
        help.contains("mouse = false"),
        "help documents the off switch"
    );
}

#[test]
fn a_click_off_the_permission_affordances_never_decides() {
    // The safety invariant (Design §3.4/§5): a click that does not land on
    // an affordance leaves the prompt pending — never a default-approve, no
    // "approve whatever is focused." (Here the hit-map has no affordance
    // region, standing in for a click on the body/header/margin.)
    let mut a = pending_permission_app();
    assert_eq!(a.on_click(5, 5), Action::None);
    assert!(
        a.prompts.permission.is_some(),
        "a click off the affordances must not decide"
    );
    // A Deny click is safe; still no *approval* ever appears without the
    // Allow/Session affordance.
    let region = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 1,
    };
    a.hit_map.push(
        region,
        ClickTarget::PermissionChoice(PermissionChoice::Deny),
    );
    assert!(matches!(
        a.on_click(0, 0),
        Action::Command(Command::PermissionAnswer {
            decision: PermissionDecision::Deny,
            ..
        })
    ));
}

#[test]
fn overlay_scrolls_and_dismisses() {
    let mut a = app();
    a.open_text_overlay("t", "line1\nline2\nline3");
    a.on_key(KeyEvent::from(KeyCode::PageDown));
    assert!(a.active_overlay().is_some_and(|o| o.scroll > 0));
    // Typing does not leak into the editor while an overlay is modal.
    a.on_key(KeyEvent::from(KeyCode::Char('x')));
    assert!(a.editor.is_empty());
    a.on_key(KeyEvent::from(KeyCode::Esc));
    assert!(a.overlays.is_empty());
}

#[test]
fn modified_files_upsert_by_path() {
    let mut a = app();
    a.apply_event(UiEvent::FileModified {
        path: "a.rs".into(),
        adds: 1,
        dels: 0,
    });
    a.apply_event(UiEvent::FileModified {
        path: "a.rs".into(),
        adds: 3,
        dels: 2,
    });
    assert_eq!(a.files.modified.len(), 1);
    assert_eq!(a.files.modified[0].adds, 3);
    assert_eq!(a.files.modified[0].dels, 2);
}

#[test]
fn permission_defaults_to_deny_on_enter() {
    let mut a = app();
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(1),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: rm -rf x".into(),
            detail: "rm -rf x".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash asks".into(),
            on_behalf_of: None,
        },
    });
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::PermissionAnswer {
            id: PermissionId(1),
            decision: PermissionDecision::Deny,
        })
    );
    assert!(a.prompts.permission.is_none());
}

// ---- ask_user question prompt (T-8, Design §5.1) ---------------------

fn ask(a: &mut App, options: &[&str]) {
    a.apply_event(UiEvent::AskUserRequest {
        id: AskId(7),
        question: "which environment?".into(),
        options: options.iter().map(|s| (*s).to_string()).collect(),
    });
}

#[test]
fn ask_enter_never_auto_answers() {
    let mut a = app();
    ask(&mut a, &["dev", "prod"]);
    // Nothing typed, no option chosen: Enter must not answer (Design §5.1).
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::None);
    assert!(a.prompts.ask.is_some(), "the question is still waiting");
}

#[test]
fn ask_esc_declines() {
    let mut a = app();
    ask(&mut a, &["dev", "prod"]);
    let action = a.on_key(KeyEvent::from(KeyCode::Esc));
    assert_eq!(
        action,
        Action::Command(Command::AskUserAnswer {
            id: AskId(7),
            answer: AskAnswer::Declined,
        })
    );
    assert!(a.prompts.ask.is_none());
}

#[test]
fn ask_arrow_then_enter_picks_the_selected_option() {
    let mut a = app();
    ask(&mut a, &["dev", "prod"]);
    a.on_key(KeyEvent::from(KeyCode::Down)); // select "dev" (index 0)
    a.on_key(KeyEvent::from(KeyCode::Down)); // select "prod" (index 1)
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::AskUserAnswer {
            id: AskId(7),
            answer: AskAnswer::Answered("prod".into()),
        })
    );
}

#[test]
fn ask_free_text_submits_and_beats_a_selection() {
    let mut a = app();
    ask(&mut a, &["dev", "prod"]);
    a.on_key(KeyEvent::from(KeyCode::Down)); // highlight an option…
    for c in "staging".chars() {
        a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
    // …but typed text wins on Enter.
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::AskUserAnswer {
            id: AskId(7),
            answer: AskAnswer::Answered("staging".into()),
        })
    );
}

#[test]
fn ask_free_text_works_without_options() {
    let mut a = app();
    ask(&mut a, &[]);
    for c in "yes".chars() {
        a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::AskUserAnswer {
            id: AskId(7),
            answer: AskAnswer::Answered("yes".into()),
        })
    );
}

#[test]
fn ask_prompt_stills_motion() {
    let mut a = app();
    a.anim.busy = true;
    a.anim.active = true;
    assert!(a.is_working(), "working before the question");
    ask(&mut a, &["dev"]);
    assert!(
        !a.is_working(),
        "no spinner while a question is up (Design §6.4)"
    );
    assert!(!a.is_animating());
}

// ---- loop-halt surface (S-5, Design §8.5) ----------------------------

fn halt(a: &mut App) {
    a.apply_event(UiEvent::LoopHalted {
        reason: "the last few steps repeated without progress".into(),
    });
}

#[test]
fn loop_halt_keep_going_resumes() {
    let mut a = app();
    halt(&mut a);
    let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
    assert_eq!(
        action,
        Action::Command(Command::ResolveLoop {
            resolution: LoopResolution::Resume
        })
    );
    assert!(a.prompts.loop_halt.is_none());
}

#[test]
fn loop_halt_stop_and_esc_both_stop() {
    for key in [KeyCode::Char('s'), KeyCode::Esc] {
        let mut a = app();
        halt(&mut a);
        let action = a.on_key(KeyEvent::from(key));
        assert_eq!(
            action,
            Action::Command(Command::ResolveLoop {
                resolution: LoopResolution::Stop
            })
        );
    }
}

#[test]
fn loop_halt_say_something_then_steer() {
    let mut a = app();
    halt(&mut a);
    a.on_key(KeyEvent::from(KeyCode::Char('t')));
    assert!(a.prompts.loop_halt.as_ref().is_some_and(|h| h.steering));
    for c in "read a.txt".chars() {
        a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::ResolveLoop {
            resolution: LoopResolution::Steer("read a.txt".into())
        })
    );
}

#[test]
fn loop_halt_steer_esc_returns_to_menu() {
    let mut a = app();
    halt(&mut a);
    a.on_key(KeyEvent::from(KeyCode::Char('t')));
    a.on_key(KeyEvent::from(KeyCode::Char('x')));
    a.on_key(KeyEvent::from(KeyCode::Esc)); // back to menu, not a decision
    assert!(a.prompts.loop_halt.as_ref().is_some_and(|h| !h.steering));
    let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
    assert_eq!(
        action,
        Action::Command(Command::ResolveLoop {
            resolution: LoopResolution::Resume
        })
    );
}

#[test]
fn loop_halt_stills_motion() {
    let mut a = app();
    a.anim.busy = true;
    a.anim.active = true;
    halt(&mut a);
    assert!(!a.is_working(), "the halt screen is perfectly still");
    assert!(!a.is_animating());
}

// ---- completion-gate halt surface (S-6, Design §8.7) -----------------

fn gate_halt(a: &mut App) {
    a.apply_event(UiEvent::CompletionGateHalted {
        failing: vec![CheckResult {
            name: "tests".into(),
            passed: false,
            reason: "exit 1".into(),
        }],
        attempts: 3,
    });
}

#[test]
fn completion_gate_keep_going_resumes() {
    let mut a = app();
    gate_halt(&mut a);
    let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
    assert_eq!(
        action,
        Action::Command(Command::ResolveCompletionGate {
            resolution: GateResolution::Resume
        })
    );
    assert!(a.prompts.completion_gate.is_none());
}

#[test]
fn completion_gate_stop_and_esc_both_stop() {
    for key in [KeyCode::Char('s'), KeyCode::Esc] {
        let mut a = app();
        gate_halt(&mut a);
        let action = a.on_key(KeyEvent::from(key));
        assert_eq!(
            action,
            Action::Command(Command::ResolveCompletionGate {
                resolution: GateResolution::Stop
            })
        );
    }
}

#[test]
fn completion_gate_finish_anyway_is_a_distinct_choice() {
    // "Finish anyway" is a fourth, separate key from stop — the explicit
    // override the loop-halt surface has no equivalent of (Design §8.7).
    let mut a = app();
    gate_halt(&mut a);
    let action = a.on_key(KeyEvent::from(KeyCode::Char('f')));
    assert_eq!(
        action,
        Action::Command(Command::ResolveCompletionGate {
            resolution: GateResolution::Finish
        })
    );
    assert!(a.prompts.completion_gate.is_none());
}

#[test]
fn completion_gate_say_something_then_steer() {
    let mut a = app();
    gate_halt(&mut a);
    a.on_key(KeyEvent::from(KeyCode::Char('t')));
    assert!(a
        .prompts
        .completion_gate
        .as_ref()
        .is_some_and(|h| h.steering));
    for c in "fix the failing test".chars() {
        a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::ResolveCompletionGate {
            resolution: GateResolution::Steer("fix the failing test".into())
        })
    );
}

#[test]
fn completion_gate_steer_esc_returns_to_menu() {
    let mut a = app();
    gate_halt(&mut a);
    a.on_key(KeyEvent::from(KeyCode::Char('t')));
    a.on_key(KeyEvent::from(KeyCode::Char('x')));
    a.on_key(KeyEvent::from(KeyCode::Esc)); // back to menu, not a decision
    assert!(a
        .prompts
        .completion_gate
        .as_ref()
        .is_some_and(|h| !h.steering));
    let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
    assert_eq!(
        action,
        Action::Command(Command::ResolveCompletionGate {
            resolution: GateResolution::Resume
        })
    );
}

#[test]
fn completion_gate_stills_motion() {
    let mut a = app();
    a.anim.busy = true;
    a.anim.active = true;
    gate_halt(&mut a);
    assert!(!a.is_working(), "the halt screen is perfectly still");
    assert!(!a.is_animating());
}

#[test]
fn completion_gate_status_hidden_until_first_evaluation() {
    let mut a = app();
    assert!(
        a.completion_status.is_empty(),
        "no gate status before any evaluation — never a \"None\" stub"
    );
    a.apply_event(UiEvent::CompletionStatus {
        checks: vec![CheckResult {
            name: "tests".into(),
            passed: true,
            reason: "exit 0".into(),
        }],
    });
    assert_eq!(a.completion_status.len(), 1);
    assert!(a.completion_status[0].passed);
}

#[test]
fn permission_scroll_keys_review_without_deciding() {
    let mut a = app();
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(9),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: x".into(),
            detail: "long\ncommand".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash asks".into(),
            on_behalf_of: None,
        },
    });
    // Scrolling and Space page-down must NOT decide.
    a.on_key(KeyEvent::from(KeyCode::Down));
    assert!(a.prompts.permission_scroll > 0);
    assert!(a.prompts.permission.is_some(), "scroll must not decide");
    a.on_key(KeyEvent::from(KeyCode::Char(' ')));
    assert!(a.prompts.permission.is_some());
    // A stray letter is ignored — no accidental decision either way.
    a.on_key(KeyEvent::from(KeyCode::Char('k')));
    assert!(a.prompts.permission.is_some());
    // Home returns to the top.
    a.on_key(KeyEvent::from(KeyCode::Home));
    assert_eq!(a.prompts.permission_scroll, 0);
}

#[test]
fn wheel_scrolls_a_permission_prompt_without_deciding() {
    let mut a = app();
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(11),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: x".into(),
            detail: "long\ncommand\nbelow\nthe\nfold".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash asks".into(),
            on_behalf_of: None,
        },
    });
    // The wheel reviews the prompt body (permission_scroll), never the
    // conversation, and never decides (Design §5, §3.4).
    a.on_scroll(false);
    assert!(a.prompts.permission_scroll > 0);
    assert_eq!(
        a.timeline.scroll, 0,
        "conversation untouched while a prompt is up"
    );
    assert!(a.prompts.permission.is_some(), "the wheel never decides");
    a.on_scroll(true);
    assert_eq!(a.prompts.permission_scroll, 0);
    assert!(a.prompts.permission.is_some());
}

#[test]
fn permission_allows_only_on_deliberate_key() {
    let mut a = app();
    a.apply_event(UiEvent::PermissionRequest {
        id: PermissionId(2),
        rendering: PermissionRendering {
            tool: "bash".into(),
            summary: "run: ls".into(),
            detail: "ls".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash asks".into(),
            on_behalf_of: None,
        },
    });
    let action = a.on_key(KeyEvent::from(KeyCode::Char('y')));
    assert_eq!(
        action,
        Action::Command(Command::PermissionAnswer {
            id: PermissionId(2),
            decision: PermissionDecision::AllowOnce,
        })
    );
}

#[test]
fn enter_submits_user_input() {
    let mut a = app();
    a.on_key(KeyEvent::from(KeyCode::Char('h')));
    a.on_key(KeyEvent::from(KeyCode::Char('i')));
    let action = a.on_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        action,
        Action::Command(Command::UserInput { text: "hi".into() })
    );
    assert!(a.editor.is_empty());
    // The prompt is echoed into the timeline so the pane is a full
    // top-to-bottom transcript of both sides.
    assert_eq!(a.timeline.items.last(), Some(&ConvItem::User("hi".into())));
}

#[test]
fn slash_attach_sends_the_path() {
    // FR-10: `/attach <path>` sends AttachImage verbatim; no path is a plain
    // usage notice, not a silent no-op (Design §4.14).
    let mut a = app();
    assert_eq!(
        a.run_slash("attach mockup.png"),
        Action::Command(Command::AttachImage {
            path: "mockup.png".into(),
        })
    );
    let mut a = app();
    assert_eq!(a.run_slash("attach"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("usage: /attach"))));
}

#[test]
fn image_attached_event_stages_then_send_turns_it_into_a_chip() {
    // FR-10, Design §4.14: `ImageAttached` stages the image in the compose
    // area; sending the message drains it into a chip right after the
    // user's own message, never as tool activity.
    let mut a = app();
    a.apply_event(UiEvent::ImageAttached {
        path: "/tmp/mockup.png".into(),
        name: "mockup.png".into(),
        media_type: "image/png".into(),
        width: 800,
        height: 600,
        format_label: "PNG".into(),
        pending_count: 1,
    });
    assert_eq!(a.pending_attachments.len(), 1);

    a.on_key(KeyEvent::from(KeyCode::Char('h')));
    a.on_key(KeyEvent::from(KeyCode::Char('i')));
    a.on_key(KeyEvent::from(KeyCode::Enter));

    assert!(a.pending_attachments.is_empty());
    let last_two: Vec<_> = a.timeline.items.iter().rev().take(2).collect();
    assert!(matches!(
        last_two[1],
        ConvItem::User(t) if t == "hi"
    ));
    assert!(matches!(
        last_two[0],
        ConvItem::Attachment { name, width: 800, height: 600, format_label }
        if name == "mockup.png" && format_label == "PNG"
    ));
}

#[test]
fn attach_failed_event_is_a_plain_notice() {
    let mut a = app();
    a.apply_event(UiEvent::AttachFailed {
        path: "data.bin".into(),
        reason: "not a recognized image format".into(),
    });
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("attach failed") && m.contains("data.bin"))));
}

#[test]
fn slash_command_is_not_echoed_as_a_message() {
    let mut a = app();
    for c in "/help".chars() {
        a.on_key(KeyEvent::from(KeyCode::Char(c)));
    }
    a.on_key(KeyEvent::from(KeyCode::Enter));
    // A slash command runs (opens the help overlay); it is not a message.
    assert!(!a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::User(_))));
}

fn picker(a: &mut App, rows: Vec<SessionRow>) {
    a.push_overlay(Overlay {
        title: "sessions".into(),
        content: OverlayContent::Sessions { rows, selected: 0 },
        scroll: 0,
    });
}

fn row(id: SessionId, current: bool) -> SessionRow {
    SessionRow {
        id,
        title: "t".into(),
        subtitle: "s".into(),
        current,
    }
}

#[test]
fn new_command_returns_action_when_idle_and_is_refused_while_busy() {
    let mut a = app();
    assert_eq!(a.run_command(AppCommand::NewSession), Action::NewSession);
    a.anim.busy = true;
    assert_eq!(a.run_command(AppCommand::NewSession), Action::None);
    assert!(matches!(a.timeline.items.last(), Some(ConvItem::Notice(_))));
}

#[test]
fn clear_is_an_alias_for_new_session() {
    assert_eq!(
        commands::by_name("clear").map(|spec| spec.cmd),
        Some(AppCommand::NewSession)
    );
}

#[test]
fn session_command_opens_the_picker() {
    let mut a = app();
    let _ = a.run_command(AppCommand::Session);
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::Sessions { .. })
    ));
}

#[test]
fn picker_enter_on_the_current_session_does_not_resume() {
    let mut a = app();
    let id = a.session.session_id;
    picker(&mut a, vec![row(id, true)]);
    let action = a.on_session_picker_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::None, "cannot resume the current session");
    assert!(a.overlays.is_empty(), "picker dismissed");
}

#[test]
fn picker_enter_on_another_session_resumes_it() {
    let mut a = app();
    let other = SessionId::new();
    picker(&mut a, vec![row(other, false)]);
    let action = a.on_session_picker_key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(action, Action::ResumeSession(other));
    assert!(a.overlays.is_empty());
}

#[test]
fn picker_enter_while_busy_defers_instead_of_switching() {
    let mut a = app();
    a.anim.busy = true;
    picker(&mut a, vec![row(SessionId::new(), false)]);
    assert_eq!(
        a.on_session_picker_key(KeyEvent::from(KeyCode::Enter)),
        Action::None
    );
    assert!(matches!(a.timeline.items.last(), Some(ConvItem::Notice(_))));
}

#[test]
fn begin_new_session_resets_the_timeline_and_identity() {
    let mut a = app();
    a.timeline.items.push(ConvItem::User("old".into()));
    a.files.modified.push(ModifiedFile {
        path: "x".into(),
        adds: 1,
        dels: 0,
    });
    let id = SessionId::new();
    a.begin_new_session(id);
    assert_eq!(a.session.session_id, id);
    assert!(a.session.title.is_empty());
    assert!(a.files.modified.is_empty());
    assert!(!a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::User(t) if t == "old")));
}

#[test]
fn begin_resumed_session_adopts_title_and_clears_prior_timeline() {
    let mut a = app();
    a.timeline.items.push(ConvItem::User("old".into()));
    let id = SessionId::new();
    a.begin_resumed_session(id, "resumed".into(), &[]);
    assert_eq!(a.session.session_id, id);
    assert_eq!(a.session.title, "resumed");
    assert!(!a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::User(t) if t == "old")));
}

#[test]
fn reasoning_delta_builds_a_trail_that_settles_on_the_answer() {
    let mut a = app();
    a.apply_event(UiEvent::ReasoningDelta {
        text: "think ".into(),
    });
    a.apply_event(UiEvent::ReasoningDelta {
        text: "more".into(),
    });
    // While thinking, the trail streams expanded.
    assert!(matches!(
        a.timeline.items.last(),
        Some(ConvItem::Reasoning { text, expanded: true }) if text == "think more"
    ));
    // The answer begins → the trail settles to collapsed (default view).
    a.apply_event(UiEvent::AssistantDelta {
        text: "answer".into(),
    });
    assert!(matches!(
        a.timeline.items.first(),
        Some(ConvItem::Reasoning {
            expanded: false,
            ..
        })
    ));
    assert!(matches!(a.timeline.items.last(), Some(ConvItem::Assistant(t)) if t == "answer"));
}

#[test]
fn hidden_view_drops_the_trail_but_keeps_the_answer() {
    let mut a = app();
    a.timeline.reasoning = ReasoningView::Hidden;
    a.apply_event(UiEvent::ReasoningDelta {
        text: "secret".into(),
    });
    a.apply_event(UiEvent::AssistantDelta {
        text: "answer".into(),
    });
    assert!(!a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Reasoning { .. })));
    assert!(matches!(a.timeline.items.last(), Some(ConvItem::Assistant(t)) if t == "answer"));
}

#[test]
fn expanded_view_keeps_the_trail_open_after_the_answer() {
    let mut a = app();
    a.timeline.reasoning = ReasoningView::Expanded;
    a.apply_event(UiEvent::ReasoningDelta { text: "why".into() });
    a.apply_event(UiEvent::AssistantDelta { text: "a".into() });
    assert!(matches!(
        a.timeline.items.first(),
        Some(ConvItem::Reasoning { expanded: true, .. })
    ));
}

#[test]
fn ctrl_r_toggles_the_reasoning_trail() {
    let mut a = app();
    a.apply_event(UiEvent::ReasoningDelta { text: "hmm".into() });
    a.apply_event(UiEvent::AssistantDelta { text: "a".into() }); // settle → collapsed
    a.toggle_reasoning();
    assert!(matches!(
        a.timeline.items.first(),
        Some(ConvItem::Reasoning { expanded: true, .. })
    ));
}

#[test]
fn ctrl_l_forces_a_redraw() {
    let mut a = app();
    let action = a.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert_eq!(action, Action::ForceRedraw);
}

#[test]
fn effort_picker_offers_levels_and_declines_when_none() {
    let mut a = app();
    // No effort control ⇒ a calm notice, no overlay.
    assert_eq!(a.run_slash("effort"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("no reasoning-effort"))));
    assert!(a.overlays.is_empty());

    // With levels available, `/effort` opens the picker.
    a.effort_levels = vec![Effort::Low, Effort::High];
    a.effort = Some(Effort::Low);
    a.run_slash("effort");
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::Choices {
            kind: ChoiceKind::Effort,
            ..
        })
    ));
}

#[test]
fn effort_arg_sets_a_supported_level_and_rejects_others() {
    let mut a = app();
    a.effort_levels = vec![Effort::Low, Effort::High];
    assert_eq!(
        a.run_slash("effort high"),
        Action::Command(Command::SetEffort {
            effort: Effort::High
        })
    );
    // A level the model doesn't offer is a notice, not a command.
    assert_eq!(a.run_slash("effort medium"), Action::None);
    assert!(a
        .timeline
        .items
        .iter()
        .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("does not offer"))));
}

#[test]
fn effort_changed_updates_sidebar_state() {
    let mut a = app();
    a.apply_event(UiEvent::EffortChanged {
        effort: Some(Effort::High),
        available: vec![Effort::Low, Effort::High],
    });
    assert_eq!(a.effort, Some(Effort::High));
    assert_eq!(a.effort_levels, vec![Effort::Low, Effort::High]);
}

#[test]
fn compact_command_is_reachable_via_slash_and_palette() {
    // The registry entry exists so /compact flows through `by_name` and
    // the palette fuzzy list (Design §3.3, Tech Spec §9).
    assert_eq!(
        crate::commands::by_name("compact").map(|spec| spec.cmd),
        Some(crate::commands::AppCommand::Compact)
    );
    // `/compact` via the slash parser dispatches Command::Compact.
    let mut a = app();
    assert_eq!(a.run_slash("compact"), Action::Command(Command::Compact));
    // The palette entry dispatches the same command.
    assert_eq!(
        a.run_command(crate::commands::AppCommand::Compact),
        Action::Command(Command::Compact)
    );
    // `compact` appears in the command listing (used by /help).
    assert!(crate::commands::COMMANDS
        .iter()
        .any(|c| c.name == "compact"));
}

// ---- memory inspector (`/memory`, FR-6, Design §4.9) ------------------

fn mem_summary(name: &str, desc: &str, scope: MemoryScope) -> EntrySummary {
    EntrySummary {
        name: name.into(),
        description: desc.into(),
        type_: None,
        scope,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn memory_command_requests_the_list() {
    let mut a = app();
    // Reachable three ways (§3.3): slash, palette dispatch, and registry.
    assert_eq!(a.run_slash("memory"), Action::Command(Command::MemoryList));
    assert_eq!(
        a.run_command(AppCommand::Memory),
        Action::Command(Command::MemoryList)
    );
    assert!(commands::COMMANDS.iter().any(|c| c.name == "memory"));
}

#[test]
fn memory_entries_open_grouped_overlay_with_origin() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
        project: vec![mem_summary("Proj", "p", MemoryScope::Project)],
    });
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::MemoryEntries {
            user,
            project,
            selected,
            confirm_delete,
        }) => {
            assert_eq!(user.len(), 1);
            assert_eq!(project.len(), 1);
            assert_eq!(user[0].scope, MemoryScope::User);
            assert_eq!(project[0].scope, MemoryScope::Project);
            assert_eq!(*selected, 0);
            assert!(!confirm_delete);
        }
        other => panic!("expected MemoryEntries overlay, got {other:?}"),
    }
}

#[test]
fn memory_enter_issues_view_for_the_selected_entry() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![
            mem_summary("Alpha", "first", MemoryScope::User),
            mem_summary("Beta", "second", MemoryScope::User),
        ],
        project: vec![],
    });
    a.on_key(key(KeyCode::Down)); // move to Beta
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        Action::Command(Command::MemoryView {
            scope: MemoryScope::User,
            name: "Beta".into(),
        })
    );
}

#[test]
fn memory_delete_requires_explicit_confirmation() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
        project: vec![],
    });
    // A single `d` arms the confirm — it does NOT delete.
    assert_eq!(a.on_key(key(KeyCode::Char('d'))), Action::None);
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::MemoryEntries {
            confirm_delete: true,
            ..
        })
    ));
    // `y` confirms → the delete is issued (harness performs the write).
    assert_eq!(
        a.on_key(key(KeyCode::Char('y'))),
        Action::Command(Command::MemoryMutate {
            op: MemoryOp::Remove,
            scope: MemoryScope::User,
            name: "Alpha".into(),
            description: None,
            type_: None,
            body: None,
        })
    );
}

#[test]
fn memory_delete_is_cancelled_by_any_other_key() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
        project: vec![],
    });
    a.on_key(key(KeyCode::Char('d')));
    // Anything but `y` cancels: no command, confirm cleared, entry intact.
    assert_eq!(a.on_key(key(KeyCode::Char('n'))), Action::None);
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::MemoryEntries {
            confirm_delete: false,
            ..
        })
    ));
}

#[test]
fn memory_edit_stages_a_pending_edit_when_body_arrives() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
        project: vec![],
    });
    // `e` issues a body fetch with the edit intent.
    assert_eq!(
        a.on_key(key(KeyCode::Char('e'))),
        Action::Command(Command::MemoryView {
            scope: MemoryScope::User,
            name: "Alpha".into(),
        })
    );
    // The body reply stages a pending edit for the loop's $EDITOR handoff,
    // carrying the metadata through unchanged (so a body edit never erases
    // the description).
    a.apply_event(UiEvent::MemoryBody {
        scope: MemoryScope::User,
        name: "Alpha".into(),
        body: "the body".into(),
    });
    let edit = a.take_pending_memory_edit().expect("edit staged");
    assert_eq!(edit.name, "Alpha");
    assert_eq!(edit.scope, MemoryScope::User);
    assert_eq!(edit.description, Some("first".into()));
    assert_eq!(edit.body, "the body");
    // An edit does not open a read-only overlay (that's the view path).
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::MemoryEntries { .. })
    ));
}

#[test]
fn memory_view_opens_a_read_only_body_overlay() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
        project: vec![],
    });
    a.on_key(key(KeyCode::Enter)); // view intent
    a.apply_event(UiEvent::MemoryBody {
        scope: MemoryScope::User,
        name: "Alpha".into(),
        body: "hello body".into(),
    });
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::Text(body)) => assert!(body.contains("hello body")),
        other => panic!("expected a Text overlay, got {other:?}"),
    }
    // A view never stages an edit.
    assert!(a.take_pending_memory_edit().is_none());
}

#[test]
fn memory_entries_refresh_in_place_and_clamp_selection() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![
            mem_summary("Alpha", "a", MemoryScope::User),
            mem_summary("Beta", "b", MemoryScope::User),
        ],
        project: vec![],
    });
    a.on_key(key(KeyCode::Down)); // select Beta (index 1)
                                  // A re-list with fewer entries reuses the overlay and clamps selection.
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "a", MemoryScope::User)],
        project: vec![],
    });
    assert_eq!(a.overlays.len(), 1);
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::MemoryEntries { selected: 0, .. })
    ));
}

#[test]
fn memory_inspector_esc_dismisses() {
    let mut a = app();
    a.apply_event(UiEvent::MemoryEntries {
        user: vec![mem_summary("Alpha", "a", MemoryScope::User)],
        project: vec![],
    });
    assert_eq!(a.on_key(key(KeyCode::Esc)), Action::None);
    assert!(a.overlays.is_empty());
}

// ---- skills inspector (`/skills`, FR-7, Design §4.9) ------------------

fn skill_meta(name: &str, desc: &str, origin: SkillOrigin) -> SkillMeta {
    SkillMeta {
        name: name.into(),
        description: desc.into(),
        origin,
    }
}

#[test]
fn skills_command_opens_inspector_from_cached_catalog() {
    let mut a = app();
    a.skills = vec![
        skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User),
        skill_meta("linter", "Run linters", SkillOrigin::Project),
    ];
    // No engine round-trip — the catalog is already cached, so the overlay
    // opens immediately.
    assert_eq!(a.run_slash("skills"), Action::None);
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::SkillList { skills, selected }) => {
            assert_eq!(skills.len(), 2);
            assert_eq!(skills[0].origin, SkillOrigin::User);
            assert_eq!(skills[1].origin, SkillOrigin::Project);
            assert_eq!(*selected, 0);
        }
        other => panic!("expected SkillList overlay, got {other:?}"),
    }
    // Reachable three ways (§3.3): palette dispatch and the registry too.
    assert_eq!(a.run_command(AppCommand::Skills), Action::None);
    assert!(commands::COMMANDS.iter().any(|c| c.name == "skills"));
}

#[test]
fn skills_enter_issues_inspect_for_the_selected_skill() {
    let mut a = app();
    a.skills = vec![
        skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User),
        skill_meta("linter", "Run linters", SkillOrigin::Project),
    ];
    a.run_command(AppCommand::Skills);
    a.on_key(key(KeyCode::Down)); // select linter
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        Action::Command(Command::InspectSkill {
            name: "linter".into(),
        })
    );
}

#[test]
fn skill_body_opens_a_read_only_overlay_with_resources() {
    let mut a = app();
    a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
    a.run_command(AppCommand::Skills);
    a.on_key(key(KeyCode::Enter));
    a.apply_event(UiEvent::SkillBody {
        name: "pdf-fill".into(),
        origin: SkillOrigin::User,
        body: "Step 1: open the template.".into(),
        resources: vec!["/abs/template.txt".into()],
    });
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::Text(body)) => {
            assert!(body.contains("Step 1: open the template."));
            assert!(body.contains("template.txt"), "bundled resource listed");
        }
        other => panic!("expected a Text overlay, got {other:?}"),
    }
}

#[test]
fn skills_inspector_has_no_edit_or_delete() {
    let mut a = app();
    a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
    a.run_command(AppCommand::Skills);
    // `e`/`d` are inert for skills (read-only) — no command, overlay stays.
    assert_eq!(a.on_key(key(KeyCode::Char('e'))), Action::None);
    assert_eq!(a.on_key(key(KeyCode::Char('d'))), Action::None);
    assert!(matches!(
        a.overlays.last().map(|o| &o.content),
        Some(OverlayContent::SkillList { .. })
    ));
}

#[test]
fn skills_empty_catalog_opens_an_empty_overlay() {
    let mut a = app();
    a.skills.clear();
    a.run_command(AppCommand::Skills);
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::SkillList { skills, .. }) => assert!(skills.is_empty()),
        other => panic!("expected an empty SkillList overlay, got {other:?}"),
    }
}

#[test]
fn skills_inspector_esc_dismisses() {
    let mut a = app();
    a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
    a.run_command(AppCommand::Skills);
    assert_eq!(a.on_key(key(KeyCode::Esc)), Action::None);
    assert!(a.overlays.is_empty());
}

// ---- Agents inspector (`/agents`, FR-9, Design §3.1/§4.13) -------------

fn agent_summary(id: &str, name: &str, ended: bool) -> crate::app::AgentSummary {
    crate::app::AgentSummary {
        id: id.into(),
        name: name.into(),
        ended,
    }
}

#[test]
fn subagent_spawned_and_ended_events_mark_ended_rather_than_remove() {
    // Design §4.13: an ended subagent stays reachable from `/agents` for the
    // rest of the session, so `SubagentEnded` marks the entry rather than
    // dropping it — only the sidebar section (rendering) filters ended ones
    // back out.
    let mut a = app();
    a.apply_event(UiEvent::SubagentSpawned {
        id: "agent-1".into(),
        name: "reviewer".into(),
        profile: "default".into(),
        model: "m".into(),
    });
    assert_eq!(a.agents.len(), 1);
    assert_eq!(a.agents[0].id, "agent-1");
    assert_eq!(a.agents[0].name, "reviewer");
    assert!(!a.agents[0].ended);
    a.apply_event(UiEvent::SubagentEnded {
        id: "agent-1".into(),
        reason: "done".into(),
    });
    assert_eq!(
        a.agents.len(),
        1,
        "ended subagent stays in the catalog, not removed"
    );
    assert!(a.agents[0].ended);
}

#[test]
fn agents_command_opens_inspector_from_cached_list() {
    let mut a = app();
    a.agents = vec![
        agent_summary("agent-1", "reviewer", false),
        agent_summary("agent-2", "tester", false),
    ];
    // No engine round-trip — the alive list is already cached.
    assert_eq!(a.run_slash("agents"), Action::None);
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::AgentList { agents, selected }) => {
            assert_eq!(agents.len(), 2);
            assert_eq!(agents[0].id, "agent-1");
            assert_eq!(*selected, 0);
        }
        other => panic!("expected AgentList overlay, got {other:?}"),
    }
    assert_eq!(a.run_command(AppCommand::Agents), Action::None);
    assert!(commands::COMMANDS.iter().any(|c| c.name == "agents"));
}

#[test]
fn agents_enter_issues_inspect_for_the_selected_agent() {
    let mut a = app();
    a.agents = vec![
        agent_summary("agent-1", "reviewer", false),
        agent_summary("agent-2", "tester", false),
    ];
    a.run_command(AppCommand::Agents);
    a.on_key(key(KeyCode::Down)); // select agent-2
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        Action::Command(Command::InspectAgent {
            id: "agent-2".into(),
        })
    );
}

#[test]
fn agents_enter_on_an_ended_entry_still_inspects_it() {
    // Design §4.13: "a subagent that has ended keeps its inspector
    // reachable for the rest of the session" — Enter on an ended row issues
    // the same InspectAgent command as a live one.
    let mut a = app();
    a.agents = vec![agent_summary("agent-1", "reviewer", true)];
    a.run_command(AppCommand::Agents);
    assert_eq!(
        a.on_key(key(KeyCode::Enter)),
        Action::Command(Command::InspectAgent {
            id: "agent-1".into(),
        })
    );
}

#[test]
fn agent_activity_opens_a_read_only_overlay() {
    let mut a = app();
    a.agents = vec![agent_summary("agent-1", "reviewer", false)];
    a.run_command(AppCommand::Agents);
    a.on_key(key(KeyCode::Enter));
    a.apply_event(UiEvent::AgentActivity {
        id: "agent-1".into(),
        name: "reviewer".into(),
        text: "user: review this diff\nassistant: looks good".into(),
    });
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::AgentActivity { id, name, text }) => {
            assert_eq!(id, "agent-1");
            assert_eq!(name, "reviewer");
            assert!(text.contains("looks good"));
        }
        other => panic!("expected an AgentActivity overlay, got {other:?}"),
    }
    assert_eq!(
        a.watched_agent_id(),
        Some("agent-1".into()),
        "the open activity overlay is the one the periodic refresh polls"
    );
}

#[test]
fn agent_activity_refresh_updates_the_open_overlay_in_place() {
    // Design §4.13's "live-updating... as it happens": a second reply for
    // the *same* id (what the periodic refresh ticker in `tui::run` sends)
    // updates the existing overlay's text rather than stacking a new one.
    let mut a = app();
    a.agents = vec![agent_summary("agent-1", "reviewer", false)];
    a.run_command(AppCommand::Agents);
    a.on_key(key(KeyCode::Enter));
    a.apply_event(UiEvent::AgentActivity {
        id: "agent-1".into(),
        name: "reviewer".into(),
        text: "user: start".into(),
    });
    let depth_before = a.overlays.len();
    a.apply_event(UiEvent::AgentActivity {
        id: "agent-1".into(),
        name: "reviewer".into(),
        text: "user: start\nassistant: still working".into(),
    });
    assert_eq!(a.overlays.len(), depth_before, "no new overlay is stacked");
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::AgentActivity { text, .. }) => {
            assert!(text.contains("still working"));
        }
        other => panic!("expected an AgentActivity overlay, got {other:?}"),
    }
}

#[test]
fn agent_activity_reply_for_an_abandoned_id_is_dropped() {
    // A stale reply for an id the user is no longer looking at must not
    // silently replace what is currently on screen.
    let mut a = app();
    a.agents = vec![
        agent_summary("agent-1", "reviewer", false),
        agent_summary("agent-2", "tester", false),
    ];
    a.run_command(AppCommand::Agents);
    a.on_key(key(KeyCode::Enter)); // opens agent-1's activity
    a.apply_event(UiEvent::AgentActivity {
        id: "agent-1".into(),
        name: "reviewer".into(),
        text: "reviewer's activity".into(),
    });
    // A late reply for a different subagent arrives (e.g. a stale periodic
    // refresh from before the user moved on).
    a.apply_event(UiEvent::AgentActivity {
        id: "agent-2".into(),
        name: "tester".into(),
        text: "tester's activity".into(),
    });
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::AgentActivity { id, text, .. }) => {
            assert_eq!(id, "agent-1", "the shown subagent did not silently change");
            assert!(text.contains("reviewer's activity"));
        }
        other => panic!("expected an AgentActivity overlay, got {other:?}"),
    }
}

#[test]
fn watched_agent_id_is_none_without_an_open_activity_overlay() {
    let mut a = app();
    assert_eq!(a.watched_agent_id(), None);
    a.agents = vec![agent_summary("agent-1", "reviewer", false)];
    a.run_command(AppCommand::Agents); // AgentList, not AgentActivity, is open
    assert_eq!(a.watched_agent_id(), None);
}

#[test]
fn agents_empty_list_opens_an_empty_overlay() {
    let mut a = app();
    a.agents.clear();
    a.run_command(AppCommand::Agents);
    match a.overlays.last().map(|o| &o.content) {
        Some(OverlayContent::AgentList { agents, .. }) => assert!(agents.is_empty()),
        other => panic!("expected an empty AgentList overlay, got {other:?}"),
    }
}

#[test]
fn agents_inspector_esc_dismisses() {
    let mut a = app();
    a.agents = vec![agent_summary("agent-1", "reviewer", false)];
    a.run_command(AppCommand::Agents);
    assert_eq!(a.on_key(key(KeyCode::Esc)), Action::None);
    assert!(a.overlays.is_empty());
}
