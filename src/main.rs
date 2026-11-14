#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ui;

fn main() -> eframe::Result {
    // `projectorhud --sim` runs the demo fleet headless, for testing other controllers.
    if std::env::args().any(|a| a == "--sim") {
        let sims: Vec<_> = projectorhud::sim::demo_fleet()
            .into_iter()
            .filter_map(|spec| {
                let name = spec.name.clone();
                match projectorhud::sim::spawn(spec) {
                    Ok(h) => {
                        println!("{name}: PJLink on 127.0.0.1:{}", h.port);
                        Some(h)
                    }
                    Err(e) => {
                        eprintln!("{name}: {e}");
                        None
                    }
                }
            })
            .collect();
        println!(
            "{} simulated projectors running. Ctrl-C to stop.",
            sims.len()
        );
        loop {
            std::thread::park();
        }
    }

    // The Mac picture keeps the margin every Dock icon has; elsewhere the
    // picture fills its square.
    let icon: &[u8] = if cfg!(target_os = "macos") {
        include_bytes!("../assets/icon/icon_512.png")
    } else {
        include_bytes!("../assets/icon/icon_256.png")
    };
    let icon = eframe::icon_data::from_png_bytes(icon).expect("the built-in icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ProjectorHUD")
            .with_icon(icon)
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([920.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ProjectorHUD",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
