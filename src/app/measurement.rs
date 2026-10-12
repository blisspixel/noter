//! CPU-only measurements of the real bounded Text Mode adapter in test builds.

use std::time::Instant;

use super::*;

const MARKER: &str = "NOTER-SEARCH-MIDDLE-932E8A04";
const RTL_SOURCE: &str = "Direction boundaries: left \u{2067}\u{5e9}\u{5dc}\u{5d5}\u{5dd} 123\u{2069} right | \u{645}\u{631}\u{62d}\u{628}\u{627}\n";

struct CpuSamples {
    first_frame_ns: u128,
    clock_ns: Vec<u128>,
    idle_ns: Vec<u128>,
    edit_ns: Vec<u128>,
    idle_nodes: Vec<usize>,
    edit_nodes: Vec<usize>,
    revisions: Vec<u64>,
}

fn frame(
    app: &mut NoterApp,
    context: &egui::Context,
    sequence: &mut u32,
    events: Vec<egui::Event>,
) -> (u128, usize) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1_200.0, 760.0),
        )),
        time: Some(f64::from(*sequence) / 60.0),
        events,
        ..Default::default()
    };
    *sequence += 1;
    let started = Instant::now();
    let output = context.run_ui(input, |ui| app.show_editor(ui));
    let elapsed = started.elapsed().as_nanos();
    assert!(!output.shapes.is_empty());
    assert!(
        app.error_msg.is_none(),
        "the bounded adapter rejected input"
    );
    let nodes = output
        .platform_output
        .accesskit_update
        .as_ref()
        .map_or(0, |update| update.nodes.len());
    (elapsed, nodes)
}

fn measure(
    source: &str,
    wrapped: bool,
    accessible: bool,
    warmup: usize,
    count: usize,
) -> CpuSamples {
    assert!(!source.is_empty() && source.len() <= INTERACTIVE_TEXT_MAX_BYTES);
    assert!(warmup <= 100 && (1..=100).contains(&count));
    let offset = source.find(MARKER).expect("the synthetic marker exists");
    assert_eq!(source.matches(MARKER).count(), 1);
    let mut app = NoterApp {
        document: Document::from_bytes(source.as_bytes()).expect("the corpus loads"),
        text: source.to_owned(),
        view: DocumentView::Text,
        text_wrap: if wrapped {
            TextWrap::Wrapped
        } else {
            TextWrap::Unwrapped
        },
        ..NoterApp::default()
    };
    let context = egui::Context::default();
    theme::configure_styles(&context);
    app.font_fallback = Some(FontFallback::new(context.clone()));
    assert!(!app.crash_recovery.is_unavailable());
    if accessible {
        context.enable_accesskit();
    }
    context.memory_mut(|memory| memory.request_focus(app.editor_id()));
    let mut sequence = 0;
    let (first_frame_ns, first_nodes) = frame(&mut app, &context, &mut sequence, Vec::new());
    assert_eq!(first_nodes > 0, accessible);
    let mut samples = CpuSamples {
        first_frame_ns,
        clock_ns: Vec::with_capacity(count),
        idle_ns: Vec::with_capacity(count),
        edit_ns: Vec::with_capacity(count),
        idle_nodes: Vec::with_capacity(count),
        edit_nodes: Vec::with_capacity(count),
        revisions: Vec::with_capacity(count),
    };
    for round in 0..warmup + count {
        let (elapsed, nodes) = frame(&mut app, &context, &mut sequence, Vec::new());
        assert_eq!(app.document.revision(), Revision::INITIAL);
        if round >= warmup {
            samples.idle_ns.push(elapsed);
            samples.idle_nodes.push(nodes);
            let started = Instant::now();
            samples.clock_ns.push(started.elapsed().as_nanos());
        }
    }
    for round in 0..warmup + count {
        // Restore selection outside the input measurement. A same-size ASCII
        // replacement remains valid at the existing 8 MiB production limit.
        app.selection = Selection::new(offset, offset + 1);
        app.pending_selection_restore = Some(app.selection);
        app.preserve_focus_on_selection_restore = true;
        let _ = frame(&mut app, &context, &mut sequence, Vec::new());
        assert!(context.memory(|memory| memory.has_focus(app.editor_id())));
        let replacement = if round % 2 == 0 { "x" } else { "y" };
        let previous_revision = app.document.revision();
        let (elapsed, nodes) = frame(
            &mut app,
            &context,
            &mut sequence,
            vec![egui::Event::Text(replacement.to_owned())],
        );
        assert_eq!(
            app.document.revision(),
            previous_revision.checked_next().unwrap()
        );
        assert_eq!(&app.text[offset..=offset], replacement);
        assert_eq!(app.selection, Selection::caret(offset + 1));
        assert_eq!(app.text.len(), source.len());
        assert!(app.document.rope().bytes().eq(app.text.bytes()));
        assert!(app.history.can_undo());
        assert_eq!(nodes > 0, accessible);
        if round >= warmup {
            samples.edit_ns.push(elapsed);
            samples.edit_nodes.push(nodes);
            samples.revisions.push(app.document.revision().get());
        }
    }
    assert_exact_history(&mut app, &context, source, offset, warmup + count);
    samples
}

fn assert_exact_history(
    app: &mut NoterApp,
    context: &egui::Context,
    source: &str,
    offset: usize,
    edits: usize,
) {
    let mut expected = source.to_owned();
    expected.replace_range(
        offset..=offset,
        if edits.is_multiple_of(2) { "y" } else { "x" },
    );
    assert!(
        app.text == expected,
        "edits must preserve every other source byte"
    );
    app.execute_edit_command(EditCommand::Undo, context);
    // Each replacement starts with a nonempty selection, so it is a separate
    // shared history entry rather than coalesced adjacent typing.
    let previous = if edits == 1 {
        "N"
    } else if edits.is_multiple_of(2) {
        "x"
    } else {
        "y"
    };
    expected.replace_range(offset..=offset, previous);
    assert!(
        app.text == expected,
        "Undo must restore the exact predecessor"
    );
    assert_eq!(app.selection, Selection::new(offset, offset + 1));
    assert!(app.document.rope().bytes().eq(app.text.bytes()));
}

#[test]
fn cpu_measurement_preserves_source_and_records_real_edits() {
    for source in [
        format!("An ordinary English draft.\n{MARKER}\nLast line.\n"),
        format!(
            "Mixed\r\nline\rends\nCafe\u{301} \u{6771} \u{1f642} \u{2067}\u{5e9}\u{2069}\n{MARKER}\n"
        ),
    ] {
        for wrapped in [false, true] {
            for accessible in [false, true] {
                for (warmup, count) in [(0, 1), (1, 2), (1, 3)] {
                    let samples = measure(&source, wrapped, accessible, warmup, count);
                    assert_eq!(samples.idle_ns.len(), count);
                    assert_eq!(samples.edit_ns.len(), count);
                    let expected = (warmup + 1..=warmup + count)
                        .map(|revision| revision as u64)
                        .collect::<Vec<_>>();
                    assert_eq!(samples.revisions, expected);
                    assert!(samples.edit_ns.iter().all(|value| *value > 0));
                }
            }
        }
    }
}

#[test]
fn cpu_probe() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::var_os("NOTER_M5_CPU_INPUT") else {
        return Ok(());
    };
    let count: usize = std::env::var("NOTER_M5_CPU_SAMPLES")?.parse()?;
    let warmup: usize = std::env::var("NOTER_M5_CPU_WARMUP")?.parse()?;
    let wrapped: bool = std::env::var("NOTER_M5_CPU_WRAP")?.parse()?;
    let accessible: bool = std::env::var("NOTER_M5_CPU_ACCESSKIT")?.parse()?;
    assert!(std::fs::metadata(&path)?.len() <= INTERACTIVE_TEXT_MAX_BYTES as u64);
    let source = std::fs::read_to_string(path)?;
    let samples = measure(&source, wrapped, accessible, warmup, count);
    println!(
        "NOTER_M5_CPU={{\"schema\":1,\"bytes\":{},\"wrapped\":{wrapped},\"accesskit\":{accessible},\"warmup_count\":{warmup},\"sample_count\":{count},\"first_frame_cpu_ns\":{},\"clock_ns\":{:?},\"idle_frame_cpu_ns\":{:?},\"edit_frame_cpu_ns\":{:?},\"idle_accesskit_nodes\":{:?},\"edit_accesskit_nodes\":{:?},\"edit_revisions\":{:?}}}",
        source.len(),
        samples.first_frame_ns,
        samples.clock_ns,
        samples.idle_ns,
        samples.edit_ns,
        samples.idle_nodes,
        samples.edit_nodes,
        samples.revisions,
    );
    Ok(())
}

fn assert_accessible_source(source: &str, fallback: Option<Vec<u8>>) {
    fn collect(
        id: egui::accesskit::NodeId,
        nodes: &std::collections::HashMap<egui::accesskit::NodeId, &egui::accesskit::Node>,
        text: &mut String,
    ) {
        let node = nodes.get(&id).expect("every accessible child must exist");
        if node.role() == egui::accesskit::Role::TextRun {
            text.push_str(node.value().expect("text runs must contain text"));
        }
        for child in node.children() {
            collect(*child, nodes, text);
        }
    }

    let context = egui::Context::default();
    theme::configure_styles(&context);
    if let Some(bytes) = fallback {
        context.add_font(egui::epaint::text::FontInsert::new(
            "m5-local-rtl-fixture",
            egui::FontData::from_owned(bytes),
            vec![egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::epaint::text::FontPriority::Lowest,
            }],
        ));
    }
    context.enable_accesskit();
    let mut app = NoterApp {
        document: Document::from_bytes(source.as_bytes()).expect("the source loads"),
        text: source.to_owned(),
        view: DocumentView::Text,
        ..NoterApp::default()
    };
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(320.0);
        app.show_editor(ui);
    });
    let update = output
        .platform_output
        .accesskit_update
        .expect("the first tree exists");
    let nodes = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let (editor, _) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(TEXT_EDITOR_ACCESSIBLE_NAME))
        .expect("the real document editor is named");
    let mut accessible = String::new();
    collect(*editor, &nodes, &mut accessible);
    let expected = crate::bounded_text_input::SourceDisplayProjection::new(source);
    assert!(
        accessible == expected.display(),
        "accessible TextRun text differs from the source projection: expected {} characters, received {}",
        expected.display().chars().count(),
        accessible.chars().count(),
    );
    assert!(app.text == source && app.document.rope().bytes().eq(source.bytes()));
}

#[test]
fn accessible_text_runs_match_source_projection() {
    for source in [
        "An ordinary English draft with a long wrapped line. ".repeat(20),
        "First\r\nSecond\rThird\nCafe\u{301} \u{6771}\u{4eac} \u{1f642}\n".to_owned(),
    ] {
        assert_accessible_source(&source, None);
    }
}

#[test]
fn rtl_accessible_text_runs_match_source_projection() {
    // Bundled-font coverage does not establish the native system-fallback-font
    // shaping or AT-SPI behavior reported in #60.
    assert_accessible_source(RTL_SOURCE, None);
}

#[test]
#[ignore = "Local RTL font fixture for known #60; requires NOTER_M5_RTL_FONT"]
fn local_font_rtl_accessible_text_runs_match_source_projection() {
    use skrifa::MetadataProvider as _;

    // This local fixture retains source equality as the repair criterion. It
    // currently fails with the identified Arial fallback, which is not bundled
    // or redistributed. The bundled-font RTL regression above remains active.
    let path = std::env::var_os("NOTER_M5_RTL_FONT").expect("supply a local RTL font");
    let bytes = crate::font_fallback::read_bounded(std::path::Path::new(&path))
        .expect("the fixture must be an ordinary bounded font file");
    let font = skrifa::FontRef::from_index(&bytes, 0).expect("the fixture must parse");
    let charmap = font.charmap();
    for character in RTL_SOURCE
        .chars()
        .filter(|ch| ('\u{590}'..='\u{6ff}').contains(ch))
    {
        assert!(
            charmap.map(character).is_some(),
            "the fixture must cover every RTL character"
        );
    }
    println!(
        "NOTER_M5_RTL_FONT_BYTES={}; BLAKE3={}",
        bytes.len(),
        blake3::hash(&bytes)
    );
    assert_accessible_source(RTL_SOURCE, Some(bytes));
}
