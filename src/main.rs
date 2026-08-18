#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
#[cfg(feature = "ui-capture")]
mod capture;
mod config;
mod driver;
mod popup;
mod routing;
mod system;
mod theme;
mod tray;
mod update;

fn main() -> eframe::Result {
    if let Some(result) = update::helper_entry() {
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let startup = match update::Startup::from_args() {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };
    if std::env::args().any(|arg| arg == "--probe-routing") {
        if let Err(e) = routing::probe() {
            eprintln!("{e:#}");
        }
        return Ok(());
    }
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
            .with_title(popup::SETTINGS_TITLE)
            .with_inner_size(popup::SETTINGS_SIZE)
            .with_visible(std::env::args().any(|arg| arg == "--settings"))
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
        Box::new(move |cc| Ok(Box::new(app::Application::new(app::App::new(cc, startup))))),
    )
}
