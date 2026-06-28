fn main() {
    #[cfg(target_os = "windows")]
    println!("cargo:rustc-link-lib=dylib=Rstrtmgr");

    // Sans cette directive, Cargo applique sa politique de repli « rerun on any
    // package change » : tout edit d'un fichier sous le manifest (notamment les
    // CSV de data/, data_golden/, data/smoke/) relance ce build script et
    // recompile le crate. On restreint donc le rerun au seul fichier build.rs.
    println!("cargo:rerun-if-changed=build.rs");
}
