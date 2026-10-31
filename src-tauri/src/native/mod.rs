//! Native sessions: terminals Maya hosts herself in pseudo-terminals.

pub mod host;
pub mod route;
pub mod screen;

use std::sync::LazyLock;

/// The terminals of every Native session.
pub static HOST: LazyLock<host::NativeHost> = LazyLock::new(|| host::NativeHost::new(host::BATCH));
