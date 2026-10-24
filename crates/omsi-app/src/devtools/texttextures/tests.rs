use super::*;
use glam::Vec2;

fn example() -> TextTexture {
    TextTexture { variable: "IBIS_cabindisplay".into(), font: "DIN".into(), width: 512, height: 64, full_color: false, color: [255.0, 128.0, 0.0], orientation: 4, grid: 2 }
}

#[test]
fn copied_block_is_a_model_definition_with_the_same_binding_and_all_fields() {
    let def = example();
    let parsed = omsi_model::Model::parse(&omsi_cfg::CfgFile::from_str("model.cfg", &block(&def)));
    assert_eq!(parsed.text_textures, [def]);
}

#[test]
fn incomplete_or_unsafe_numeric_input_does_not_apply() {
    let def = example();
    let mut draft = Draft::new((0, 0), &def);
    for (index, invalid) in [(0, ""), (0, "-1"), (1, "4097"), (2, "NaN"), (3, "256"), (5, "-1")] {
        draft.numbers[index] = invalid.into();
        assert!(draft.read(def.clone()).is_none());
        draft = Draft::new((0, 0), &def);
    }
}

#[test]
fn live_edits_cannot_replace_the_string_variable_binding() {
    let old = example();
    let edited = TextTexture { variable: "another_variable".into(), width: 128, font: "Arial".into(), ..old.clone() };
    let actual = apply::live_definition(&old, edited).unwrap();
    assert_eq!(actual.variable, old.variable);
    assert_eq!((actual.width, actual.font.as_str()), (128, "Arial"));
}

#[test]
fn a_live_edit_emits_once_and_a_model_reload_replaces_the_draft() {
    let def = example();
    let mut editor = Editor::default();
    editor.fonts = Some(vec!["DIN".into(), "Arial".into()]);
    let mut shell = crate::shell::Shell::new();
    let r = Rect::new(10.0, 10.0, 372.0, 700.0);
    let mut requests = Vec::new();
    shell.begin(1000.0, 800.0, 1.0, 0.016);
    draw_parts(&mut shell.ui, &mut editor, &[("Test.bus".into(), &[def.clone()])], r, &mut requests);
    shell.finish();
    assert!(requests.is_empty());
    editor.draft.as_mut().unwrap().numbers[0] = "256".into();
    shell.begin(1000.0, 800.0, 1.0, 0.016);
    draw_parts(&mut shell.ui, &mut editor, &[("Test.bus".into(), &[def.clone()])], r, &mut requests);
    shell.finish();
    let [Request::ApplyTextTexture(0, 0, updated)] = requests.as_slice() else { panic!("{requests:?}"); };
    assert_eq!(updated.width, 256);
    assert_eq!(updated.variable, def.variable);
    let updated = updated.clone();
    requests.clear();
    shell.begin(1000.0, 800.0, 1.0, 0.016);
    draw_parts(&mut shell.ui, &mut editor, &[("Test.bus".into(), &[updated])], r, &mut requests);
    shell.finish();
    assert!(requests.is_empty());
    // Reading model.cfg restores its 512px definition, not the unsaved UI edit.
    shell.begin(1000.0, 800.0, 1.0, 0.016);
    draw_parts(&mut shell.ui, &mut editor, &[("Test.bus".into(), &[def])], r, &mut requests);
    shell.finish();
    assert_eq!(editor.draft.as_ref().unwrap().numbers[0], "512");
    assert!(requests.is_empty());
}

#[test]
fn selecting_a_missing_id_or_an_empty_coupled_part_is_safe() {
    let def = example();
    let mut editor = Editor { part: 99, id: 99, ..Default::default() };
    let mut shell = crate::shell::Shell::new();
    for size in [Vec2::new(400.0, 720.0), Vec2::new(320.0, 240.0)] {
        shell.begin(size.x, size.y, 1.0, 0.016);
        let mut out = Vec::new();
        draw_parts(&mut shell.ui, &mut editor, &[("Test.bus".into(), &[def.clone()]), ("Trailer.bus".into(), &[])], Rect::new(0.0, 0.0, size.x, size.y), &mut out);
        shell.finish();
        assert!(out.is_empty());
        assert_eq!(editor.part, 1);
    }
}

#[test]
#[ignore = "requires GPU; exports the actual texttexture editor for visual review"]
fn export_texttexture_editor() {
    omsi_ui::i18n::set_lookup(|language, key| {
        crate::_rust_i18n_try_translate(language, key).map(|value| value.into_owned())
    });
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(omsi_render::Renderer::new_with(
        &instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        omsi_render::RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, ..Default::default() },
    )).expect("test renderer");
    let def = example();
    for (language, width, height) in [("en", 420, 780), ("hu", 420, 780), ("hu", 320, 300)] {
        omsi_ui::i18n::set_language(language);
        let mut editor = Editor { fonts: Some(vec!["Arial".into(), "DIN".into(), "Krueger 16x9".into()]), ..Default::default() };
        let mut shell = crate::shell::Shell::new();
        shell.begin(width as f32, height as f32, 1.0, 0.016);
        shell.ui.panel(Rect::new(0.0, 0.0, width as f32, height as f32));
        draw_parts(&mut shell.ui, &mut editor, &[("Citybus.bus".into(), &[def.clone()])], Rect::new(14.0, 14.0, width as f32 - 28.0, height as f32 - 28.0), &mut Vec::new());
        shell.finish();
        let rgba = shell.picture_for_shot(&renderer, width, height).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/review-validation/texttexture-editor-{language}-{width}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::save_buffer(&path, &rgba, width, height, image::ColorType::Rgba8).unwrap();
        println!("{}", path.display());
    }
}
