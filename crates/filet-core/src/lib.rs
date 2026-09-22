pub mod config;
pub mod daemon;
pub mod error;
pub mod executor;
pub mod model;
pub mod paths;
pub mod planner;
pub mod process;
pub mod snapshot;
pub mod state;

pub use error::{Error, Result};
pub use model::*;

pub fn schemas() -> serde_json::Value {
    serde_json::json!({"configuration":schemars::schema_for!(config::Config),"actionPlan":schemars::schema_for!(Plan)})
}
