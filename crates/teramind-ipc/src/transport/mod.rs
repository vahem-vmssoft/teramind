#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::{connect, default_socket_path, listen};
#[cfg(windows)]
pub use windows::{connect, default_socket_path, listen};
