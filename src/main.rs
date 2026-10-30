//! Point d'entrée de l'ERP CLI (Rust, aucune GUI — CDC-ERP-CLI.md).
//!
//! Utilisation :
//!   erp                        mode interactif (chemins + raccourcis)
//!   erp /factures              commande unique (scriptable, sortie propre)
//!   erp /clients nouveau --nom "Dupont" --type pro
//!
//! Données : dossier local (ADR-002), par défaut `./donnees`,
//! modifiable via `ERP_DONNEES_DIR`. `--memoire` = démonstration sans persistance.
//! PostgreSQL arrivera comme autre implémentation du trait `Store` (CDC §5.1).

use my_erp::cli::{repl, App};
use my_erp::fichier::Fichier;
use my_erp::store::{Memoire, Store};
use std::io::{BufReader, Stdin};
use std::path::PathBuf;
use std::process::ExitCode;

fn ouvrir_store(args: &[String]) -> Result<Box<dyn Store>, String> {
    if args.iter().any(|a| a == "--memoire") {
        eprintln!("(mode mémoire : données perdues à la fermeture — démonstration)");
        return Ok(Box::new(Memoire::new()));
    }
    let dossier = std::env::var("ERP_DONNEES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("donnees"));
    Ok(Box::new(Fichier::ouvrir(&dossier)?))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = match ouvrir_store(&args) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App { store };
    let mut sortie = std::io::stdout();

    let commande: Vec<String> = args
        .iter()
        .filter(|a| *a != "--memoire")
        .cloned()
        .collect();

    if commande.is_empty() {
        let stdin = std::io::stdin();
        let mut lecteur: BufReader<Stdin> = BufReader::new(stdin);
        match repl(&mut app, &mut lecteur, &mut sortie) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("✗ {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        match app.executer(&commande.join(" "), &mut sortie) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("✗ {e}");
                ExitCode::FAILURE
            }
        }
    }
}
