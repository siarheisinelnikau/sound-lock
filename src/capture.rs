//! Opt-in framebuffer capture of preview mode, without accessing audio hardware.

use std::{
    fs::File,
    io::BufWriter,
    time::{Duration, Instant},
};

pub fn update(ctx: &egui::Context) -> Result<bool, String> {
    let Some(path) = std::env::var_os("SOUND_LOCK_CAPTURE_TO").filter(|path| !path.is_empty())
    else {
        return Ok(false);
    };
    if ctx.input(|i| i.viewport().minimized == Some(true)) {
        return Ok(false);
    }
    let id = egui::Id::unique("sound-lock-visual-capture");
    if let Some(image) =
        ctx.data(|d| d.get_temp::<std::sync::Arc<egui::ColorImage>>(id.with("image")))
    {
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(path).map_err(|e| e.to_string())?),
            image.width() as u32,
            image.height() as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
        writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        return Ok(true);
    }
    let (started, requested) = ctx.data_mut(|d| {
        let started = *d.get_temp_mut_or_insert_with(id, Instant::now);
        (
            started,
            d.get_temp::<bool>(id.with("requested")).unwrap_or(false),
        )
    });
    if started.elapsed() > Duration::from_secs(8) {
        return Err("Framebuffer capture did not become ready within eight seconds".into());
    }
    if !requested && started.elapsed() >= Duration::from_millis(500) {
        let context = ctx.clone();
        ctx.request_screenshot(move |image| {
            context.data_mut(|d| d.insert_temp(id.with("image"), image));
            context.request_repaint();
        });
        ctx.data_mut(|d| d.insert_temp(id.with("requested"), true));
    }
    ctx.request_repaint_after(Duration::from_millis(40));
    Ok(false)
}
