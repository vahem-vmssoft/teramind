fn main() {
    let dist = std::path::Path::new("../../dashboard/dist");
    if !dist.join("index.html").exists() {
        println!("cargo:warning=dashboard/dist/index.html missing; server will serve placeholder");
    }
    println!("cargo:rerun-if-changed=../../dashboard/dist");
}
