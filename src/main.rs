use app::FathomApp;
use clap::Parser;
use crossbeam_channel::unbounded;
use std::thread;
use tracing_subscriber::EnvFilter;

mod app;
mod backend;
mod config;
mod mcp_server;
mod protocol;
pub mod sections;
use protocol::{BackendCmd, BackendEventEnvelope};

use crate::config::Args;

/// Fathom - Kubernetes Dashboard with MCP integration
fn main() -> eframe::Result<()> {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    let _ = rustls::crypto::ring::default_provider().install_default();

    init_tracing();

    let args = Args::parse();

    // Channels (UI thread never awaits; backend owns tokio runtime)
    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (backend_tx, backend_rx) = unbounded::<BackendCmd>();

    // Clone backend_tx for Ctrl+C handler
    let _backend_tx_shutdown = backend_tx.clone();

    // Handle Ctrl+C gracefully
    ctrlc::set_handler(move || {
        tracing::info!("Received Ctrl+C, shutting down...");
        // Channel will be closed when backend_tx is dropped, signaling backend to exit

        // Give backend 500ms to clean up, then force exit
        // (eframe window may still be open, blocking run_native())
        std::thread::sleep(std::time::Duration::from_millis(500));
        std::process::exit(0);
    })
    .expect("Error setting Ctrl+C handler");

    let backend_thread = thread::spawn(move || {
        backend::init(backend_rx, gui_tx);
    });

    // Only-MCP mode: skip GUI, block on backend thread
    if args.only_mcp {
        tracing::info!("Running in headless MCP-only mode");
        let _ = backend_thread.join();
        return Ok(());
    }

    // Normal mode: run GUI + backend
    let icon_data = load_icon();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(icon_data),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Fathom K8s Dashboard",
        native_options,
        Box::new(|_cc| Ok(Box::new(FathomApp::new(gui_rx, backend_tx)))),
    );

    // After GUI closes, wait for backend thread with a timeout
    // The FathomApp::on_exit() should have already sent Shutdown command
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(3);

    loop {
        if backend_thread.is_finished() {
            tracing::info!("Backend thread exited cleanly");
            let _ = backend_thread.join();
            break;
        }

        if start.elapsed() > timeout {
            tracing::warn!(
                "Backend thread did not exit after {:.1}s, force exiting",
                start.elapsed().as_secs_f64()
            );
            std::process::exit(1);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    result
}

fn load_icon() -> egui::IconData {
    let icon_path = std::path::Path::new("assets/icon.png");

    if icon_path.exists() {
        if let Ok(img) = image::open(icon_path) {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();

            return egui::IconData {
                rgba: pixels,
                width,
                height,
            };
        }
    }

    // Fallback: create a simple colored square as icon
    let size = 32;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            // Create a teal/cyan gradient matching the logo
            let r = (x * 180 / size) as u8;
            let g = (200 + (y * 55 / size)) as u8;
            let b = (180 + (x * 75 / size)) as u8;
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(255); // Alpha
        }
    }

    egui::IconData {
        rgba: pixels,
        width: size as u32,
        height: size as u32,
    }
}

fn init_tracing() {
    // Prefer `FATHOM_LOG` but fall back to the conventional `RUST_LOG`.
    // Example: `FATHOM_LOG=fathom=debug,kube=info`
    let filter = std::env::var("FATHOM_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_owned());

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact()
        .try_init();
}
