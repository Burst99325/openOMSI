use super::*;

fn example() -> ProcessPrompt {
    ProcessPrompt {
        id: 7,
        plugin: "Example plugin".into(),
        entry: "C:/Games/openOMSI/Plugins/Example/main.lua".into(),
        file: "C:/Games/openOMSI/Plugins/Example/RenderHelper.exe".into(),
        args: vec!["--render".into(), "dashboard".into()],
        cwd: Some("C:/Games/openOMSI".into()),
    }
}

#[test]
fn showing_a_request_never_implicitly_answers_it() {
    let mut shell = crate::shell::Shell::new();
    for (w, h) in [(1280.0, 720.0), (640.0, 480.0), (320.0, 240.0)] {
        shell.begin(w, h, 1.0, 0.016);
        assert_eq!(draw(&mut shell.ui, &example()), None);
        shell.finish();
        assert!(shell.showing());
        assert!(shell.actions.is_empty());
    }
}

#[test]
fn permission_text_is_localized_without_translating_plugin_data() {
    let prompt = example();
    for language in ["hu", "de", "fr"] {
        let mut looked_up = Vec::new();
        // The translator accepts interface keys only; plugin data must never reach it.
        let text = dialog::details(&prompt, |key| {
            assert!(!key.contains("Example") && !key.contains("dashboard"));
            crate::_rust_i18n_try_translate(language, key)
                .expect(key)
                .into_owned()
        });
        assert!(text.contains(&prompt.file.to_string_lossy().to_string()));
        assert!(text.contains(&prompt.plugin));
        assert!(text.contains("dashboard"));
        for key in [
            "Plugin process permission",
            "Allow once",
            "Always allow",
            "Deny",
        ] {
            looked_up.push(crate::_rust_i18n_try_translate(language, key).expect(key));
        }
        assert_eq!(looked_up.len(), 4);
    }
}

#[test]
fn each_permission_button_requires_its_own_click() {
    for (index, expected) in [
        ProcessDecision::AllowOnce,
        ProcessDecision::AlwaysAllow,
        ProcessDecision::Deny,
    ]
    .into_iter()
    .enumerate()
    {
        let mut shell = crate::shell::Shell::new();
        // 680 x 440 dialog centred in a 1000 x 700 surface.
        shell.pointer(180.0 + (208.0 + 8.0) * index as f32 + 100.0, 534.0);
        shell.button(true);
        shell.begin(1000.0, 700.0, 1.0, 0.016);
        assert_eq!(draw(&mut shell.ui, &example()), None);
        shell.finish();
        shell.button(false);
        shell.begin(1000.0, 700.0, 1.0, 0.016);
        assert_eq!(draw(&mut shell.ui, &example()), Some(expected));
        shell.finish();
    }
}

#[test]
#[ignore = "requires GPU; exports the actual game permission dialog for visual review"]
fn export_process_permission_dialog() {
    omsi_ui::i18n::set_lookup(|language, key| {
        crate::_rust_i18n_try_translate(language, key).map(|v| v.into_owned())
    });
    omsi_ui::i18n::set_language("hu");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(omsi_render::Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        omsi_render::RenderOptions {
            msaa: 1,
            ssao: false,
            shadow_size: 1024,
            fxaa: false,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let mut shell = crate::shell::Shell::new();
    shell.begin(1000.0, 700.0, 1.0, 0.016);
    assert_eq!(draw(&mut shell.ui, &example()), None);
    shell.finish();
    let rgba = shell.picture_for_shot(&renderer, 1000, 700).unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/review-validation/process-permission-dialog.png");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    image::save_buffer(&output, &rgba, 1000, 700, image::ColorType::Rgba8).unwrap();
    println!("Permission dialog preview: {}", output.display());
}
