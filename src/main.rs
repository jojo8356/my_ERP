//! Point d'entrée de l'ERP CLI (Rust, aucune GUI — CDC-ERP-CLI.md).
//!
//! Utilisation :
//!   erp                          mode interactif (chemins + raccourcis)
//!   erp /factures                commande unique (scriptable, sortie propre)
//!   erp /clients nouveau --nom "Dupont" --type pro
//!   erp tui                      écran plein terminal (Ratatui)
//!
//! Données : dossier local (ADR-002 `tables/*.tsv` + `meta.tsv`), par défaut
//! `./donnees`, modifiable via `ERP_DONNEES_DIR` ou `--donnees <dossier>`.
//! `--memoire` = démonstration sans persistance. Aucune base de données n'est
//! requise : le schéma Rustorm (`rustorm/schema.rsm`) sert de contrat de
//! colonnes au moteur TSV (ADR-006).

use my_erp::cli::{repl, App};
use my_erp::theme;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn version() -> String {
    format!("my_ERP {} (CDC-ERP-CLI.md v1.1)", env!("CARGO_PKG_VERSION"))
}

fn aide_demarrage() -> &'static str {
    "\
usage : my-erp [options] [chemin [action] [options]]

options globales :
  --donnees <dossier>   dossier de données (défaut : ./donnees, ou ERP_DONNEES_DIR)
  --memoire             démonstration : rien n'est écrit sur le disque
  --societe <nom>       change de société pour la commande
  --json | --csv        sortie machine sur les listes
  --version, --aide     cette aide

sans argument    mode interactif (chemins relatifs, `aide`, `q` pour quitter)
avec un chemin   commande unique, code de sortie 0 (succès) ou 1 (erreur)
  ex. my-erp /factures ; my-erp /clients nouveau --nom \"Dupont\" --type pro
      my-erp /compta/fec ; my-erp tui"
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", version());
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--aide" || a == "--help" || a == "-h") {
        println!("{}", theme::violet(aide_demarrage()));
        return ExitCode::SUCCESS;
    }

    // Options de démarrage (le reste part au CLI).
    let mut memoire = false;
    let mut dossier: Option<PathBuf> = None;
    let mut base: Option<String> = None;
    let mut commande: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--memoire" => memoire = true,
            "--donnees" if i + 1 < args.len() => {
                dossier = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--bdd" | "--base" | "--database-url" if i + 1 < args.len() => {
                base = Some(args[i + 1].clone());
                i += 1;
            }
            a if a.starts_with("--bdd=") => base = Some(a["--bdd=".len()..].to_string()),
            a if a.starts_with("--base=") => base = Some(a["--base=".len()..].to_string()),
            autre => commande.push(autre.to_string()),
        }
        i += 1;
    }

    // La base peut aussi venir de l'environnement (ERP_DATABASE_URL) : c'est ce
    // que fait le script de recette et la CI.
    if base.is_none() {
        base = std::env::var("ERP_DATABASE_URL").ok().filter(|u| !u.trim().is_empty());
    }

    let mut app = if let Some(url) = base {
        #[cfg(feature = "postgres")]
        {
            match App::ouvrir_bdd(&url) {
                Ok(app) => app,
                Err(e) => {
                    eprintln!("{}", theme::erreur(format!("✗ {e}")));
                    return ExitCode::FAILURE;
                }
            }
        }
        #[cfg(not(feature = "postgres"))]
        {
            let _ = url;
            eprintln!(
                "{}",
                theme::erreur(
                    "✗ ce binaire est compilé sans la base de données PostgreSQL.\n  \
                     Reconstruisez avec : cargo build --features postgres\n  \
                     (sans base : laissez tomber --bdd et utilisez le dossier local)"
                )
            );
            return ExitCode::FAILURE;
        }
    } else if memoire {
        eprintln!(
            "{}",
            theme::info("(mode mémoire : données perdues à la fermeture — démonstration)")
        );
        App::memoire()
    } else {
        let dossier = dossier
            .or_else(|| std::env::var("ERP_DONNEES_DIR").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("donnees"));
        match App::ouvrir(&dossier) {
            Ok(app) => app,
            Err(e) => {
                eprintln!("{}", theme::erreur(format!("✗ {e}")));
                return ExitCode::FAILURE;
            }
        }
    };

    let mut sortie = std::io::stdout();

    // Interface TUI plein écran : `my-erp tui`
    if commande.iter().any(|a| a == "tui" || a == "--tui") {
        return match my_erp::tui::lancer(&mut app) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", theme::erreur(format!("✗ TUI : {e}")));
                eprintln!(
                    "{}",
                    theme::info("  (mode interactif classique : lancez le binaire sans argument)")
                );
                ExitCode::FAILURE
            }
        };
    }

    if commande.is_empty() {
        // REPL : invite `erp:/chemin>` et commandes relatives au chemin courant.
        let stdin = std::io::stdin();
        let mut lecteur = BufReader::new(stdin);
        let resultat = repl(&mut app, &mut lecteur, &mut sortie);
        let _ = sortie.flush();
        match resultat {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", theme::erreur(format!("✗ {e}")));
                ExitCode::FAILURE
            }
        }
    } else {
        let resultat = app.executer_jetons(commande, &mut sortie);
        let _ = sortie.flush();
        match resultat {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", theme::erreur(format!("✗ {e}")));
                ExitCode::FAILURE
            }
        }
    }
}
