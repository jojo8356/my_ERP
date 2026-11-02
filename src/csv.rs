//! Import / export CSV — « des fichiers, pas des silos » (INT-03, ENF-PORT-01).
//!
//! - lecture tolérante : séparateur détecté (`;`, `,`, tabulation), guillemets
//!   doublés, BOM UTF-8, lignes vides ignorées ;
//! - en-têtes reconnues par synonymes français/anglais (`nom`, `client`,
//!   `designation`, `prix`, `montant`…) ;
//! - montants acceptés en centimes entiers ou en euros à virgule.
//!
//! Aucune dépendance : le format CSV est assez simple pour être écrit ici.

use crate::domain::Cents;
use std::path::Path;

pub type Resultat<T> = Result<T, String>;

#[derive(Debug, Clone, Default)]
pub struct Table {
    pub entetes: Vec<String>,
    pub lignes: Vec<Vec<String>>,
}

impl Table {
    /// Lit un CSV depuis un texte (séparateur détecté automatiquement).
    pub fn depuis_texte(texte: &str) -> Resultat<Table> {
        let texte = texte.strip_prefix('\u{feff}').unwrap_or(texte);
        let mut lignes_brutes: Vec<&str> = texte
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lignes_brutes.is_empty() {
            return Ok(Table::default());
        }
        let separateur = deviner_separateur(lignes_brutes[0]);
        let entetes: Vec<String> = decouper(lignes_brutes.remove(0), separateur)
            .into_iter()
            .map(|e| normaliser_entete(&e))
            .collect();
        let mut lignes = Vec::new();
        for brute in lignes_brutes {
            let champs = decouper(brute, separateur);
            if champs.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            lignes.push(champs);
        }
        Ok(Table { entetes, lignes })
    }

    pub fn depuis_fichier(chemin: &Path) -> Resultat<Table> {
        let texte = std::fs::read_to_string(chemin)
            .map_err(|e| format!("lecture de {} : {e}", chemin.display()))?;
        Table::depuis_texte(&texte)
    }

    /// Indice d'une colonne dont l'en-tête fait partie des synonymes.
    pub fn indice(&self, synonymes: &[&str]) -> Option<usize> {
        self.entetes
            .iter()
            .position(|e| synonymes.iter().any(|s| e == &normaliser_entete(s)))
    }

    /// Valeur d'une ligne pour un jeu de synonymes (chaîne vide si absente).
    pub fn valeur(&self, ligne: &[String], synonymes: &[&str]) -> String {
        self.indice(synonymes)
            .and_then(|i| ligne.get(i).cloned())
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn requis(&self, synonymes: &[&str]) -> Resultat<usize> {
        self.indice(synonymes).ok_or_else(|| {
            format!(
                "colonne obligatoire absente : {} (en-têtes trouvées : {})",
                synonymes.first().copied().unwrap_or("?"),
                self.entetes.join(", ")
            )
        })
    }
}

fn deviner_separateur(ligne: &str) -> char {
    let candidats = [';', ',', '\t', '|'];
    let mut meilleur = (';', 0usize);
    for candidat in candidats {
        let compte = ligne.matches(candidat).count();
        if compte > meilleur.1 {
            meilleur = (candidat, compte);
        }
    }
    meilleur.0
}

/// Découpe une ligne CSV en respectant les guillemets (`""` = guillemet).
pub fn decouper(ligne: &str, separateur: char) -> Vec<String> {
    let mut champs = Vec::new();
    let mut courant = String::new();
    let mut dans_guillemets = false;
    let mut caracteres = ligne.chars().peekable();
    while let Some(c) = caracteres.next() {
        match c {
            '"' if dans_guillemets => {
                if caracteres.peek() == Some(&'"') {
                    caracteres.next();
                    courant.push('"');
                } else {
                    dans_guillemets = false;
                }
            }
            '"' if courant.trim().is_empty() => {
                dans_guillemets = true;
                courant.clear();
            }
            c if c == separateur && !dans_guillemets => {
                champs.push(courant.trim().to_string());
                courant.clear();
            }
            c => courant.push(c),
        }
    }
    champs.push(courant.trim().to_string());
    champs
}

fn normaliser_entete(entete: &str) -> String {
    let brut = entete.trim().trim_matches('"').to_lowercase();
    let sans_accent: String = crate::domain::slug_recherche(&brut)
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .collect();
    sans_accent
}

/// Échappe un champ pour l'export (guillemets si nécessaire).
pub fn echapper(champ: &str) -> String {
    if champ.contains(';') || champ.contains('"') || champ.contains('\n') || champ.contains('\r') {
        format!("\"{}\"", champ.replace('"', "\"\""))
    } else {
        champ.to_string()
    }
}

/// Construit un CSV prêt à écrire (séparateur `;`, entêtes en première ligne).
pub fn exporter(entetes: &[&str], lignes: &[Vec<String>]) -> String {
    let mut sortie = String::new();
    sortie.push_str(
        &entetes
            .iter()
            .map(|e| echapper(e))
            .collect::<Vec<_>>()
            .join(";"),
    );
    sortie.push('\n');
    for ligne in lignes {
        sortie.push_str(
            &ligne
                .iter()
                .map(|v| echapper(v))
                .collect::<Vec<_>>()
                .join(";"),
        );
        sortie.push('\n');
    }
    sortie
}

/// Montant depuis un texte : « 1 234,56 € », « 42.5 », « 4250 » ou « 4250c ».
pub fn montant(texte: &str) -> Resultat<Cents> {
    let propre = texte.trim();
    if propre.is_empty() {
        return Ok(0);
    }
    if let Some(centimes) = propre.strip_suffix('c').or_else(|| propre.strip_suffix('C')) {
        let valeur: i64 = centimes
            .trim()
            .parse()
            .map_err(|_| format!("montant illisible : « {texte} »"))?;
        return Ok(valeur);
    }
    crate::domain::parse_eur(propre).map_err(|_| format!("montant illisible : « {texte} »"))
}

/// Quantité depuis un texte, en millièmes (« 2,5 » → 2500).
pub fn quantite_milli(texte: &str) -> Resultat<i64> {
    let propre = texte.trim().replace(' ', "").replace(',', ".");
    if propre.is_empty() {
        return Ok(1000);
    }
    let valeur: f64 = propre
        .parse()
        .map_err(|_| format!("quantité illisible : « {texte} »"))?;
    Ok((valeur * 1000.0).round() as i64)
}

/// Taux de TVA : accepte « 20 », « 20 % », « 2000bp », « 5,5 ».
pub fn taux_bp(texte: &str) -> Resultat<i32> {
    let propre = texte.trim().trim_end_matches('%').trim().replace(',', ".");
    if propre.is_empty() {
        return Ok(0);
    }
    if let Some(bp) = propre.strip_suffix("bp") {
        return bp
            .trim()
            .parse()
            .map_err(|_| format!("taux illisible : « {texte} »"));
    }
    let valeur: f64 = propre
        .parse()
        .map_err(|_| format!("taux illisible : « {texte} »"))?;
    Ok((valeur * 100.0).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_simple_point_virgule() {
        let table = Table::depuis_texte("Nom;Email;Type\nAtelier Dupont;a@b.fr;pro\nMartin;m@b.fr;particulier\n").unwrap();
        assert_eq!(table.entetes, vec!["nom", "email", "type"]);
        assert_eq!(table.lignes.len(), 2);
        assert_eq!(table.valeur(&table.lignes[0], &["nom", "client"]), "Atelier Dupont");
        assert_eq!(table.valeur(&table.lignes[1], &["type"]), "particulier");
    }

    #[test]
    fn guillemets_et_virgules() {
        let table = Table::depuis_texte("nom,ville\n\"Dupont, fils\",\"Nice\"\n\"Il a dit \"\"bonjour\"\"\",Paris\n").unwrap();
        assert_eq!(table.entetes, vec!["nom", "ville"]);
        assert_eq!(table.lignes[0][0], "Dupont, fils");
        assert_eq!(table.lignes[1][0], "Il a dit \"bonjour\"");
    }

    #[test]
    fn bom_et_lignes_vides() {
        let table = Table::depuis_texte("\u{feff}nom;email\n\nDupont;a@b.fr\n\n").unwrap();
        assert_eq!(table.lignes.len(), 1);
        assert_eq!(table.entetes[0], "nom");
    }

    #[test]
    fn export_puis_relecture() {
        let texte = exporter(
            &["nom", "email"],
            &[vec!["Dupont; fils".to_string(), "a@b.fr".to_string()]],
        );
        let table = Table::depuis_texte(&texte).unwrap();
        assert_eq!(table.lignes[0][0], "Dupont; fils");
    }

    #[test]
    fn montants_et_quantites() {
        assert_eq!(montant("1 234,56 €").unwrap(), 123_456);
        assert_eq!(montant("42.5").unwrap(), 4_250);
        assert_eq!(montant("4250c").unwrap(), 4_250);
        assert_eq!(montant("").unwrap(), 0);
        assert_eq!(quantite_milli("2,5").unwrap(), 2_500);
        assert_eq!(quantite_milli("3").unwrap(), 3_000);
        assert_eq!(taux_bp("20 %").unwrap(), 2_000);
        assert_eq!(taux_bp("5,5").unwrap(), 550);
        assert_eq!(taux_bp("2000bp").unwrap(), 2_000);
    }

    #[test]
    fn colonne_obligatoire_absente() {
        let table = Table::depuis_texte("nom\nDupont\n").unwrap();
        assert!(table.requis(&["email"]).is_err());
        assert!(table.requis(&["nom"]).is_ok());
    }
}
