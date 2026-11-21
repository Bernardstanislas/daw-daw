//! Opens the editor window of an installed VST3 for a few seconds and prints
//! the plugin view / window sizes (shows a window on screen).
//! Run: cargo run --example vst3_editor_probe [plugin name, default "Surge XT"]
fn main() {
    use daw_core::dsp::registry::ExternalDevices;
    use daw_lib::plugins::{Provider, manager, ui, vst3};
    ui::init_app();
    let want = std::env::args().nth(1).unwrap_or_else(|| "Surge XT".into());
    let info = vst3::scan().into_iter().find(|p| p.name.eq_ignore_ascii_case(&want) && p.kind == "instrument").expect("VST3 installed");
    manager().list(true);
    let device = daw_core::model::DeviceState { id: "probe".into(), type_id: info.id, enabled: true, ..Default::default() };
    let _inst = Provider.create_instrument(&device, 48_000.0, false).expect("instantiate");
    ui::with_pool(|| manager().open_editor("probe", &format!("{want} (VST3) — probe")).expect("window"));
    for i in 0..15 {
        ui::pump_run_loop(0.2);
        manager().fit_editors();
        let (v, w) = manager().editor_sizes("probe").unwrap();
        println!("t={:.1}s window {:.0}x{:.0} view {:.0}x{:.0} visible windows: {}", (i + 1) as f64 * 0.2, w.0, w.1, v.0, v.1, manager().visible_editor_windows().len());
        if i == 8 {
            let shot = std::env::temp_dir().join("vst3-editor.png");
            let _ = std::process::Command::new("screencapture").arg("-x").arg(&shot).status();
            println!("screenshot: {}", shot.display());
        }
    }
    manager().close_editor("probe");
    println!("closed; editors left: {}", manager().has_editors());
}
