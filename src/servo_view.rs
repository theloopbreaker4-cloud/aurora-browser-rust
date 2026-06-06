// Re-export engine/servo under the legacy `servo_view` module name
// so app.rs keeps calling crate::servo_view::ServoView etc. unchanged.
#[cfg(feature = "servo-engine")]
pub use crate::engine::servo::*;
