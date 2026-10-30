//! Interface TUI (Terminal User Interface) — plein écran dans le terminal,
//! toujours aucune GUI (CDC §6). Rust 100 % standard : codes ANSI pour
//! l'affichage, `stty` pour le mode raw, lecture touche par touche.
//!
//! Lancement : `my-erp tui`
//! Touches : ↑/↓ ou j/k naviguer · Tab changer de module · Entrée ouvrir
//!           v valider · p payer (facture sélectionnée) · ? aide · q quitter

use crate::domain::*;
use crate::store::*;
use std::io::{Read, Write};

pub const MODULES: [&str; 5] = ["Tableau de bord", "Clients", "Factures", "Vinted", "Comptabilité"];

#[derive(Debug, Clone, PartialEq)]
pub struct VueTui {
    pub module: usize,
    pub selection: usize,
    pub aide: bool,
    pub message: String,
}

impl Default for VueTui {
    fn default() -> Self {
        Self {
            module: 0,
            selection: 0,
            aide: false,
            message: "Tab = module · ↑↓ = naviguer · ? = aide · q = quitter".to_string(),
        }
    }
}

/// Données affichables d'un module (lignes de liste + pied de page).
pub struct ContenuModule {
    pub lignes: Vec<String>,
    pub factures: Vec<Document>, // uniquement pour le module Factures (actions v/p)
}

pub fn charger_contenu(store: &mut dyn Store, module: usize) -> Result<ContenuModule, String> {
    let regime = store.societe()?.regime;
    match module {
        0 => {
            let annee = annee_de(&aujourdhui());
            let (ventes, services) = store.ca_annuel(annee)?;
            let etat = etat_seuils(ventes, services, SEUILS_2026);
            let mut lignes = vec![
                format!("CA ventes {annee}   : {}", format_eur(ventes)),
                format!("CA services {annee} : {}", format_eur(services)),
                format!("CA global          : {}", format_eur(ventes + services)),
            ];
            for a in &etat.alertes {
                lignes.push(format!("⚠ {a}"));
            }
            let impayes = store
                .documents(Some(TypeDocument::Facture))?
                .into_iter()
                .filter(|d| matches!(d.statut, StatutDocument::Valide | StatutDocument::PartiellementPaye))
                .count();
            lignes.push(format!("Factures en attente de paiement : {impayes}"));
            lignes.push(String::new());
            lignes.push("Astuce : vous pouvez aussi taper un chemin, ex. /factures FA-2026-0001 payer".to_string());
            Ok(ContenuModule { lignes, factures: Vec::new() })
        }
        1 => {
            let lignes = store
                .clients()?
                .iter()
                .map(|c| format!("{:>4}  {:<30} {:<14} {}", c.id, c.nom, c.type_client.as_str(), c.email))
                .collect();
            Ok(ContenuModule { lignes, factures: Vec::new() })
        }
        2 => {
            let factures = store.documents(Some(TypeDocument::Facture))?;
            let lignes = factures
                .iter()
                .map(|d| {
                    let t = d.totaux(regime);
                    format!(
                        "{} {:<14} {}  TTC {:>12}  payé {:>12}  ({})",
                        d.statut.symbole(),
                        d.numero,
                        d.date_doc,
                        format_eur(t.ttc),
                        format_eur(d.paye),
                        d.statut.as_str()
                    )
                })
                .collect();
            Ok(ContenuModule { lignes, factures })
        }
        3 => {
            let mut lignes: Vec<String> = store
                .articles()?
                .iter()
                .map(|a| format!("{:>4}  {:<14} {:<26} taille {:<4} achat {:>10}  [{}]", a.id, a.marque, a.description, a.taille, format_eur(a.prix_achat), a.statut))
                .collect();
            let ventes = store.ventes()?;
            let marge: Cents = ventes
                .iter()
                .map(|v| {
                    store
                        .article_par_id(v.article_id)
                        .map(|a| marge_nette(&a, v))
                        .unwrap_or(0)
                })
                .sum();
            lignes.push(String::new());
            lignes.push(format!("Ventes : {} · marge nette cumulée : {}", ventes.len(), format_eur(marge)));
            Ok(ContenuModule { lignes, factures: Vec::new() })
        }
        _ => {
            let annee = annee_de(&aujourdhui());
            let (ventes, services) = store.ca_annuel(annee)?;
            let e = etat_seuils(ventes, services, SEUILS_2026);
            let mut lignes = vec![
                format!("Micro-entreprise — activité mixte ({annee})"),
                format!("  CA ventes   : {}  (franchise TVA dès {})", format_eur(e.ca_ventes), format_eur(SEUILS_2026.franchise_tva_ventes)),
                format!("  CA services : {}  (plafond services {})", format_eur(e.ca_services), format_eur(SEUILS_2026.ca_services_max)),
                format!("  CA global   : {}  (plafond global {})", format_eur(e.ca_ventes + e.ca_services), format_eur(SEUILS_2026.ca_global_max)),
            ];
            if e.alertes.is_empty() {
                lignes.push("  ✓ Aucune alerte de seuil".to_string());
            }
            for a in &e.alertes {
                lignes.push(format!("  ⚠ {a}"));
            }
            lignes.push(String::new());
            lignes.push("Exports (en CLI) : /compta fec · /admin audit".to_string());
            Ok(ContenuModule { lignes, factures: Vec::new() })
        }
    }
}

const LARG: usize = 96;

/// Rend l'écran complet dans une chaîne (testable sans terminal).
pub fn rendre(vue: &VueTui, contenu: &ContenuModule, societe_nom: &str) -> String {
    let mut s = String::new();
    s.push_str("\u{1b}[H\u{1b}[2J"); // curseur maison + effacer
    // ── Cadre du titre ──────────────────────────────────────────────
    let bord = "─".repeat(LARG - 2);
    let titre = format!(" ERP — {societe_nom}");
    s.push_str(&format!("\u{1b}[37m┌{bord}┐\u{1b}[0m\r\n"));
    s.push_str(&format!(
        "\u{1b}[37m│\u{1b}[0m\u{1b}[1;37m {:<largeur$} \u{1b}[0m\u{1b}[37m│\u{1b}[0m\r\n",
        titre,
        largeur = LARG - 4
    ));
    // ── Onglets alignés en colonnes de largeur égale, dans le même cadre ──
    // 5 modules, cellules de 18 caractères : 5×18 + 6 bordures = 96.
    const CELL: usize = 18;
    let jonction_haut: String = (0..MODULES.len())
        .map(|_| "─".repeat(CELL))
        .collect::<Vec<_>>()
        .join("┬");
    s.push_str(&format!("\u{1b}[37m├{jonction_haut}┤\u{1b}[0m\r\n"));
    let mut rangee = String::from("\u{1b}[37m│\u{1b}[0m");
    for (i, m) in MODULES.iter().enumerate() {
        let centre = {
            let texte = if i == vue.module { format!("▸ {m}") } else { m.to_string() };
            let n = texte.chars().count();
            let gauche = (CELL - n) / 2;
            let droite = CELL - n - gauche;
            format!("{}{}{}", " ".repeat(gauche), texte, " ".repeat(droite))
        };
        if i == vue.module {
            rangee.push_str(&format!("\u{1b}[1;37m{centre}\u{1b}[0m"));
        } else {
            rangee.push_str(&centre);
        }
        rangee.push_str("\u{1b}[37m│\u{1b}[0m");
    }
    s.push_str(&format!("{rangee}\r\n"));
    let bas: String = (0..MODULES.len())
        .map(|_| "─".repeat(CELL))
        .collect::<Vec<_>>()
        .join("┴");
    s.push_str(&format!("\u{1b}[37m└{bas}┘\u{1b}[0m\r\n"));
    if vue.aide {
        for l in [
            "AIDE — touches de la TUI",
            "",
            "  Tab / Maj+Tab     module suivant / précédent",
            "  ↑ / ↓  ou  j / k  monter / descendre dans la liste",
            "  Entrée            (réservé : ouvrir l'élément)",
            "  v                 valider la facture sélectionnée (module Factures)",
            "  p                 payer le reste dû de la facture sélectionnée",
            "  r                 rafraîchir les données",
            "  ?                 afficher / masquer cette aide",
            "  q                 quitter la TUI (le CLI reste disponible)",
            "",
            "Créations et éditions fines : via le CLI, ex.",
            "  erp /clients nouveau --nom \"Dupont\" --type pro",
            "  erp /factures nouvelle --client dupont --ligne \"Prestation;1;300,00;2000\"",
        ] {
            s.push_str(&format!("{l}\r\n"));
        }
    } else if contenu.lignes.is_empty() {
        s.push_str("(aucun élément — créez-en via le CLI, puis r pour rafraîchir)\r\n");
    } else {
        for (i, l) in contenu.lignes.iter().enumerate() {
            if i == vue.selection && vue.module != 0 && vue.module != 4 && !l.trim().is_empty() {
                s.push_str(&format!("\u{1b}[1;95m▸ {}\u{1b}[0m\r\n", tronquer(l)));
            } else {
                s.push_str(&format!(" {}\r\n", tronquer(l)));
            }
        }
    }
    s.push_str(&format!("\u{1b}[37m{}\u{1b}[0m\r\n", "─".repeat(LARG)));
    s.push_str(&format!("\u{1b}[37m{}\u{1b}[0m", tronquer(&vue.message)));
    s
}

fn tronquer(s: &str) -> String {
    if s.chars().count() > LARG - 2 {
        let mut t: String = s.chars().take(LARG - 5).collect();
        t.push_str("...");
        t
    } else {
        s.to_string()
    }
}

// ------------------------------------------------------------ Mode raw (Unix)

struct GardeTerminal;

impl GardeTerminal {
    fn activer() -> Result<Self, String> {
        let statut = std::process::Command::new("stty")
            .args(["-F", "/dev/tty", "raw", "-echo"])
            .status()
            .map_err(|e| format!("stty indisponible ({e}) — la TUI requiert un terminal Unix"))?;
        if !statut.success() {
            return Err("impossible de passer le terminal en mode raw (pas un terminal interactif ?)".into());
        }
        Ok(Self)
    }
}

impl Drop for GardeTerminal {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty")
            .args(["-F", "/dev/tty", "sane"])
            .status();
    }
}

enum Touche {
    Haut,
    Bas,
    Tab,
    MajTab,
    Entree,
    Caractere(char),
    Autre,
}

fn lire_touche(entree: &mut impl Read) -> Option<Touche> {
    let mut buf = [0u8; 1];
    if entree.read(&mut buf).ok()? == 0 {
        return None;
    }
    match buf[0] {
        b'\t' => Some(Touche::Tab),
        b'\r' | b'\n' => Some(Touche::Entree),
        b'k' => Some(Touche::Haut),
        b'j' => Some(Touche::Bas),
        0x1b => {
            let mut seq = [0u8; 2];
            if entree.read(&mut seq).ok()? < 2 {
                return Some(Touche::Autre);
            }
            match (seq[0], seq[1] as char) {
                (b'[', 'A') => Some(Touche::Haut),
                (b'[', 'B') => Some(Touche::Bas),
                (b'[', 'Z') => Some(Touche::MajTab),
                _ => Some(Touche::Autre),
            }
        }
        c if c.is_ascii() && !c.is_ascii_control() => Some(Touche::Caractere(c as char)),
        _ => Some(Touche::Autre),
    }
}

/// Boucle principale de la TUI. Retourne une erreur si pas de terminal.
pub fn lancer(store: &mut dyn Store) -> Result<(), String> {
    let _garde = GardeTerminal::activer()?;
    let mut sortie = std::io::stdout();
    // Écran alternatif + masquer le curseur
    write!(sortie, "\u{1b}[?1049h\u{1b}[?25l").map_err(|e| e.to_string())?;
    let resultat = boucle(store, &mut sortie);
    write!(sortie, "\u{1b}[?25h\u{1b}[?1049l").map_err(|e| e.to_string())?;
    sortie.flush().map_err(|e| e.to_string())?;
    resultat
}

fn boucle(store: &mut dyn Store, sortie: &mut dyn Write) -> Result<(), String> {
    let mut vue = VueTui::default();
    let mut contenu = charger_contenu(store, vue.module)?;
    let nom = store.societe()?.nom;
    let stdin = std::io::stdin();
    let mut entree = stdin.lock();
    loop {
        let ecran = rendre(&vue, &contenu, &nom);
        write!(sortie, "{ecran}").map_err(|e| e.to_string())?;
        sortie.flush().map_err(|e| e.to_string())?;
        match lire_touche(&mut entree) {
            None => break,
            Some(Touche::Caractere('q')) => break,
            Some(Touche::Caractere('?')) => vue.aide = !vue.aide,
            Some(Touche::Caractere('r')) => {
                contenu = charger_contenu(store, vue.module)?;
                vue.message = "Données rafraîchies".to_string();
            }
            Some(Touche::Tab) => {
                vue.module = (vue.module + 1) % MODULES.len();
                vue.selection = 0;
                contenu = charger_contenu(store, vue.module)?;
            }
            Some(Touche::MajTab) => {
                vue.module = (vue.module + MODULES.len() - 1) % MODULES.len();
                vue.selection = 0;
                contenu = charger_contenu(store, vue.module)?;
            }
            Some(Touche::Haut) => {
                vue.selection = vue.selection.saturating_sub(1);
            }
            Some(Touche::Bas) => {
                if vue.selection + 1 < contenu.lignes.len() {
                    vue.selection += 1;
                }
            }
            Some(Touche::Caractere('v')) if vue.module == 2 => {
                if let Some(d) = contenu.factures.get(vue.selection).cloned() {
                    match store.changer_statut(&d.numero, StatutDocument::Valide) {
                        Ok(_) => {
                            let _ = store.journaliser("tui.validation", &d.numero);
                            vue.message = format!("✓ {} validée", d.numero);
                        }
                        Err(e) => vue.message = format!("✗ {e}"),
                    }
                    contenu = charger_contenu(store, vue.module)?;
                }
            }
            Some(Touche::Caractere('p')) if vue.module == 2 => {
                if let Some(d) = contenu.factures.get(vue.selection).cloned() {
                    let regime = store.societe()?.regime;
                    let reste = reste_a_payer(d.totaux(regime).ttc, d.paye);
                    if reste <= 0 {
                        vue.message = format!("{} est déjà soldée", d.numero);
                    } else {
                        match store.ajouter_paiement(&d.numero, reste, &aujourdhui(), "virement") {
                            Ok(_) => {
                                let _ = store.journaliser("tui.paiement", &d.numero);
                                vue.message = format!("✓ {} payée ({})", d.numero, format_eur(reste));
                            }
                            Err(e) => vue.message = format!("✗ {e}"),
                        }
                        contenu = charger_contenu(store, vue.module)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendu_contient_modules_et_lignes() {
        let vue = VueTui {
            module: 2,
            selection: 0,
            aide: false,
            message: "test".into(),
        };
        let contenu = ContenuModule {
            lignes: vec!["✓ FA-2026-0001  TTC 1 000,00 €".into()],
            factures: Vec::new(),
        };
        let ecran = rendre(&vue, &contenu, "Ma micro-entreprise");
        assert!(ecran.contains("Factures"));
        assert!(ecran.contains("FA-2026-0001"));
        assert!(ecran.contains("Ma micro-entreprise"));
        // Cadre du titre + onglets alignés en colonnes égales
        assert!(ecran.contains("┌"), "coin du cadre manquant");
        assert!(ecran.contains("┬"), "séparateur d'onglets manquant");
        assert!(ecran.contains("┴"), "bas des onglets manquant");
        assert!(ecran.contains("▸ Factures"), "onglet actif non marqué");
        assert!(ecran.contains("\u{1b}[35m"), "écriture violette du menu manquante");
        assert!(ecran.contains("\u{1b}[1;95m"), "écriture violet vif de la sélection manquante");
        assert!(!ecran.contains("\u{1b}[7m"), "surlignage en vidéo inverse encore présent");
        assert!(!ecran.contains("48;5;"), "fond coloré encore présent");
    }

    #[test]
    fn aide_affichee() {
        let vue = VueTui {
            aide: true,
            ..VueTui::default()
        };
        let contenu = ContenuModule { lignes: Vec::new(), factures: Vec::new() };
        let ecran = rendre(&vue, &contenu, "X");
        assert!(ecran.contains("AIDE"));
        assert!(ecran.contains("quitter"));
    }

    #[test]
    fn contenu_tableau_de_bord_en_memoire() {
        let mut s = Memoire::new();
        let c = charger_contenu(&mut s, 0).unwrap();
        assert!(c.lignes.iter().any(|l| l.contains("CA global")));
    }
}
