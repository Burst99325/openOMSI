//! Live texttexture definitions. Script variable bindings are deliberately read-only.
mod apply;
#[cfg(test)]
mod tests;

use super::Request;
use crate::player::Player;
use omsi_model::TextTexture;
use omsi_ui::{Rect, Weight, kit::{ButtonKind, Ui}, paint::Align, theme::*};

#[derive(Default)]
pub(super) struct Editor {
    part: usize,
    id: usize,
    pub fonts: Option<Vec<String>>,
    draft: Option<Draft>,
}

impl Editor {
    pub fn reset(&mut self) { self.draft = None; }
}

struct Draft {
    key: (usize, usize),
    observed: TextTexture,
    numbers: [String; 6], // width, height, red, green, blue, grid
}

impl Draft {
    fn new(key: (usize, usize), def: &TextTexture) -> Self {
        Self { key, observed: def.clone(), numbers: [def.width.to_string(), def.height.to_string(), def.color[0].to_string(), def.color[1].to_string(), def.color[2].to_string(), def.grid.to_string()] }
    }

    fn read(&self, mut def: TextTexture) -> Option<TextTexture> {
        def.width = self.numbers[0].parse().ok()?;
        def.height = self.numbers[1].parse().ok()?;
        def.grid = self.numbers[5].parse().ok()?;
        for (i, c) in def.color.iter_mut().enumerate() { *c = self.numbers[i + 2].parse().ok()?; }
        valid(&def).then_some(def)
    }
}

fn valid(def: &TextTexture) -> bool {
    (1..=4096).contains(&def.width) && (1..=4096).contains(&def.height)
        && (0..=4096).contains(&def.grid) && (0..=5).contains(&def.orientation)
        && def.color.iter().all(|c| c.is_finite() && (0.0..=255.0).contains(c))
}

pub(super) fn definition(p: &Player, part: usize, id: usize) -> Option<&TextTexture> {
    let ty = if part == 0 { &p.vehicle.ty } else { &p.vehicle.trailers.get(part - 1)?.ty };
    ty.model.text_textures.get(id)
}

pub(super) fn block(def: &TextTexture) -> String {
    format!("[texttexture_enh]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n", def.variable, def.font, def.width, def.height, u8::from(def.full_color), def.color[0], def.color[1], def.color[2], def.orientation, def.grid)
}

fn field(ui: &mut Ui, key: &str, r: Rect, label: &str, value: &mut String) {
    ui.text_in(&omsi_ui::tr(label), Rect::new(r.x, r.y, r.w * 0.45, r.h), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
    ui.text_input(key, Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), value, "", None);
}

pub(super) fn draw(ui: &mut Ui, editor: &mut Editor, p: Option<&Player>, r: Rect, out: &mut Vec<Request>) {
    let Some(p) = p else { return };
    let parts: Vec<_> = std::iter::once(&p.vehicle.ty).chain(p.vehicle.trailers.iter().map(|t| &t.ty))
        .map(|ty| (ty.def.path.file_name().unwrap_or_default().to_string_lossy().into_owned(), ty.model.text_textures.as_slice())).collect();
    draw_parts(ui, editor, &parts, r, out);
}

fn draw_parts(ui: &mut Ui, editor: &mut Editor, parts: &[(String, &[TextTexture])], r: Rect, out: &mut Vec<Request>) {
    if parts.is_empty() { return; }
    ui.scroll_area("dt-texttextures", r, &mut |ui, area| {
        let mut y = area.y;
        let mut row = || { let row = Rect::new(area.x, y, area.w - 10.0, 30.0); y += 38.0; row };
        let labels: Vec<_> = parts.iter().enumerate().map(|(i, (name, _))| format!("{i}: {name}")).collect();
        editor.part = editor.part.min(parts.len() - 1);
        ui.select("dt-tt-part", row(), &mut editor.part, &labels);
        let definitions = parts[editor.part].1;
        if definitions.is_empty() {
            ui.text_in(&omsi_ui::tr("This vehicle has no texttextures"), row(), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            return y - area.y;
        }
        editor.id = editor.id.min(definitions.len() - 1);
        let ids: Vec<_> = definitions.iter().enumerate().map(|(i, t)| format!("ID {i}: {}", t.variable)).collect();
        ui.select("dt-tt-id", row(), &mut editor.id, &ids);
        let key = (editor.part, editor.id);
        let current = &definitions[editor.id];
        if editor.draft.as_ref().is_none_or(|d| d.key != key || d.observed != *current) {
            editor.draft = Some(Draft::new(key, current));
        }
        let draft = editor.draft.as_mut().unwrap();
        let mut def = current.clone();
        ui.text_in(&format!("Stringvar: {}", current.variable), row(), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        let mut fonts = editor.fonts.clone().unwrap_or_default();
        if !fonts.iter().any(|f| f == &def.font) { fonts.insert(0, def.font.clone()); }
        let mut font = fonts.iter().position(|f| f == &def.font).unwrap_or(0);
        let fr = row();
        ui.text_in(&omsi_ui::tr("Font"), Rect::new(fr.x, fr.y, 55.0, fr.h), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
        if ui.select("dt-tt-font", Rect::new(fr.x + 60.0, fr.y, fr.w - 60.0, fr.h), &mut font, &fonts) { def.font = fonts[font].clone(); }
        for (i, label) in ["Width (pixels)", "Height (pixels)", "Red (0–255)", "Green (0–255)", "Blue (0–255)", "Pixel grid"].iter().enumerate() {
            field(ui, &format!("dt-tt-num{i}"), row(), label, &mut draft.numbers[i]);
        }
        ui.toggle("dt-tt-full", row(), &mut def.full_color, &omsi_ui::tr("Use font colours"));
        let orientations: Vec<_> = ["Centred", "Left", "Right", "Centred (rounded)", "Centred (round down)", "Centred (round up)"].iter().enumerate().map(|(i, s)| format!("{i}: {}", omsi_ui::tr(s))).collect();
        let mut orientation = def.orientation.clamp(0, 5) as usize;
        if ui.select("dt-tt-align", row(), &mut orientation, &orientations) { def.orientation = orientation as i32; }
        if let Some(next) = draft.read(def) {
            if next != *current {
                draft.observed = next.clone();
                out.push(Request::ApplyTextTexture(editor.part, editor.id, next));
            }
        } else {
            ui.text_in(&omsi_ui::tr("Invalid values: size 1–4096, colour 0–255, grid 0–4096"), row(), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
        if ui.button("dt-tt-copy", row(), &omsi_ui::tr("Copy the block"), Some("content_copy"), ButtonKind::Primary) {
            out.push(Request::CopyTextTexture(editor.part, editor.id));
        }
        ui.text_in(&omsi_ui::tr("Live preview only. Reload model.cfg to restore the file."), row(), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
        y - area.y
    });
}
