#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod driver;
mod system;
mod tray;

fn main() -> eframe::Result {
    if std::env::args().any(|arg| arg == "--list-devices") {
        match audio::devices() {
            Ok(devices) => {
                for device in devices.outputs {
                    println!("OUT {} | {}", device.id, device.name);
                }
            }
            Err(e) => eprintln!("{e:#}"),
        }
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--probe-system") {
        if let Ok(devices) = audio::devices() {
            for device in devices.outputs.iter().filter(|d| !d.cable) {
                match system::probe(&device.id) {
                    Ok((volume, peak)) => println!(
                        "{}: volume {volume:.1} dB, pre-volume peak {peak:.5}",
                        device.name
                    ),
                    Err(e) => eprintln!("{}: {e:#}", device.name),
                }
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([420.0, 446.0])
            .with_resizable(false)
            .with_icon(eframe::egui::IconData {
                rgba: tray::icon_rgba(64),
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Sound Lock",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
