use super::{definition, valid};
use crate::App;
use std::sync::Arc;

pub(super) fn live_definition(old: &omsi_model::TextTexture, mut edited: omsi_model::TextTexture) -> Option<omsi_model::TextTexture> {
    if !valid(&edited) { return None; }
    edited.variable = old.variable.clone();
    Some(edited)
}

impl App {
    pub(crate) fn devtools_edit_texttexture(&mut self, part: usize, id: usize, def: omsi_model::TextTexture) {
        let (Some(p), Some(world)) = (self.player.as_mut(), self.world.as_ref()) else { return };
        let Some(old) = definition(p, part, id) else { return };
        // Enforce the read-only binding here too, rather than relying on the UI.
        let Some(def) = live_definition(old, def) else { return };
        let atlas = world.fonts.lock().load(&def.font);
        let (ty, states) = if part == 0 {
            (&mut p.vehicle.ty, &mut p.vehicle.text_textures)
        } else {
            let Some(t) = p.vehicle.trailers.get_mut(part - 1) else { return };
            (&mut t.ty, &mut t.text_textures)
        };
        let Some(state) = states.get_mut(id) else { return };
        Arc::make_mut(ty).model.text_textures[id] = def.clone();
        *state = omsi_sim::texttex::TextTextureState::new(def, atlas);
    }

    /// Recreate CPU text states, retaining each GPU texture's ID. The normal view sync
    /// uploads unchanged strings again and rebinds materials if dimensions changed.
    pub(crate) fn devtools_reload_texttextures(&mut self) {
        let (Some(p), Some(world), Some(renderer), Some(scene)) = (
            self.player.as_mut(), self.world.as_ref(), self.renderer.as_ref(), self.scene.as_mut(),
        ) else { return };
        let decode = |path: &std::path::Path| omsi_texture::decode_file(path).ok().map(|i| (i.width, i.height, i.rgba));
        p.vehicle.init_text_textures(&mut world.fonts.lock(), &decode);
        world.reload_vehicle_text_texture_slots(renderer, scene, &mut p.render, &p.vehicle.ty.model.text_textures);
        for (part, render) in p.vehicle.trailers.iter_mut().zip(&mut p.trailer_renders) {
            // The existing soft reload handles the leading vehicle's model. Coupled
            // parts also need their own text definitions read from their model file.
            if let Some(path) = part.ty.def.model.as_ref().map(|m| omsi_cfg::resolve_path(part.ty.def.dir(), m)) {
                match omsi_model::Model::load(&path) {
                    Ok(model) => Arc::make_mut(&mut part.ty).model.text_textures = model.text_textures,
                    Err(e) => log::warn!("developer tools: {}: {e}", path.display()),
                }
            }
            part.init_text_textures(&mut world.fonts.lock(), &decode);
            world.reload_vehicle_text_texture_slots(renderer, scene, render, &part.ty.model.text_textures);
        }
    }
}
