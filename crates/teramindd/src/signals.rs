use tokio::signal;

/// Resolves on SIGTERM / SIGINT (Unix) or Ctrl-C (Windows).
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use signal::unix::{signal as unix_signal, SignalKind};
        let mut term = unix_signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut intr = unix_signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = intr.recv() => {}
        }
    }
    #[cfg(windows)]
    {
        let _ = signal::ctrl_c().await;
    }
}
