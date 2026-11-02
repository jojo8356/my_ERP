//! Stockage local *piloté par le modèle Rustorm* (ADR-006).
//!
//! Une table du schéma `.rsm` = un fichier `donnees/tables/<table>.tsv`.
//! L'en-tête de chaque fichier est exactement la liste de colonnes générée
//! par Rustorm (`orm::colonnes`) : aucune liste de colonnes n'est recopiée à
//! la main, et une évolution du `.rsm` se répercute ici.
//!
//! Principes :
//! - écriture atomique (fichier temporaire + renommage) — ENF-FIA-01 ;
//! - lecture tolérante : une colonne absente du fichier (ancienne version)
//!   vaut vide, une colonne inconnue est ignorée → migration indolore ;
//! - aucune base de données : ce module est le « moteur » local, et le jour
//!   du passage à PostgreSQL il suffit de rejouer `orm::ddl_postgres()`.

use crate::domain::*;
use crate::orm;
use crate::store::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Resultat<T> = Result<T, String>;

// ------------------------------------------------------------ Échappement

fn echapper(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\t', "%09")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn desechapper(s: &str) -> String {
    s.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%09", "\t")
        .replace("%25", "%")
}

/// Échappement compact pour les listes encodées sur une seule colonne.
fn cage(s: &str) -> String {
    echapper(s).replace(';', "%3B").replace('|', "%7C")
}

fn decage(s: &str) -> String {
    desechapper(&s.replace("%3B", ";").replace("%7C", "|"))
}

// ------------------------------------------------------------ Lecture

/// Une ligne de table : accès par nom de colonne (jamais par position).
#[derive(Debug, Clone)]
pub struct LigneTable {
    colonnes: &'static [&'static str],
    valeurs: Vec<String>,
}

impl LigneTable {
    /// Construit une ligne depuis les valeurs brutes d'une autre source
    /// (PostgreSQL via `bdd::Bdd::charger`), dans l'ordre des colonnes du modèle.
    /// Les décodeurs sont ainsi **partagés** entre fichier et base.
    pub fn depuis_valeurs(table: &str, valeurs: Vec<String>) -> Resultat<Self> {
        let colonnes =
            orm::colonnes(table).ok_or_else(|| format!("table absente du schéma : {table}"))?;
        if valeurs.len() != colonnes.len() {
            return Err(format!(
                "table {table} : {} valeurs pour {} colonnes",
                valeurs.len(),
                colonnes.len()
            ));
        }
        Ok(Self { colonnes, valeurs })
    }

    pub fn texte(&self, colonne: &str) -> String {
        self.colonnes
            .iter()
            .position(|c| *c == colonne)
            .and_then(|i| self.valeurs.get(i).cloned())
            .unwrap_or_default()
    }

    pub fn entier(&self, colonne: &str) -> i64 {
        self.texte(colonne).trim().parse().unwrap_or(0)
    }

    pub fn entier32(&self, colonne: &str) -> i32 {
        self.texte(colonne).trim().parse().unwrap_or(0)
    }

    pub fn booleen(&self, colonne: &str) -> bool {
        matches!(self.texte(colonne).trim(), "1" | "oui" | "true" | "vrai")
    }
}

// ------------------------------------------------------------ Écriture

/// Constructeur de ligne nommé : impossible d'oublier ou d'inventer une
/// colonne sans que `fin()` le signale (les tests font l'aller-retour).
pub struct Ecrit {
    table: &'static str,
    colonnes: &'static [&'static str],
    valeurs: HashMap<String, String>,
}

impl Ecrit {
    pub fn pour(table: &'static str) -> Resultat<Self> {
        let colonnes =
            orm::colonnes(table).ok_or_else(|| format!("table absente du schéma : {table}"))?;
        Ok(Self {
            table,
            colonnes,
            valeurs: HashMap::new(),
        })
    }

    pub fn t(mut self, colonne: &str, valeur: impl Into<String>) -> Self {
        self.valeurs.insert(colonne.to_string(), valeur.into());
        self
    }

    pub fn n(self, colonne: &str, valeur: i64) -> Self {
        self.t(colonne, valeur.to_string())
    }

    pub fn i32(self, colonne: &str, valeur: i32) -> Self {
        self.t(colonne, valeur.to_string())
    }

    pub fn b(self, colonne: &str, valeur: bool) -> Self {
        self.t(colonne, if valeur { "1" } else { "0" })
    }

    pub fn fin(self) -> Resultat<Vec<String>> {
        let mut ligne = Vec::with_capacity(self.colonnes.len());
        let mut manquantes = Vec::new();
        for colonne in self.colonnes {
            match self.valeurs.get(*colonne) {
                Some(v) => ligne.push(v.clone()),
                None => manquantes.push(*colonne),
            }
        }
        if !manquantes.is_empty() {
            return Err(format!(
                "table {} : colonnes non renseignées : {}",
                self.table,
                manquantes.join(", ")
            ));
        }
        for cle in self.valeurs.keys() {
            if !self.colonnes.contains(&cle.as_str()) {
                return Err(format!(
                    "table {} : colonne « {cle} » absente du schéma",
                    self.table
                ));
            }
        }
        Ok(ligne)
    }
}

// ------------------------------------------------------------ Fichiers

pub fn dossier_tables(dossier: &Path) -> PathBuf {
    dossier.join("tables")
}

pub fn chemin_table(dossier: &Path, table: &str) -> PathBuf {
    dossier_tables(dossier).join(format!("{table}.tsv"))
}

pub fn ecrire_table(
    dossier: &Path,
    table: &'static str,
    lignes: &[Vec<String>],
) -> Resultat<()> {
    let colonnes =
        orm::colonnes(table).ok_or_else(|| format!("table absente du schéma : {table}"))?;
    std::fs::create_dir_all(dossier_tables(dossier))
        .map_err(|e| format!("création du dossier de données : {e}"))?;
    let mut contenu = String::new();
    contenu.push_str(&colonnes.join("\t"));
    contenu.push('\n');
    for ligne in lignes {
        if ligne.len() != colonnes.len() {
            return Err(format!(
                "table {table} : {} valeurs pour {} colonnes",
                ligne.len(),
                colonnes.len()
            ));
        }
        let champs: Vec<String> = ligne.iter().map(|v| echapper(v)).collect();
        contenu.push_str(&champs.join("\t"));
        contenu.push('\n');
    }
    let cible = chemin_table(dossier, table);
    let temporaire = dossier_tables(dossier).join(format!("{table}.tsv.tmp"));
    std::fs::write(&temporaire, contenu).map_err(|e| format!("écriture de {table} : {e}"))?;
    std::fs::rename(&temporaire, &cible).map_err(|e| format!("écriture de {table} : {e}"))?;
    Ok(())
}

pub fn lire_table(dossier: &Path, table: &str) -> Resultat<Vec<LigneTable>> {
    let colonnes =
        orm::colonnes(table).ok_or_else(|| format!("table absente du schéma : {table}"))?;
    let chemin = chemin_table(dossier, table);
    let contenu = match std::fs::read_to_string(&chemin) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("lecture de {} : {e}", chemin.display())),
    };
    let mut lignes = contenu.lines();
    let entete: Vec<String> = match lignes.next() {
        Some(l) => l.split('\t').map(desechapper).collect(),
        None => return Ok(Vec::new()),
    };
    let mut sortie = Vec::new();
    for brute in lignes {
        if brute.trim().is_empty() {
            continue;
        }
        let champs: Vec<String> = brute.split('\t').map(desechapper).collect();
        let mut valeurs = Vec::with_capacity(colonnes.len());
        for colonne in colonnes {
            let valeur = entete
                .iter()
                .position(|e| e == colonne)
                .and_then(|i| champs.get(i).cloned())
                .unwrap_or_default();
            valeurs.push(valeur);
        }
        sortie.push(LigneTable { colonnes, valeurs });
    }
    Ok(sortie)
}

/// Métadonnées du dossier de données (société courante, session, compteurs).
pub fn lire_meta(dossier: &Path) -> Resultat<HashMap<String, String>> {
    let chemin = dossier.join("meta.tsv");
    let mut meta = HashMap::new();
    let contenu = match std::fs::read_to_string(&chemin) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(meta),
        Err(e) => return Err(format!("lecture de {} : {e}", chemin.display())),
    };
    for ligne in contenu.lines() {
        if let Some((cle, valeur)) = ligne.split_once('\t') {
            meta.insert(desechapper(cle), desechapper(valeur));
        }
    }
    Ok(meta)
}

pub fn ecrire_meta(dossier: &Path, meta: &HashMap<String, String>) -> Resultat<()> {
    let mut cles: Vec<&String> = meta.keys().collect();
    cles.sort();
    let mut contenu = String::new();
    for cle in cles {
        contenu.push_str(&format!(
            "{}\t{}\n",
            echapper(cle),
            echapper(&meta[cle])
        ));
    }
    let cible = dossier.join("meta.tsv");
    let temporaire = dossier.join("meta.tsv.tmp");
    std::fs::write(&temporaire, contenu).map_err(|e| format!("écriture des métadonnées : {e}"))?;
    std::fs::rename(&temporaire, &cible).map_err(|e| format!("écriture des métadonnées : {e}"))?;
    Ok(())
}

// ------------------------------------------------------------ Listes encodees

/// Encode une liste de lignes sur une seule colonne (`recurrences.lignes`).
pub fn encoder_lignes(lignes: &[Ligne]) -> String {
    lignes
        .iter()
        .map(|l| {
            format!(
                "{};{};{};{};{};{}",
                cage(&l.designation),
                l.quantite_milli,
                l.prix_unitaire_ht,
                l.remise_bp,
                l.tva_bp,
                l.article_id
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

pub fn decoder_lignes(texte: &str) -> Vec<Ligne> {
    let mut sortie = Vec::new();
    for (position, brute) in texte.split('|').enumerate() {
        let champs: Vec<&str> = brute.split(';').collect();
        if champs.len() < 3 {
            continue;
        }
        let mut ligne = Ligne {
            position: position as i64,
            designation: decage(champs[0]),
            quantite_milli: champs[1].parse().unwrap_or(1000),
            prix_unitaire_ht: champs[2].parse().unwrap_or(0),
            remise_bp: champs.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            tva_bp: champs.get(4).and_then(|v| v.parse().ok()).unwrap_or(2000),
            article_id: champs.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
            ..Ligne::default()
        };
        ligne.id = 0;
        sortie.push(ligne);
    }
    sortie
}

pub fn encoder_photos(photos: &[String]) -> String {
    photos
        .iter()
        .map(|p| cage(p))
        .collect::<Vec<_>>()
        .join("|")
}

pub fn decoder_photos(texte: &str) -> Vec<String> {
    texte
        .split('|')
        .map(|p| decage(p))
        .filter(|p| !p.is_empty())
        .collect()
}

// ------------------------------------------------------------ Encodeurs

pub fn encoder_societe(s: &Societe) -> Resultat<Vec<String>> {
    Ecrit::pour("societes")?
        .n("id", s.id)
        .t("nom", &s.nom)
        .t("forme_juridique", &s.forme_juridique)
        .t("capital", &s.capital)
        .t("siren", &s.siren)
        .t("siret", &s.siret)
        .t("tva_intracom", &s.tva_intracom)
        .t("adresse", &s.adresse)
        .t("code_postal", &s.code_postal)
        .t("ville", &s.ville)
        .t("pays", &s.pays)
        .t("email", &s.email)
        .t("telephone", &s.telephone)
        .t("regime_tva", s.regime.as_str())
        .t("devise", &s.devise)
        .n("delai_paiement_jours", s.delai_paiement_jours)
        .n("penalites_taux_bp", s.penalites_taux_bp)
        .n("indemnite_forfaitaire", s.indemnite_forfaitaire)
        .t("cree_le", &s.cree_le)
        .fin()
}

pub fn decoder_societe(l: &LigneTable) -> Resultat<Societe> {
    Ok(Societe {
        id: l.entier("id"),
        nom: l.texte("nom"),
        forme_juridique: l.texte("forme_juridique"),
        capital: l.texte("capital"),
        siren: l.texte("siren"),
        siret: l.texte("siret"),
        tva_intracom: l.texte("tva_intracom"),
        adresse: l.texte("adresse"),
        code_postal: l.texte("code_postal"),
        ville: l.texte("ville"),
        pays: l.texte("pays"),
        email: l.texte("email"),
        telephone: l.texte("telephone"),
        regime: RegimeTva::from_str(&l.texte("regime_tva"))?,
        devise: {
            let d = l.texte("devise");
            if d.is_empty() {
                "EUR".to_string()
            } else {
                d
            }
        },
        delai_paiement_jours: l.entier("delai_paiement_jours"),
        penalites_taux_bp: l.entier("penalites_taux_bp"),
        indemnite_forfaitaire: l.entier("indemnite_forfaitaire"),
        cree_le: l.texte("cree_le"),
    })
}

pub fn encoder_utilisateur(u: &Utilisateur) -> Resultat<Vec<String>> {
    Ecrit::pour("utilisateurs")?
        .n("id", u.id)
        .t("nom_utilisateur", &u.nom_utilisateur)
        .t("nom_complet", &u.nom_complet)
        .t("email", &u.email)
        .t("role", u.role.code())
        .t("empreinte", &u.empreinte)
        .t("secret_totp", &u.secret_totp)
        .t("codes_recuperation", u.codes_recuperation.join(","))
        .t("code_connexion", &u.code_connexion)
        .n("code_connexion_expire", u.code_connexion_expire)
        .n("code_connexion_essais", u.code_connexion_essais)
        .b("actif", u.actif)
        .t("cree_le", &u.cree_le)
        .fin()
}

pub fn decoder_utilisateur(l: &LigneTable) -> Resultat<Utilisateur> {
    Ok(Utilisateur {
        id: l.entier("id"),
        nom_utilisateur: l.texte("nom_utilisateur"),
        nom_complet: l.texte("nom_complet"),
        email: l.texte("email"),
        role: Role::depuis_code(&l.texte("role")),
        empreinte: l.texte("empreinte"),
        secret_totp: l.texte("secret_totp"),
        codes_recuperation: l
            .texte("codes_recuperation")
            .split(',')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect(),
        code_connexion: l.texte("code_connexion"),
        code_connexion_expire: l.entier("code_connexion_expire"),
        code_connexion_essais: l.entier("code_connexion_essais"),
        actif: l.booleen("actif"),
        cree_le: l.texte("cree_le"),
    })
}

pub fn encoder_client(c: &Client) -> Resultat<Vec<String>> {
    Ecrit::pour("clients")?
        .n("id", c.id)
        .n("societe_id", c.societe_id)
        .t("nom", &c.nom)
        .t("type_client", c.type_client.code())
        .t("email", &c.email)
        .t("telephone", &c.telephone)
        .t("adresse", &c.adresse)
        .t("code_postal", &c.code_postal)
        .t("ville", &c.ville)
        .t("pays", &c.pays)
        .t("siren", &c.siren)
        .t("tva_intracom", &c.tva_intracom)
        .t("notes", &c.notes)
        .t("etiquettes", etiquettes_vers_texte(&c.etiquettes))
        .t("reference_externe", &c.reference_externe)
        .t("rgpd_efface_le", &c.rgpd_efface_le)
        .t("cree_le", &c.cree_le)
        .t("modifie_le", &c.modifie_le)
        .fin()
}

pub fn decoder_client(l: &LigneTable) -> Resultat<Client> {
    Ok(Client {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        nom: l.texte("nom"),
        type_client: TypeClient::depuis_code(&l.texte("type_client")),
        email: l.texte("email"),
        telephone: l.texte("telephone"),
        adresse: l.texte("adresse"),
        code_postal: l.texte("code_postal"),
        ville: l.texte("ville"),
        pays: l.texte("pays"),
        siren: l.texte("siren"),
        tva_intracom: l.texte("tva_intracom"),
        notes: l.texte("notes"),
        etiquettes: etiquettes_depuis_texte(&l.texte("etiquettes")),
        reference_externe: l.texte("reference_externe"),
        rgpd_efface_le: l.texte("rgpd_efface_le"),
        cree_le: l.texte("cree_le"),
        modifie_le: l.texte("modifie_le"),
    })
}

pub fn encoder_document(d: &Document) -> Resultat<Vec<String>> {
    Ecrit::pour("documents")?
        .n("id", d.id)
        .n("societe_id", d.societe_id)
        .t("numero", &d.numero)
        .t("type_doc", d.type_doc.as_str())
        .t("statut", d.statut.code())
        .n("client_id", d.client_id)
        .t("date_doc", &d.date_doc)
        .t("date_prestation", &d.date_prestation)
        .t("echeance", &d.echeance)
        .t("activite", &d.activite)
        .t("devise", &d.devise)
        .n("taux_change_micro", d.taux_change_micro)
        .t("conditions_paiement", &d.conditions_paiement)
        .n("delai_paiement_jours", d.delai_paiement_jours)
        .n("penalites_taux_bp", d.penalites_taux_bp)
        .n("indemnite_forfaitaire", d.indemnite_forfaitaire)
        .n("paye", d.paye)
        .t("envoye_le", &d.envoye_le)
        .t("motif", &d.motif)
        .t("cree_le", &d.cree_le)
        .fin()
}

pub fn decoder_document(l: &LigneTable) -> Resultat<Document> {
    Ok(Document {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        numero: l.texte("numero"),
        type_doc: TypeDocument::from_str(&l.texte("type_doc"))?,
        statut: StatutDocument::depuis_code(&l.texte("statut")),
        client_id: l.entier("client_id"),
        date_doc: l.texte("date_doc"),
        date_prestation: l.texte("date_prestation"),
        echeance: l.texte("echeance"),
        activite: {
            let a = l.texte("activite");
            if a.is_empty() {
                "services".to_string()
            } else {
                a
            }
        },
        devise: {
            let d = l.texte("devise");
            if d.is_empty() {
                DEVISE_REFERENCE.to_string()
            } else {
                d
            }
        },
        taux_change_micro: {
            let t = l.entier("taux_change_micro");
            if t == 0 {
                1_000_000
            } else {
                t
            }
        },
        conditions_paiement: l.texte("conditions_paiement"),
        delai_paiement_jours: l.entier("delai_paiement_jours"),
        penalites_taux_bp: l.entier("penalites_taux_bp"),
        indemnite_forfaitaire: l.entier("indemnite_forfaitaire"),
        lignes: Vec::new(),
        paye: l.entier("paye"),
        envoye_le: l.texte("envoye_le"),
        motif: l.texte("motif"),
        cree_le: l.texte("cree_le"),
    })
}

pub fn encoder_ligne(l: &Ligne) -> Resultat<Vec<String>> {
    Ecrit::pour("lignes")?
        .n("id", l.id)
        .n("document_id", l.document_id)
        .n("position", l.position)
        .t("designation", &l.designation)
        .n("quantite_milli", l.quantite_milli)
        .n("prix_unitaire_ht", l.prix_unitaire_ht)
        .i32("remise_bp", l.remise_bp)
        .i32("tva_bp", l.tva_bp)
        .n("article_id", l.article_id)
        .fin()
}

pub fn decoder_ligne(l: &LigneTable) -> Resultat<Ligne> {
    Ok(Ligne {
        id: l.entier("id"),
        document_id: l.entier("document_id"),
        position: l.entier("position"),
        designation: l.texte("designation"),
        quantite_milli: l.entier("quantite_milli"),
        prix_unitaire_ht: l.entier("prix_unitaire_ht"),
        remise_bp: l.entier32("remise_bp"),
        tva_bp: l.entier32("tva_bp"),
        article_id: l.entier("article_id"),
    })
}

pub fn encoder_paiement(p: &Paiement) -> Resultat<Vec<String>> {
    Ecrit::pour("paiements")?
        .n("id", p.id)
        .n("document_id", p.document_id)
        .t("date_paiement", &p.date_paiement)
        .n("montant", p.montant)
        .t("moyen", &p.moyen)
        .t("reference", &p.reference)
        .b("rapproche", p.rapproche)
        .fin()
}

pub fn decoder_paiement(l: &LigneTable) -> Resultat<Paiement> {
    Ok(Paiement {
        id: l.entier("id"),
        document_id: l.entier("document_id"),
        date_paiement: l.texte("date_paiement"),
        montant: l.entier("montant"),
        moyen: l.texte("moyen"),
        reference: l.texte("reference"),
        rapproche: l.booleen("rapproche"),
    })
}

pub fn encoder_ecriture(e: &Ecriture) -> Resultat<Vec<String>> {
    Ecrit::pour("ecritures")?
        .n("id", e.id)
        .n("societe_id", e.societe_id)
        .t("journal", &e.journal)
        .n("ecriture_num", e.ecriture_num)
        .t("date_ecriture", &e.date_ecriture)
        .t("compte", &e.compte)
        .t("libelle", &e.libelle)
        .n("debit", e.debit)
        .n("credit", e.credit)
        .t("piece_ref", &e.piece_ref)
        .t("piece_date", &e.piece_date)
        .t("comp_aux_num", &e.comp_aux_num)
        .t("comp_aux_lib", &e.comp_aux_lib)
        .t("devise", &e.devise)
        .t("lettrage", &e.lettrage)
        .n("document_id", e.document_id)
        .n("paiement_id", e.paiement_id)
        .fin()
}

pub fn decoder_ecriture(l: &LigneTable) -> Resultat<Ecriture> {
    Ok(Ecriture {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        journal: l.texte("journal"),
        ecriture_num: l.entier("ecriture_num"),
        date_ecriture: l.texte("date_ecriture"),
        compte: l.texte("compte"),
        libelle: l.texte("libelle"),
        debit: l.entier("debit"),
        credit: l.entier("credit"),
        piece_ref: l.texte("piece_ref"),
        piece_date: l.texte("piece_date"),
        comp_aux_num: l.texte("comp_aux_num"),
        comp_aux_lib: l.texte("comp_aux_lib"),
        devise: l.texte("devise"),
        lettrage: l.texte("lettrage"),
        document_id: l.entier("document_id"),
        paiement_id: l.entier("paiement_id"),
    })
}

pub fn encoder_article(a: &Article) -> Resultat<Vec<String>> {
    Ecrit::pour("articles")?
        .n("id", a.id)
        .n("societe_id", a.societe_id)
        .t("marque", &a.marque)
        .t("description", &a.description)
        .t("taille", &a.taille)
        .t("composition", &a.composition)
        .t("etat", &a.etat)
        .n("prix_achat", a.prix_achat)
        .t("date_achat", &a.date_achat)
        .t("statut", &a.statut)
        .t("regime", a.regime.as_str())
        .t("photos", encoder_photos(&a.photos))
        .t("sku", &a.sku)
        .n("quantite", a.quantite)
        .t("cree_le", &a.cree_le)
        .fin()
}

pub fn decoder_article(l: &LigneTable) -> Resultat<Article> {
    Ok(Article {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        marque: l.texte("marque"),
        description: l.texte("description"),
        taille: l.texte("taille"),
        composition: l.texte("composition"),
        etat: l.texte("etat"),
        prix_achat: l.entier("prix_achat"),
        date_achat: l.texte("date_achat"),
        statut: l.texte("statut"),
        regime: RegimeVente::from_str(&l.texte("regime")).unwrap_or(RegimeVente::AchatRevente),
        photos: decoder_photos(&l.texte("photos")),
        sku: l.texte("sku"),
        quantite: l.entier("quantite"),
        cree_le: l.texte("cree_le"),
    })
}

pub fn encoder_vente(v: &Vente) -> Resultat<Vec<String>> {
    Ecrit::pour("ventes")?
        .n("id", v.id)
        .n("societe_id", v.societe_id)
        .n("article_id", v.article_id)
        .t("date_vente", &v.date_vente)
        .n("prix_vente", v.prix_vente)
        .n("frais", v.frais)
        .t("plateforme", &v.plateforme)
        .t("acheteur", &v.acheteur)
        .t("reference", &v.reference)
        .t("regime", v.regime.as_str())
        .t("cree_le", &v.cree_le)
        .fin()
}

pub fn decoder_vente(l: &LigneTable) -> Resultat<Vente> {
    Ok(Vente {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        article_id: l.entier("article_id"),
        date_vente: l.texte("date_vente"),
        prix_vente: l.entier("prix_vente"),
        frais: l.entier("frais"),
        plateforme: l.texte("plateforme"),
        acheteur: l.texte("acheteur"),
        reference: l.texte("reference"),
        regime: RegimeVente::from_str(&l.texte("regime")).unwrap_or(RegimeVente::AchatRevente),
        cree_le: l.texte("cree_le"),
    })
}

pub fn encoder_taux(t: &TauxChange) -> Resultat<Vec<String>> {
    Ecrit::pour("taux_change")?
        .n("id", t.id)
        .t("devise", &t.devise)
        .t("date_taux", &t.date_taux)
        .n("taux_micro", t.taux_micro)
        .t("source", &t.source)
        .fin()
}

pub fn decoder_taux(l: &LigneTable) -> Resultat<TauxChange> {
    Ok(TauxChange {
        id: l.entier("id"),
        devise: l.texte("devise"),
        date_taux: l.texte("date_taux"),
        taux_micro: l.entier("taux_micro"),
        source: l.texte("source"),
    })
}

pub fn encoder_operation(o: &OperationBancaire) -> Resultat<Vec<String>> {
    Ecrit::pour("operations_bancaires")?
        .n("id", o.id)
        .n("societe_id", o.societe_id)
        .t("date_operation", &o.date_operation)
        .t("libelle", &o.libelle)
        .n("montant", o.montant)
        .t("reference", &o.reference)
        .n("paiement_id", o.paiement_id)
        .fin()
}

pub fn decoder_operation(l: &LigneTable) -> Resultat<OperationBancaire> {
    Ok(OperationBancaire {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        date_operation: l.texte("date_operation"),
        libelle: l.texte("libelle"),
        montant: l.entier("montant"),
        reference: l.texte("reference"),
        paiement_id: l.entier("paiement_id"),
    })
}

pub fn encoder_audit(e: &EntreeAudit) -> Resultat<Vec<String>> {
    Ecrit::pour("audit_log")?
        .n("id", e.id)
        .n("societe_id", e.societe_id)
        .t("horodatage", &e.horodatage)
        .t("utilisateur", &e.utilisateur)
        .t("action", &e.action)
        .t("detail", &e.detail)
        .t("empreinte_precedente", &e.empreinte_precedente)
        .t("empreinte", &e.empreinte)
        .fin()
}

pub fn decoder_audit(l: &LigneTable) -> Resultat<EntreeAudit> {
    Ok(EntreeAudit {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        horodatage: l.texte("horodatage"),
        utilisateur: l.texte("utilisateur"),
        action: l.texte("action"),
        detail: l.texte("detail"),
        empreinte_precedente: l.texte("empreinte_precedente"),
        empreinte: l.texte("empreinte"),
    })
}

pub fn encoder_sequence(s: &Sequence) -> Resultat<Vec<String>> {
    Ecrit::pour("sequences")?
        .n("id", s.id)
        .n("societe_id", s.societe_id)
        .t("type_doc", &s.type_doc)
        .n("annee", s.annee)
        .n("valeur", s.valeur)
        .fin()
}

pub fn decoder_sequence(l: &LigneTable) -> Resultat<Sequence> {
    Ok(Sequence {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        type_doc: l.texte("type_doc"),
        annee: l.entier("annee"),
        valeur: l.entier("valeur"),
    })
}

pub fn encoder_sauvegarde(s: &Sauvegarde) -> Resultat<Vec<String>> {
    Ecrit::pour("sauvegardes")?
        .n("id", s.id)
        .n("societe_id", s.societe_id)
        .t("date_sauvegarde", &s.date_sauvegarde)
        .t("chemin", &s.chemin)
        .t("empreinte", &s.empreinte)
        .n("taille_octets", s.taille_octets)
        .fin()
}

pub fn decoder_sauvegarde(l: &LigneTable) -> Resultat<Sauvegarde> {
    Ok(Sauvegarde {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        date_sauvegarde: l.texte("date_sauvegarde"),
        chemin: l.texte("chemin"),
        empreinte: l.texte("empreinte"),
        taille_octets: l.entier("taille_octets"),
    })
}

pub fn encoder_recurrence(r: &Recurrence) -> Resultat<Vec<String>> {
    Ecrit::pour("recurrences")?
        .n("id", r.id)
        .n("societe_id", r.societe_id)
        .n("client_id", r.client_id)
        .t("type_doc", r.type_doc.as_str())
        .t("activite", &r.activite)
        .t("periodicite", &r.periodicite)
        .t("prochaine_le", &r.prochaine_le)
        .t("lignes", encoder_lignes(&r.lignes))
        .b("actif", r.actif)
        .t("libelle", &r.libelle)
        .fin()
}

pub fn decoder_recurrence(l: &LigneTable) -> Resultat<Recurrence> {
    Ok(Recurrence {
        id: l.entier("id"),
        societe_id: l.entier("societe_id"),
        client_id: l.entier("client_id"),
        type_doc: TypeDocument::from_str(&l.texte("type_doc"))?,
        activite: l.texte("activite"),
        periodicite: l.texte("periodicite"),
        prochaine_le: l.texte("prochaine_le"),
        lignes: decoder_lignes(&l.texte("lignes")),
        actif: l.booleen("actif"),
        libelle: l.texte("libelle"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier_test(nom: &str) -> PathBuf {
        let dossier = std::env::temp_dir().join(format!(
            "erp-fichier-{nom}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dossier);
        dossier
    }

    #[test]
    fn echappement() {
        assert_eq!(desechapper(&echapper("a\tb\n100 %")), "a\tb\n100 %");
        assert_eq!(decage(&cage("a;b|c")), "a;b|c");
    }

    #[test]
    fn toutes_les_tables_ont_une_liste_de_colonnes() {
        for (table, colonnes) in orm::tables() {
            assert!(!colonnes.is_empty(), "table {table} sans colonne");
            assert_eq!(colonnes[0], "id", "table {table} : première colonne attendue id");
        }
    }

    #[test]
    fn aller_retour_table() {
        let dossier = dossier_test("table");
        let ligne = Ecrit::pour("clients")
            .unwrap()
            .n("id", 7)
            .n("societe_id", 1)
            .t("nom", "Atelier Dupont")
            .t("type_client", "professionnel")
            .t("email", "a@b.fr")
            .t("telephone", "")
            .t("adresse", "1 rue des\tTabulations")
            .t("code_postal", "06000")
            .t("ville", "Nice")
            .t("pays", "FR")
            .t("siren", "")
            .t("tva_intracom", "")
            .t("notes", "ligne 1\nligne 2")
            .t("etiquettes", "maintenance,urgent")
            .t("reference_externe", "")
            .t("rgpd_efface_le", "")
            .t("cree_le", "2026-10-30")
            .t("modifie_le", "2026-10-30")
            .fin()
            .unwrap();
        ecrire_table(&dossier, "clients", &[ligne]).unwrap();
        let lignes = lire_table(&dossier, "clients").unwrap();
        assert_eq!(lignes.len(), 1);
        let client = decoder_client(&lignes[0]).unwrap();
        assert_eq!(client.nom, "Atelier Dupont");
        assert_eq!(client.notes, "ligne 1\nligne 2");
        assert_eq!(client.etiquettes, vec!["maintenance", "urgent"]);
        assert_eq!(client.type_client, TypeClient::Professionnel);
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn colonnes_manquantes_signalees() {
        let r = Ecrit::pour("clients").unwrap().n("id", 1).fin();
        assert!(r.is_err());
        let r2 = Ecrit::pour("clients").unwrap().t("colonne_inventee", "x").fin();
        assert!(r2.is_err());
    }

    #[test]
    fn retour_lignes_encodees() {
        let lignes = vec![
            Ligne::new("Maintenance; forfait", 2.0, 50_000, 2000).avec_remise(500),
            Ligne::new("Port", 1.0, 1_000, 2000),
        ];
        let texte = encoder_lignes(&lignes);
        let relues = decoder_lignes(&texte);
        assert_eq!(relues.len(), 2);
        assert_eq!(relues[0].designation, "Maintenance; forfait");
        assert_eq!(relues[0].remise_bp, 500);
        assert_eq!(relues[1].prix_unitaire_ht, 1_000);
    }

    #[test]
    fn lecture_tolerante_aux_colonnes_nouvelles() {
        let dossier = dossier_test("tolerant");
        std::fs::create_dir_all(dossier_tables(&dossier)).unwrap();
        // Fichier ancien : uniquement trois colonnes.
        std::fs::write(
            chemin_table(&dossier, "clients"),
            "id\tnom\tville\n3\tMartin\tLyon\n",
        )
        .unwrap();
        let lignes = lire_table(&dossier, "clients").unwrap();
        let client = decoder_client(&lignes[0]).unwrap();
        assert_eq!(client.id, 3);
        assert_eq!(client.nom, "Martin");
        assert_eq!(client.ville, "Lyon");
        assert_eq!(client.email, "");
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn metadonnees() {
        let dossier = dossier_test("meta");
        std::fs::create_dir_all(&dossier).unwrap();
        let mut meta = HashMap::new();
        meta.insert("societe_courante".to_string(), "2".to_string());
        ecrire_meta(&dossier, &meta).unwrap();
        let relu = lire_meta(&dossier).unwrap();
        assert_eq!(relu.get("societe_courante"), Some(&"2".to_string()));
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
