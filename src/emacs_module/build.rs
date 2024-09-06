use std::{env, path::PathBuf};

fn main() {
    let bindings = bindgen::Builder::default()
        .header("vendor/emacs-29.4/emacs-module.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .unwrap();

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("emacs_module.rs"))
        .unwrap();
}
