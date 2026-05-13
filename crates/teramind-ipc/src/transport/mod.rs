#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::{connect, listen, default_socket_path};
#[cfg(windows)]
pub use windows::{connect, listen, default_socket_path};
