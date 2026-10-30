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
        eprintln!("{}", my_erp::theme::info("(mode mémoire : données perdues à la fermeture — démonstration)"));
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
            eprintln!("{}", my_erp::theme::erreur(format!("✗ {e}")));
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

    // Interface TUI plein écran (terminal) : `my-erp tui`
    if commande.iter().any(|a| a == "tui" || a == "--tui") {
        return match my_erp::tui::lancer(app.store.as_mut()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", my_erp::theme::erreur(format!("✗ TUI : {e}")));
                eprintln!("{}", my_erp::theme::info("  (utilisez le mode interactif classique : lancez le binaire sans argument)"));
                ExitCode::FAILURE
            }
        };
    }

    if commande.is_empty() {
        let stdin = std::io::stdin();
        let mut lecteur: BufReader<Stdin> = BufReader::new(stdin);
        match repl(&mut app, &mut lecteur, &mut sortie) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", my_erp::theme::erreur(format!("✗ {e}")));
                ExitCode::FAILURE
            }
        }
    } else {
        match app.executer_jetons(commande, &mut sortie) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", my_erp::theme::erreur(format!("✗ {e}")));
                ExitCode::FAILURE
            }
        }
    }
}
