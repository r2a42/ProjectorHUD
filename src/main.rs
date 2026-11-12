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

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ProjectorHUD")
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
