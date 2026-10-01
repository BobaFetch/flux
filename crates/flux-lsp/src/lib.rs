//! Language servers as processes: starting them, and JSON-RPC messages over their stdio. What
//! the messages mean is up to the editor (`flux_view::lsp`, `flux_vim::lsp`).

pub mod config;
pub mod transport;

pub use config::{ServerConfig, builtin_configs, find_root};
pub use transport::{Event, Server, ServerId};
