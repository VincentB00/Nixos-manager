// rust-embed needs the UI output folder to exist at compile time. Create a
// placeholder when the Angular build has not run yet (dev builds of the
// daemon alone), so `cargo build` always works.
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist/nixos-manager-ui/browser");
    if !dir.join("index.html").exists() {
        std::fs::create_dir_all(&dir).expect("create ui placeholder dir");
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><title>Nixos-manager</title><p>UI not built. Run <code>npm run build</code> in <code>ui/</code> or use <code>ng serve</code> with the API proxy.</p>",
        )
        .expect("write placeholder");
    }
    println!("cargo:rerun-if-changed=../../ui/dist/nixos-manager-ui/browser");
}
