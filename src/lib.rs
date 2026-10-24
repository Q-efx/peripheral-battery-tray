//! Battery levels for Razer mice and SteelSeries Arctis headsets.
//!
//! Protocol knowledge comes from OpenRazer (Razer) and HeadsetControl
//! (SteelSeries); see README.md for credits.

pub mod alert;
pub mod icon;
pub mod model;
pub mod razer;
pub mod razer_devices;
pub mod scan;
pub mod steelseries;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod tray;
