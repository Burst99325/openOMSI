//! Keep text texture IDs stable across a developer model reload. Materials and their
//! switched variants can continue to reference the same IDs when dimensions change.
use super::*;

impl World {
    pub fn reload_vehicle_text_texture_slots(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        render: &mut VehicleRender,
        definitions: &[omsi_model::TextTexture],
    ) {
        let mut gpu = self.gpu.lock();
        for def in definitions.iter().skip(render.text_textures.len()) {
            let max = renderer.device.limits().max_texture_dimension_2d;
            let id = gpu.add_blank_mips(renderer, scene, (def.width.max(1) as u32).min(max), (def.height.max(1) as u32).min(max));
            render.text_textures.push(Some(id));
        }
        drop(gpu);
        // Retain removed slots until this vehicle is released, so a material referring to
        // a removed index becomes transparent and re-adding it reuses its old ID.
        let blank = omsi_texture::Image { width: 1, height: 1, rgba: vec![0; 4], has_alpha: true };
        let mut rebound = Vec::new();
        for id in render.text_textures.iter().skip(definitions.len()).flatten() {
            if renderer.update_texture_mips(scene, *id, &blank) { rebound.push(*id); }
        }
        renderer.rebind_textures(scene, &rebound);
    }
}
