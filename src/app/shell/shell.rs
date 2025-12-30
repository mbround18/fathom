use super::super::FathomApp;

/// Fixed egui shell that owns and renders all panels
///
/// The shell is responsible for layout only. Actual content rendering
/// is delegated to views and helper functions.
pub struct Shell;

impl Shell {
    /// Render the complete fixed UI layout
    ///
    /// This method is called once per frame from FathomApp::update()
    /// and orchestrates rendering all panels in the correct order.
    pub fn render(ctx: &egui::Context, app: &mut FathomApp) {
        // Render top bar (context/namespace selector)
        app.render_top_bar(ctx);

        // Render left navigation sidebar
        app.render_left_nav(ctx);

        // Render center panel (view dispatch)
        app.render_center(ctx);

        // Render right panel (details/tabs for selected resource)
        app.render_right_panel(ctx);

        // Render status log (bottom)
        app.render_bottom_status(ctx);

        // Render modals (overlays)
        app.render_modal(ctx);

        // Keep UI responsive during async operations
        ctx.request_repaint();
    }
}
