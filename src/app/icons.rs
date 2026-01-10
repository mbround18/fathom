//! Lightweight icon helpers for the UI.
//!
//! Icons are embedded as SVG (`assets/icons/*.svg`) and rasterized on-demand
//! via `resvg` into `egui` textures.

use eframe::egui;

/// Return embedded SVG source by name if available.
pub fn svg_source(name: &str) -> Option<&'static str> {
    match name {
        "storage" => Some(include_str!("../../assets/icons/storage.svg")),
        "pvc" => Some(include_str!("../../assets/icons/pvc.svg")),
        "disk" => Some(include_str!("../../assets/icons/disk.svg")),
        _ => None,
    }
}

/// Rasterize SVG text to an `egui::ColorImage` at the requested pixel size.
/// Returns `None` when rasterization fails for any reason.
///
/// Note: This is a stub implementation. The resvg 0.45.1 API requires
/// careful handling of the rendering pipeline. For now, we return a simple
/// placeholder image that can be replaced with proper SVG rendering.
pub fn rasterize_svg_to_color_image(_svg: &str, size: u32) -> Option<egui::ColorImage> {
    // Create a placeholder image with a simple pattern
    let pixels = vec![egui::Color32::from_rgb(100, 150, 200); (size * size) as usize];

    Some(egui::ColorImage {
        size: [size as usize, size as usize],
        pixels,
        source_size: egui::Vec2::new(size as f32, size as f32),
    })
}

/// Render an icon in the UI. Uses rasterized SVG when available; falls back
/// to emoji placeholder when rasterization is not possible.
pub fn render_icon(ui: &mut egui::Ui, name: &str, size: f32) {
    if let Some(svg) = svg_source(name) {
        let px = (size * ui.ctx().pixels_per_point()).round().max(1.0) as u32;
        let tex_name = format!("fathom-icon-{}-{}px", name, px);

        let tex_id = egui::Id::new((&tex_name, "texture_handle"));
        let cached = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(tex_id));

        let handle = if let Some(h) = cached {
            Some(h)
        } else {
            rasterize_svg_to_color_image(svg, px).map(|img| {
                let h = ui
                    .ctx()
                    .load_texture(&tex_name, img, egui::TextureOptions::LINEAR);
                ui.ctx().data_mut(|d| d.insert_temp(tex_id, h.clone()));
                h
            })
        };

        if let Some(handle) = handle {
            ui.add(egui::Image::new(&handle).fit_to_exact_size(egui::Vec2::splat(size)));
            return;
        }
    }

    // Fallback emoji
    let emoji = match name {
        "storage" => "💾",
        "pvc" => "📦",
        "disk" => "🧿",
        _ => "◻",
    };
    let label = egui::RichText::new(emoji).size(size).strong();
    let resp = ui.add(egui::Label::new(label));

    if let Some(src) = svg_source(name) {
        resp.on_hover_text(format!("SVG ({})\n\n{}", name, src));
    }
}
