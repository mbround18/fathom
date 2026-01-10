//! Lightweight icon helpers for the UI.
//!
//! Icons are embedded as SVG (`assets/icons/*.svg`) and rasterized on-demand
//! via `resvg` into `egui` textures.

use eframe::egui;

use resvg::tiny_skia;
use resvg::usvg;
use usvg::{TreeParsing, TreeTextToPath};

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
pub fn rasterize_svg_to_color_image(svg: &str, size: u32) -> Option<egui::ColorImage> {
    let size = size.max(1);

    // Parse with usvg.
    let opt = usvg::Options::default();
    let mut tree = usvg::Tree::from_str(svg, &opt).ok()?;

    // resvg requires text to be converted into paths.
    // This is best-effort; if fonts can't be loaded, conversion may be partial.
    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    tree.convert_text(&fontdb);

    let rtree = resvg::Tree::from_usvg(&tree);

    // Render into tiny-skia pixmap.
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;

    // Scale the SVG's intrinsic size into our target size.
    let svg_w = rtree.size.width().max(1.0);
    let svg_h = rtree.size.height().max(1.0);
    let sx = (size as f64 / svg_w) as f32;
    let sy = (size as f64 / svg_h) as f32;
    let transform = tiny_skia::Transform::from_scale(sx, sy);
    let mut pixmap_mut = pixmap.as_mut();
    rtree.render(transform, &mut pixmap_mut);

    // tiny-skia stores premultiplied RGBA; we must un-premultiply for egui
    let data = pixmap.data();
    let mut out: Vec<u8> = Vec::with_capacity((size * size * 4) as usize);

    for chunk in data.chunks_exact(4) {
        let r = chunk[0] as u32;
        let g = chunk[1] as u32;
        let b = chunk[2] as u32;
        let a = chunk[3] as u32;

        if a == 0 {
            out.push(0);
            out.push(0);
            out.push(0);
            out.push(0);
        } else {
            // Un-premultiply components
            let rf = ((r * 255) + (a / 2)) / a;
            let gf = ((g * 255) + (a / 2)) / a;
            let bf = ((b * 255) + (a / 2)) / a;
            out.push(rf as u8);
            out.push(gf as u8);
            out.push(bf as u8);
            out.push(a as u8);
        }
    }

    Some(egui::ColorImage::from_rgba_unmultiplied(
        [size as usize, size as usize],
        &out,
    ))
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
