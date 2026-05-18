use include_dir::{include_dir, Dir};

static DASHBOARD: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../dashboard/dist");

pub fn lookup(path: &str) -> Option<(&'static [u8], &'static str)> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (file, effective_path) = DASHBOARD
        .get_file(path)
        .map(|f| (f, path))
        .or_else(|| DASHBOARD.get_file("index.html").map(|f| (f, "index.html")))?;
    let content_type = match effective_path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    Some((file.contents(), content_type))
}
