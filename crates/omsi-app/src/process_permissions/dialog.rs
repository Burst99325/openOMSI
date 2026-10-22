use glam::Vec2;
use omsi_plugin::api::process::{ProcessDecision, ProcessPrompt};
use omsi_ui::{Color, Rect, Weight, kit::ButtonKind, paint::Align, theme::*};

fn clean(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
// Break long file paths as well as words; no part of the executable path is omitted.
fn wrapped(ui: &omsi_ui::kit::Ui, text: &str, width: f32) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for c in text.chars() {
        if c == '\n' {
            out.push(c);
            line.clear();
            continue;
        }
        line.push(c);
        if ui.fonts.width_as_is(&line, 14.0, Weight::Regular) > width {
            out.push('\n');
            line.clear();
            line.push(c);
        }
        out.push(c);
    }
    out
}

// Translate only interface text, never plugin names, paths or arguments.
pub(super) fn details(prompt: &ProcessPrompt, tr: impl Fn(&str) -> String) -> String {
    format!(
        "{} {}\n\n{}\n{}\n\n{} {}\n\n{} {}\n{} {}\n\n{}\n{}",
        tr("Plugin:"),
        clean(&prompt.plugin),
        tr("This plugin wants to run the following file:"),
        clean(&prompt.file.to_string_lossy()),
        tr("Plugin entry:"),
        clean(&prompt.entry.to_string_lossy()),
        tr("Arguments:"),
        clean(&format!("{:?}", prompt.args)),
        tr("Working directory:"),
        prompt
            .cwd
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| tr("Inherited from the game")),
        tr("This program runs with your account's permissions. Only allow programs you trust."),
        tr(
            "Allow once permits this launch. Always allow remembers this plugin and file until their content changes. Deny blocks this file for this session."
        )
    )
}
pub(super) fn draw(ui: &mut omsi_ui::kit::Ui, prompt: &ProcessPrompt) -> Option<ProcessDecision> {
    let screen = Rect::new(0.0, 0.0, ui.size.x, ui.size.y);
    ui.solid(screen);
    ui.p().rect(screen, Color([0.0, 0.0, 0.0, 0.65]));
    let w = (ui.size.x - 24.0).clamp(240.0, 680.0);
    let h = (ui.size.y - 24.0).clamp(240.0, 440.0);
    let r = Rect::new((ui.size.x - w) * 0.5, (ui.size.y - h) * 0.5, w, h);
    let narrow = w < 480.0;
    ui.panel(r);
    ui.text(
        "Plugin process permission",
        Vec2::new(r.x + 20.0, r.y + 32.0),
        if narrow { 14.0 } else { 21.0 },
        Weight::Bold,
        TEXT,
        Align::Left,
    );
    let footer = if narrow { 132.0 } else { 52.0 };
    let area = Rect::new(r.x + 20.0, r.y + 48.0, r.w - 40.0, r.h - footer - 60.0);
    let details = details(prompt, |key| omsi_ui::tr(key).into_owned());
    let details = wrapped(ui, &details, area.w - 20.0);
    ui.scroll_area(
        &format!("process-details-{}", prompt.id),
        area,
        &mut |ui, body| {
            ui.paragraph_as_is(
                &details,
                Vec2::new(body.x, body.y),
                body.w - 16.0,
                14.0,
                Weight::Regular,
                TEXT,
            )
        },
    );
    let bw = if narrow {
        r.w - 40.0
    } else {
        (r.w - 56.0) / 3.0
    };
    for (i, label, choice, style) in [
        (
            0,
            "Allow once",
            ProcessDecision::AllowOnce,
            ButtonKind::Normal,
        ),
        (
            1,
            "Always allow",
            ProcessDecision::AlwaysAllow,
            ButtonKind::Normal,
        ),
        (2, "Deny", ProcessDecision::Deny, ButtonKind::Primary),
    ] {
        if ui.button(
            &format!("process-{}-{i}", prompt.id),
            Rect::new(
                r.x + 20.0 + if narrow { 0.0 } else { (bw + 8.0) * i as f32 },
                r.y + r.h - footer + if narrow { 40.0 * i as f32 } else { 0.0 },
                bw,
                32.0,
            ),
            label,
            None,
            style,
        ) {
            return Some(choice);
        }
    }
    None
}
