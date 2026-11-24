#![doc = include_str!("../README.md")]

mod config;
mod embedded;
mod router;

pub use config::{SpaConfig, dev_base};
pub use router::spa_router;
