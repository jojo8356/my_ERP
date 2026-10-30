//! Point d'entrée de l'ERP CLI (Rust, aucune GUI — CDC-ERP-CLI.md).
//!
//! Utilisation :
//!   erp                        mode interactif (chemins + raccourcis)
//!   erp /factures              commande unique (scriptable, sortie propre)
//!   erp /clients nouveau --nom "Dupont" --type pro
//!
//! Base de données : PostgreSQL via `ERP_DATABASE_URL`
//! (ex. postgres://user:pass@localhost:5432/erp). Sans cette variable, ou avec
//! `--memoire`, le mode mémoire sert à la démonstration et aux tests.

use my_erp::cli::{repl, App};
use my_erp::pg::PgStore;
use my_erp::store::{Memoire, Store};
use std::io::{BufReader, Stdin};
use std::process::ExitCode;

fn ouvrir_store(args: &[String]) -> Result<Box<dyn Store>, String> {
    if args.iter().any(|a| a == "--memoire") {
        eprintln!("(mode mémoire : données perdues à la fermeture — démonstration)");
        return Ok(Box::new(Memoire::new()));
    }
    match std::env::var("ERP_DATABASE_URL") {
        Ok(url) if !url.is_empty() => Ok(Box::new(PgStore::connect(&url)?)),
        _ => {
            eprintln!("(aucune base configurée : mode mémoire. Pour PostgreSQL :");
            eprintln!("  ERP_DATABASE_URL=postgres://user:pass@localhost:5432/erp erp …)");
            Ok(Box::new(Memoire::new()))
        }
    }
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
