//! Opens the Vital editor window for a few seconds and prints its size over
//! time (shows a window on screen). Run: cargo run --example editor_probe
fn main() {
    use daw_core::dsp::registry::ExternalDevices;
    use daw_lib::plugins::{Provider, manager, ui};
    ui::init_app();
    let vital = manager().list(true).into_iter().find(|p| p.name.eq_ignore_ascii_case("vital") && p.kind == "instrument").expect("Vital installed");
    let device = daw_core::model::DeviceState { id: "probe".into(), type_id: vital.id, enabled: true, ..Default::default() };
    let _inst = Provider.create_instrument(&device, 48_000.0, false).expect("instantiate");
    ui::with_pool(|| manager().open_editor("probe", "Vital — probe").expect("window"));
    for i in 0..10 {
        ui::pump_run_loop(0.2);
        manager().fit_editors();
        let (v, w) = manager().editor_sizes("probe").unwrap();
        println!("t={:.1}s window {:.0}x{:.0} Vital {:.0}x{:.0}", (i + 1) as f64 * 0.2, w.0, w.1, v.0, v.1);
    }
    // Ctrl+Tab cycling between a stand-in "main" window and the plugin window.
    let main = ui::test_window("main (stand-in)");
    ui::install_window_cycling(main, || manager().visible_editor_windows());
    let plugin = manager().visible_editor_windows()[0];
    let name = |w: usize| if w == main as usize { "main" } else if w == plugin { "Vital" } else { "other" };
    ui::activate_and_key_window();
    ui::pump_events(0.5);
    println!("key window: {}", name(ui::activate_and_key_window()));
    for _ in 0..3 {
        ui::post_ctrl_tab();
        ui::pump_events(0.4);
        println!("after Ctrl+Tab: {}", name(ui::activate_and_key_window()));
    }
    manager().close_editor("probe");
}
