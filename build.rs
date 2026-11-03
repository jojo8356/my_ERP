//! Génération du client typé à la compilation depuis `rustorm/schema.rsm`.
//!
//! Le schéma est la source de vérité du modèle (CDC §8). Une faute dans le
//! `.rsm` fait échouer le build : le code ne peut pas dériver silencieusement
//! du modèle de données. Le client généré est inclus par `src/orm.rs`.

fn main() {
    println!("cargo:rerun-if-changed=rustorm/schema.rsm");
    println!("cargo:rerun-if-changed=build.rs");

    let chemin = "rustorm/schema.rsm";
    let source = std::fs::read_to_string(chemin)
        .unwrap_or_else(|e| panic!("{chemin} illisible : {e}"));

    let schema = match rustorm::schema::parse(&source) {
        Ok(s) => s,
        Err(e) => panic!("schéma Rustorm invalide ({chemin}) : {e}"),
    };

    let code = rustorm::schema::generate(&schema);
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR absent (build.rs hors cargo ?)");
    let destination = std::path::Path::new(&out_dir).join("rustorm_generated.rs");
    std::fs::write(&destination, code).unwrap_or_else(|e| panic!("écriture du client généré : {e}"));

    // Empreinte du schéma : affichée par `erp /admin schema` et utile pour
    // vérifier que le binaire correspond bien au modèle livré.
    let empreinte = rustorm::checksum::sha256(source.as_bytes());
    println!("cargo:rustc-env=RUSTORM_SCHEMA_CHECKSUM={}", &empreinte[..16]);
    println!(
        "cargo:warning=rustorm : {} modèles générés depuis {chemin}",
        schema.models.len()
    );
}
