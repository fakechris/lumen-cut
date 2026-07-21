//! lumen-cut — main entry. Delegates everything to the library so tests can
//! construct pieces without booting a window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    // EnvFilter: RUST_LOG wins, otherwise default to info from lumen-cut + warn from others.
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lumen_cut=info,warn".into()),
        )
        .init();

    tracing::info!(version = lumen_cut::VERSION, "lumen-cut starting");
    lumen_cut::run();
}
