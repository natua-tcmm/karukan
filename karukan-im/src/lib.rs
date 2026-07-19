//! karukan-im: Japanese IME engine for the macOS InputMethodKit frontend.
//!
//! The macOS stdio JSON-RPC server lives in [`server`] and is built
//!   as the `karukan-imserver` binary, bundled inside `karukan-macos`.

pub mod config;
pub mod core;
pub mod server;

pub use core::engine::{EngineAction, EngineResult, InputMethodEngine};
pub use core::keycode::{KeyEvent, KeyModifiers, Keysym};
pub use core::state::InputState;
