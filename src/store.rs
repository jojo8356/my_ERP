//! Magasin de données de l'ERP : toute la logique d'accès aux objets métier.
//!
//! Un seul type `Store`, deux modes :
//! - `Store::memoire()` : tout en RAM (tests, `--memoire`) ;
//! - `Store::ouvrir(dossier)` : persisté dans `donnees/tables/*.tsv`
//!   (colonnes issues du schéma Rustorm, ADR-006).
//!
//! Aucun SQL, aucune connexion : la couche est volontairement simple et
//! synchrone (CLI mono-utilisateur, ENF-SIMPL-01). Le passage à PostgreSQL
//! consiste à réutiliser `orm::ddl_postgres()` et à remplacer les
//! `fichier::lire_table`/`ecrire_table` par des requêtes — le reste du code
//! (métier, CLI, TUI) ne bouge pas.

use crate::compta;
use crate::domain::*;
use crate::fichier;
use crate::orm;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use crate::securite::Role;

pub type Resultat<T> = Result<T, String>;

// ------------------------------------------------------------ Enregistrements

#[derive(Debug, Clone, PartialEq)]
pub struct Paiement {
    pub id: i64,
    pub document_id: i64,
    pub date_paiement: String,
    pub montant: Cents,
    pub moyen: String,
    pub reference: String,
    pub rapproche: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ecriture {
    pub id: i64,
    pub societe_id: i64,
    /// VE (ventes) · BQ (banque) · AC (achats) · AN (à-nouveaux/OD).
    pub journal: String,
    pub ecriture_num: i64,
    pub date_ecriture: String,
    pub compte: String,
    pub libelle: String,
    pub debit: Cents,
    pub credit: Cents,
    pub piece_ref: String,
    pub piece_date: String,
    pub comp_aux_num: String,
    pub comp_aux_lib: String,
    pub devise: String,
    pub lettrage: String,
    pub document_id: i64,
    pub paiement_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Utilisateur {
    pub id: i64,
    pub nom_utilisateur: String,
    pub nom_complet: String,
    pub email: String,
    pub role: Role,
    pub empreinte: String,
    pub secret_totp: String,
    pub codes_recuperation: Vec<String>,
    /// Empreinte du code de connexion envoyé par e-mail (SEC-03) ; vide = aucun.
    pub code_connexion: String,
    /// Expiration du code en cours (secondes epoch) ; 0 = aucun code.
    pub code_connexion_expire: i64,
    /// Tentatives erronées sur le code en cours (remis à 0 à chaque envoi).
    pub code_connexion_essais: i64,
    pub actif: bool,
    pub cree_le: String,
}

impl Utilisateur {
    pub fn nouveau(nom: &str, role: Role) -> Self {
        Self {
            id: 0,
            nom_utilisateur: nom.to_string(),
            nom_complet: String::new(),
            email: String::new(),
            role,
            empreinte: String::new(),
            secret_totp: String::new(),
            codes_recuperation: Vec::new(),
            code_connexion: String::new(),
            code_connexion_expire: 0,
            code_connexion_essais: 0,
            actif: true,
            cree_le: aujourdhui(),
        }
    }
    /// SEC-02 : la double authentification est obligatoire pour les admins.
    pub fn deux_facteurs_actif(&self) -> bool {
        !self.secret_totp.trim().is_empty()
    }

    /// Durée de vie du code de connexion envoyé par e-mail (SEC-03), en secondes.
    pub const CODE_CONNEXION_VALIDITE: i64 = 900;

    /// Nombre d'essais erronés avant que le code ne soit invalidé.
    pub const CODE_CONNEXION_ESSAIS_MAX: i64 = 5;

    /// Un code de connexion est-il encore utilisable à cet instant ?
    pub fn code_connexion_en_attente(&self, maintenant: i64) -> bool {
        !self.code_connexion.is_empty() && self.code_connexion_expire > maintenant
    }
}

/// Note datée sur une fiche (prospect ou client) : c'est le journal de la
/// relation commerciale (« relancé le 12 », « demande un devis en janvier »).
///
/// Distincte du champ libre `Client.notes`, qui reste un résumé : une note est
/// **horodatée, signée et conservée** avec la fiche (RGPD-02 : le dernier
/// contact peut être une note).
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: i64,
    pub societe_id: i64,
    pub client_id: i64,
    pub date_note: String,
    pub texte: String,
    pub auteur: String,
    pub cree_le: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub id: i64,
    pub societe_id: i64,
    pub type_doc: String,
    pub annee: i64,
    pub valeur: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sauvegarde {
    pub id: i64,
    pub societe_id: i64,
    pub date_sauvegarde: String,
    pub chemin: String,
    pub empreinte: String,
    pub taille_octets: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationBancaire {
    pub id: i64,
    pub societe_id: i64,
    pub date_operation: String,
    pub libelle: String,
    pub montant: Cents,
    pub reference: String,
    /// Paiement lettré (0 = non rapproché).
    pub paiement_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntreeAudit {
    pub id: i64,
    pub societe_id: i64,
    pub horodatage: String,
    pub utilisateur: String,
    pub action: String,
    pub detail: String,
    pub empreinte_precedente: String,
    pub empreinte: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recurrence {
    pub id: i64,
    pub societe_id: i64,
    pub client_id: i64,
    pub type_doc: TypeDocument,
    pub activite: String,
    /// mensuelle | trimestrielle | annuelle.
    pub periodicite: String,
    pub prochaine_le: String,
    pub lignes: Vec<Ligne>,
    pub actif: bool,
    pub libelle: String,
}

/// Paramètres de création d'un document (évite les fonctions à huit arguments).
#[derive(Debug, Clone)]
pub struct NouveauDocument {
    pub type_doc: TypeDocument,
    pub client_id: i64,
    pub date_doc: String,
    pub date_prestation: String,
    pub echeance: String,
    pub activite: String,
    pub devise: String,
    pub taux_change_micro: i64,
    pub lignes: Vec<Ligne>,
    pub motif: String,
}

impl NouveauDocument {
    pub fn nouveau(type_doc: TypeDocument, client_id: i64, date_doc: &str) -> Self {
        Self {
            type_doc,
            client_id,
            date_doc: date_doc.to_string(),
            date_prestation: date_doc.to_string(),
            echeance: String::new(),
            activite: "services".to_string(),
            devise: DEVISE_REFERENCE.to_string(),
            taux_change_micro: 1_000_000,
            lignes: Vec::new(),
            motif: String::new(),
        }
    }
    pub fn avec_lignes(mut self, lignes: Vec<Ligne>) -> Self {
        self.lignes = lignes;
        self
    }

    pub fn avec_activite(mut self, activite: &str) -> Self {
        self.activite = activite.to_string();
        self
    }
    pub fn avec_devise(mut self, devise: &str, taux_change_micro: i64) -> Self {
        self.devise = devise.to_string();
        self.taux_change_micro = taux_change_micro;
        self
    }
}

/// Historique unifié d'un client (EF-CRM-02).
#[derive(Debug, Clone, Default)]
pub struct HistoriqueClient {
    pub documents: Vec<Document>,
    pub paiements: Vec<Paiement>,
    pub ventes: Vec<Vente>,
    pub total_facture: Cents,
    pub total_paye: Cents,
    pub total_du: Cents,
}

/// Résultats de la recherche globale `/` (EF-SOC-03).
#[derive(Debug, Clone, Default)]
pub struct ResultatsRecherche {
    pub clients: Vec<Client>,
    pub documents: Vec<Document>,
    pub articles: Vec<Article>,
    pub ventes: Vec<Vente>,
}

impl ResultatsRecherche {
    pub fn total(&self) -> usize {
        self.clients.len() + self.documents.len() + self.articles.len() + self.ventes.len()
    }
}

/// Statistiques Vinted : marges par marque/taille, rotation, DAC7 (EF-VIN-04).
#[derive(Debug, Clone, Default)]
pub struct StatsVinted {
    pub nb_ventes: u32,
    pub ca: Cents,
    pub marge: Cents,
    pub par_marque: Vec<(String, u32, Cents)>,
    pub par_taille: Vec<(String, u32, Cents)>,
    pub articles_en_stock: usize,
    pub articles_vendus: usize,
    pub valeur_stock: Cents,
    pub ca_personnel: Cents,
    /// Part du stock écoulé : articles vendus / (en stock + vendus), en %.
    pub taux_ecoulement: i64,
    /// Délai moyen entre l'achat et la vente, en jours (0 si inconnu).
    pub delai_moyen_jours: i64,
}

// ------------------------------------------------------------ Le magasin

pub struct Store {
    /// Mode fichier : dossier des tables TSV.
    disque: Option<PathBuf>,
    /// Mode base : connexion PostgreSQL pilotée par Rustorm (feature `postgres`).
    #[cfg(feature = "postgres")]
    bdd: Option<crate::bdd::Bdd>,
    societe_courante: i64,
    session: String,
    prochain_id: i64,
    societes: Vec<Societe>,
    utilisateurs: Vec<Utilisateur>,
    clients: Vec<Client>,
    notes: Vec<Note>,
    documents: Vec<Document>,
    paiements: Vec<Paiement>,
    ecritures: Vec<Ecriture>,
    articles: Vec<Article>,
    ventes: Vec<Vente>,
    taux: Vec<TauxChange>,
    operations: Vec<OperationBancaire>,
    audit: Vec<EntreeAudit>,
    sequences: Vec<Sequence>,
    sauvegardes: Vec<Sauvegarde>,
    recurrences: Vec<Recurrence>,
}

impl Default for Store {
    fn default() -> Self {
        Self::memoire()
    }
}

impl Store {
    // ---------------------------------------------------------- Ouverture

    /// Magasin volatil (tests, démonstration `--memoire`).
    pub fn memoire() -> Self {
        Self {
            disque: None,
            #[cfg(feature = "postgres")]
            bdd: None,
            societe_courante: 0,
            session: "local".to_string(),
            prochain_id: 1,
            societes: Vec::new(),
            utilisateurs: Vec::new(),
            clients: Vec::new(),
            notes: Vec::new(),
            documents: Vec::new(),
            paiements: Vec::new(),
            ecritures: Vec::new(),
            articles: Vec::new(),
            ventes: Vec::new(),
            taux: Vec::new(),
            operations: Vec::new(),
            audit: Vec::new(),
            sequences: Vec::new(),
            sauvegardes: Vec::new(),
            recurrences: Vec::new(),
        }
    }

    /// Ouvre (ou crée) un dossier de données persistant.
    pub fn ouvrir(dossier: &Path) -> Resultat<Self> {
        std::fs::create_dir_all(dossier)
            .map_err(|e| format!("création du dossier de données {} : {e}", dossier.display()))?;
        let mut store = Self::memoire();
        store.disque = Some(dossier.to_path_buf());
        store.charger()?;
        // Reprise des données de la version 1 (`etat.tsv`) — ENF-PORT-01.
        let ancien = dossier.join("etat.tsv");
        if ancien.exists() && store.clients.is_empty() && store.documents.is_empty() {
            let importes = store.importer_etat_v1(&ancien)?;
            if importes > 0 {
                store.persister()?;
            }
        }
        Ok(store)
    }

    pub fn persistant(&self) -> bool {
        if self.disque.is_some() {
            return true;
        }
        #[cfg(feature = "postgres")]
        if self.bdd.is_some() {
            return true;
        }
        false
    }

    /// Vrai quand l'état vit dans PostgreSQL (mode base).
    pub fn en_base(&self) -> bool {
        #[cfg(feature = "postgres")]
        {
            self.bdd.is_some()
        }
        #[cfg(not(feature = "postgres"))]
        {
            false
        }
    }

    /// Description du stockage, pour `/admin sante`.
    /// Instantané complet de l'état, dans le format exact des supports :
    /// `(table, lignes encodées)` + métadonnées. C'est ce que consomment
    /// `/admin export` (CSV/TSV) et l'envoi vers PostgreSQL — une seule source
    /// d'encodage, donc aucune divergence possible entre les trois sorties.
    pub fn instantane(
        &self,
    ) -> Resultat<(
        Vec<(&'static str, Vec<Vec<String>>)>,
        std::collections::HashMap<String, String>,
    )> {
        Ok((self.tables_encodees()?, self.meta_encodee()))
    }

    pub fn support(&self) -> String {
        #[cfg(feature = "postgres")]
        if let Some(bdd) = &self.bdd {
            return format!("PostgreSQL via Rustorm ({})", bdd.description());
        }
        match &self.disque {
            Some(dossier) => format!("fichiers locaux ({})", dossier.display()),
            None => "mémoire (rien n'est conservé)".to_string(),
        }
    }

    /// Ouvre l'ERP sur une base PostgreSQL (feature `postgres`).
    ///
    /// La migration issue du schéma `.rsm` est appliquée si la base est vide :
    /// les tables sont donc créées par l'ORM, jamais à la main. L'état est
    /// ensuite chargé en mémoire ; chaque écriture est rejouée dans une
    /// transaction (`Store::persister`).
    #[cfg(feature = "postgres")]
    pub fn ouvrir_bdd(url: &str) -> Resultat<Self> {
        let mut bdd = crate::bdd::Bdd::connecter(url)?;
        if bdd.migrer()? {
            // Première utilisation : rien à charger, la base vient d'être créée.
        }
        let mut store = Self::memoire();
        store.bdd = Some(bdd);
        store.charger_bdd()?;
        Ok(store)
    }

    pub fn dossier(&self) -> Option<&Path> {
        self.disque.as_deref()
    }

    /// URL de la base courante, **mot de passe masqué** (journaux, `/admin
    /// chemin`). `None` en mode fichiers.
    #[cfg(feature = "postgres")]
    pub fn base_masquee(&self) -> Option<String> {
        self.bdd.as_ref().map(|b| b.description().to_string())
    }
    #[cfg(not(feature = "postgres"))]
    pub fn base_masquee(&self) -> Option<String> {
        None
    }

    /// Charge toutes les tables depuis PostgreSQL (mêmes décodeurs que le mode
    /// fichier : `fichier::decoder_*`, alimentés par `bdd::Bdd::charger`).
    #[cfg(feature = "postgres")]
    fn charger_bdd(&mut self) -> Resultat<()> {
        // La connexion est sortie du champ le temps du chargement : les
        // décodeurs écrivent dans `self` et ne peuvent pas emprunter `self.bdd`
        // en même temps (le compilateur l'interdit, et il a raison).
        let mut bdd = match self.bdd.take() {
            Some(b) => b,
            None => return Ok(()),
        };
        let resultat = self.decoder_bdd(&mut bdd);
        self.bdd = Some(bdd);
        resultat
    }

    #[cfg(feature = "postgres")]
    fn decoder_bdd(&mut self, bdd: &mut crate::bdd::Bdd) -> Resultat<()> {
        let meta = bdd.lire_meta()?;
        self.appliquer_meta(&meta);
        let mut decoder = |table: &'static str| -> Resultat<Vec<fichier::LigneTable>> {
            bdd.charger(table)?
                .into_iter()
                .map(|valeurs| fichier::LigneTable::depuis_valeurs(table, valeurs))
                .collect()
        };
        for l in decoder("societes")? {
            self.societes.push(fichier::decoder_societe(&l)?);
        }
        for l in decoder("utilisateurs")? {
            self.utilisateurs.push(fichier::decoder_utilisateur(&l)?);
        }
        for l in decoder("clients")? {
            self.clients.push(fichier::decoder_client(&l)?);
        }
        for l in decoder("notes")? {
            self.notes.push(fichier::decoder_note(&l)?);
        }
        for l in decoder("documents")? {
            self.documents.push(fichier::decoder_document(&l)?);
        }
        for l in decoder("lignes")? {
            let ligne = fichier::decoder_ligne(&l)?;
            if let Some(doc) = self.documents.iter_mut().find(|d| d.id == ligne.document_id) {
                doc.lignes.push(ligne);
            }
        }
        for doc in self.documents.iter_mut() {
            doc.lignes.sort_by_key(|l| l.position);
        }
        for l in decoder("paiements")? {
            self.paiements.push(fichier::decoder_paiement(&l)?);
        }
        for l in decoder("ecritures")? {
            self.ecritures.push(fichier::decoder_ecriture(&l)?);
        }
        for l in decoder("articles")? {
            self.articles.push(fichier::decoder_article(&l)?);
        }
        for l in decoder("ventes")? {
            self.ventes.push(fichier::decoder_vente(&l)?);
        }
        for l in decoder("taux_change")? {
            self.taux.push(fichier::decoder_taux(&l)?);
        }
        for l in decoder("operations_bancaires")? {
            self.operations.push(fichier::decoder_operation(&l)?);
        }
        for l in decoder("audit_log")? {
            self.audit.push(fichier::decoder_audit(&l)?);
        }
        for l in decoder("sequences")? {
            self.sequences.push(fichier::decoder_sequence(&l)?);
        }
        for l in decoder("sauvegardes")? {
            self.sauvegardes.push(fichier::decoder_sauvegarde(&l)?);
        }
        for l in decoder("recurrences")? {
            self.recurrences.push(fichier::decoder_recurrence(&l)?);
        }
        Ok(())
    }

    /// Applique les métadonnées du magasin (mêmes clés en fichier et en base).
    fn appliquer_meta(&mut self, meta: &HashMap<String, String>) {
        self.societe_courante = meta
            .get("societe_courante")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.session = meta
            .get("session")
            .cloned()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "local".to_string());
        self.prochain_id = meta
            .get("prochain_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .max(1);
    }

    fn charger(&mut self) -> Resultat<()> {
        let dossier = match self.disque.clone() {
            Some(d) => d,
            None => return Ok(()),
        };
        let meta = fichier::lire_meta(&dossier)?;
        self.appliquer_meta(&meta);

        let decoder = |table: &str| fichier::lire_table(&dossier, table);

        for l in decoder("societes")? {
            self.societes.push(fichier::decoder_societe(&l)?);
        }
        for l in decoder("utilisateurs")? {
            self.utilisateurs.push(fichier::decoder_utilisateur(&l)?);
        }
        for l in decoder("clients")? {
            self.clients.push(fichier::decoder_client(&l)?);
        }
        for l in decoder("notes")? {
            self.notes.push(fichier::decoder_note(&l)?);
        }
        for l in decoder("documents")? {
            self.documents.push(fichier::decoder_document(&l)?);
        }
        for l in decoder("lignes")? {
            let ligne = fichier::decoder_ligne(&l)?;
            if let Some(doc) = self.documents.iter_mut().find(|d| d.id == ligne.document_id) {
                doc.lignes.push(ligne);
            }
        }
        for doc in self.documents.iter_mut() {
            doc.lignes.sort_by_key(|l| l.position);
        }
        for l in decoder("paiements")? {
            self.paiements.push(fichier::decoder_paiement(&l)?);
        }
        for l in decoder("ecritures")? {
            self.ecritures.push(fichier::decoder_ecriture(&l)?);
        }
        for l in decoder("articles")? {
            self.articles.push(fichier::decoder_article(&l)?);
        }
        for l in decoder("ventes")? {
            self.ventes.push(fichier::decoder_vente(&l)?);
        }
        for l in decoder("taux_change")? {
            self.taux.push(fichier::decoder_taux(&l)?);
        }
        for l in decoder("operations_bancaires")? {
            self.operations.push(fichier::decoder_operation(&l)?);
        }
        for l in decoder("audit_log")? {
            self.audit.push(fichier::decoder_audit(&l)?);
        }
        for l in decoder("sequences")? {
            self.sequences.push(fichier::decoder_sequence(&l)?);
        }
        for l in decoder("sauvegardes")? {
            self.sauvegardes.push(fichier::decoder_sauvegarde(&l)?);
        }
        for l in decoder("recurrences")? {
            self.recurrences.push(fichier::decoder_recurrence(&l)?);
        }
        Ok(())
    }

    /// Écrit toutes les tables (une par fichier, écriture atomique).
    /// Encode toutes les tables (lignes prêtes pour le fichier **ou** la base).
    ///
    /// Un seul encodage sert aux deux supports : un champ ajouté au modèle ne
    /// peut donc pas être persisté dans un mode et oublié dans l'autre.
    fn tables_encodees(&self) -> Resultat<Vec<(&'static str, Vec<Vec<String>>)>> {
        let mut tables: Vec<(&'static str, Vec<Vec<String>>)> = Vec::new();

        let mut lignes = Vec::new();
        for s in &self.societes {
            lignes.push(fichier::encoder_societe(s)?);
        }
        tables.push(("societes", lignes));

        let mut lignes = Vec::new();
        for u in &self.utilisateurs {
            lignes.push(fichier::encoder_utilisateur(u)?);
        }
        tables.push(("utilisateurs", lignes));

        let mut lignes = Vec::new();
        for c in &self.clients {
            lignes.push(fichier::encoder_client(c)?);
        }
        tables.push(("clients", lignes));

        let mut lignes = Vec::new();
        for n in &self.notes {
            lignes.push(fichier::encoder_note(n)?);
        }
        tables.push(("notes", lignes));

        let mut lignes_doc = Vec::new();
        let mut lignes_lignes = Vec::new();
        for d in &self.documents {
            lignes_doc.push(fichier::encoder_document(d)?);
            for l in &d.lignes {
                lignes_lignes.push(fichier::encoder_ligne(l)?);
            }
        }
        tables.push(("documents", lignes_doc));
        tables.push(("lignes", lignes_lignes));

        let mut lignes = Vec::new();
        for p in &self.paiements {
            lignes.push(fichier::encoder_paiement(p)?);
        }
        tables.push(("paiements", lignes));

        let mut lignes = Vec::new();
        for e in &self.ecritures {
            lignes.push(fichier::encoder_ecriture(e)?);
        }
        tables.push(("ecritures", lignes));

        let mut lignes = Vec::new();
        for a in &self.articles {
            lignes.push(fichier::encoder_article(a)?);
        }
        tables.push(("articles", lignes));

        let mut lignes = Vec::new();
        for v in &self.ventes {
            lignes.push(fichier::encoder_vente(v)?);
        }
        tables.push(("ventes", lignes));

        let mut lignes = Vec::new();
        for t in &self.taux {
            lignes.push(fichier::encoder_taux(t)?);
        }
        tables.push(("taux_change", lignes));

        let mut lignes = Vec::new();
        for o in &self.operations {
            lignes.push(fichier::encoder_operation(o)?);
        }
        tables.push(("operations_bancaires", lignes));

        let mut lignes = Vec::new();
        for e in &self.audit {
            lignes.push(fichier::encoder_audit(e)?);
        }
        tables.push(("audit_log", lignes));

        let mut lignes = Vec::new();
        for sq in &self.sequences {
            lignes.push(fichier::encoder_sequence(sq)?);
        }
        tables.push(("sequences", lignes));

        let mut lignes = Vec::new();
        for sv in &self.sauvegardes {
            lignes.push(fichier::encoder_sauvegarde(sv)?);
        }
        tables.push(("sauvegardes", lignes));

        let mut lignes = Vec::new();
        for r in &self.recurrences {
            lignes.push(fichier::encoder_recurrence(r)?);
        }
        tables.push(("recurrences", lignes));

        Ok(tables)
    }

    /// Métadonnées persistées à côté des tables.
    fn meta_encodee(&self) -> HashMap<String, String> {
        let mut meta = HashMap::new();
        meta.insert(
            "societe_courante".to_string(),
            self.societe_courante.to_string(),
        );
        meta.insert("session".to_string(), self.session.clone());
        meta.insert("prochain_id".to_string(), self.prochain_id.to_string());
        meta.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
        meta.insert("schema".to_string(), orm::EMPREINTE_SCHEMA.to_string());
        meta
    }

    /// Enregistre l'état sur le support courant (dossier de fichiers ou base).
    pub fn persister(&mut self) -> Resultat<()> {
        #[cfg(feature = "postgres")]
        if self.bdd.is_some() {
            let tables = self.tables_encodees()?;
            let meta = self.meta_encodee();
            let bdd = self.bdd.as_mut().expect("bdd présente");
            bdd.enregistrer_tout(&tables, &meta)?;
            return Ok(());
        }

        let dossier = match self.disque.clone() {
            Some(d) => d,
            None => return Ok(()),
        };
        for (table, lignes) in self.tables_encodees()? {
            fichier::ecrire_table(&dossier, table, &lignes)?;
        }
        fichier::ecrire_meta(&dossier, &self.meta_encodee())?;
        Ok(())
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.prochain_id;
        self.prochain_id += 1;
        id
    }

    /// Nombre de lignes par table du schéma, dans l'ordre du modèle Rustorm.
    ///
    /// Sert à `/admin sante` : on voit d'un coup d'œil quelles tables existent
    /// réellement et lesquelles sont vides, sans ouvrir un seul fichier.
    pub fn compteurs(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("societes", self.societes.len()),
            ("utilisateurs", self.utilisateurs.len()),
            ("clients", self.clients.len()),
            ("notes", self.notes.len()),
            ("documents", self.documents.len()),
            (
                "lignes",
                self.documents.iter().map(|d| d.lignes.len()).sum::<usize>(),
            ),
            ("paiements", self.paiements.len()),
            ("ecritures", self.ecritures.len()),
            ("articles", self.articles.len()),
            ("ventes", self.ventes.len()),
            ("taux_change", self.taux.len()),
            ("operations_bancaires", self.operations.len()),
            ("audit_log", self.audit.len()),
            ("sequences", self.sequences.len()),
            ("sauvegardes", self.sauvegardes.len()),
            ("recurrences", self.recurrences.len()),
        ]
    }

    /// Nombre de lignes par table **telles qu'elles sont dans PostgreSQL**.
    ///
    /// `None` quand le magasin n'est pas connecté à une base. Sert à
    /// `/admin bdd` : on vérifie ainsi que l'ORM a bien créé et rempli le
    /// schéma, sans ouvrir un client SQL à côté.
    #[cfg(feature = "postgres")]
    pub fn compteurs_bdd(&mut self) -> Resultat<Option<Vec<(&'static str, i64)>>> {
        match self.bdd.as_mut() {
            Some(bdd) => Ok(Some(bdd.tables_presentes()?)),
            None => Ok(None),
        }
    }

    /// Vide toutes les tables de la base (le schéma reste en place) et remet le
    /// magasin en mémoire dans le même état — utile pour rejouer un scénario.
    #[cfg(feature = "postgres")]
    pub fn vider_bdd(&mut self) -> Resultat<()> {
        {
            let bdd = self
                .bdd
                .as_mut()
                .ok_or_else(|| "aucune base connectée".to_string())?;
            bdd.vider()?;
        }
        self.societes.clear();
        self.utilisateurs.clear();
        self.clients.clear();
        self.notes.clear();
        self.documents.clear();
        self.paiements.clear();
        self.ecritures.clear();
        self.articles.clear();
        self.ventes.clear();
        self.taux.clear();
        self.operations.clear();
        self.audit.clear();
        self.sequences.clear();
        self.sauvegardes.clear();
        self.recurrences.clear();
        self.societe_courante = 0;
        self.prochain_id = 1;
        Ok(())
    }

    // ---------------------------------------------------------- Sociétés

    pub fn societes(&self) -> Vec<Societe> {
        self.societes.clone()
    }

    /// Régime de TVA de la société courante, en lecture seule.
    ///
    /// Le CLI doit calculer des totaux TTC pour *afficher* (listes, JSON, CSV) :
    /// il ne peut pas payer un `&mut self` (création implicite de la société)
    /// à chaque ligne affichée. La valeur par défaut est la franchise en base,
    /// c'est-à-dire le régime d'une micro-entreprise qui démarre.
    pub fn societe_regime_public(&self) -> RegimeTva {
        self.societes
            .iter()
            .find(|s| s.id == self.societe_courante)
            .or_else(|| self.societes.first())
            .map(|s| s.regime)
            .unwrap_or(RegimeTva::Franchise)
    }

    /// Société courante (créée à la première utilisation — mono-utilisateur v1).
    pub fn societe(&mut self) -> Resultat<Societe> {
        if self.societes.is_empty() {
            let mut societe = Societe::default();
            societe.id = self.alloc_id();
            societe.cree_le = aujourdhui();
            self.societe_courante = societe.id;
            self.societes.push(societe);
            self.persister()?;
        }
        let id = self.societe_courante;
        if let Some(s) = self.societes.iter().find(|s| s.id == id) {
            return Ok(s.clone());
        }
        if let Some(s) = self.societes.first() {
            let id = s.id;
            self.societe_courante = id;
            return Ok(s.clone());
        }
        Err("aucune société".into())
    }

    pub fn creer_societe(&mut self, mut societe: Societe) -> Resultat<Societe> {
        if societe.nom.trim().is_empty() {
            return Err("le nom de la société est obligatoire".into());
        }
        if societe.cree_le.is_empty() {
            societe.cree_le = aujourdhui();
        }
        societe.id = self.alloc_id();
        self.societe_courante = societe.id;
        self.societes.push(societe.clone());
        self.persister()?;
        Ok(societe)
    }

    pub fn modifier_societe(&mut self, societe: Societe) -> Resultat<Societe> {
        let idx = self
            .societes
            .iter()
            .position(|s| s.id == societe.id)
            .ok_or_else(|| "société introuvable".to_string())?;
        if societe.nom.trim().is_empty() {
            return Err("le nom de la société est obligatoire".into());
        }
        self.societes[idx] = societe.clone();
        self.persister()?;
        Ok(societe)
    }

    /// Bascule de société par recherche floue sur le nom (EF-SOC-01).
    pub fn basculer_societe(&mut self, recherche: &str) -> Resultat<Societe> {
        let trouvee = self
            .societes
            .iter()
            .find(|s| correspond(&s.nom, recherche))
            .cloned()
            .ok_or_else(|| format!("aucune société ne correspond à « {recherche} »"))?;
        self.societe_courante = trouvee.id;
        self.persister()?;
        Ok(trouvee)
    }

    pub fn set_regime_tva(&mut self, regime: RegimeTva) -> Resultat<()> {
        let mut societe = self.societe()?;
        societe.regime = regime;
        self.modifier_societe(societe)?;
        Ok(())
    }

    // ---------------------------------------------------------- Devises (EF-SOC-02)

    pub fn enregistrer_taux(
        &mut self,
        devise: &str,
        date_taux: &str,
        taux_micro: i64,
        source: &str,
    ) -> Resultat<TauxChange> {
        if !date_valide(date_taux) {
            return Err(format!("date de taux invalide : « {date_taux} » (AAAA-MM-JJ)"));
        }
        if taux_micro <= 0 {
            return Err("le taux de change doit être strictement positif".into());
        }
        let taux = TauxChange {
            id: self.alloc_id(),
            devise: devise.trim().to_uppercase(),
            date_taux: date_taux.to_string(),
            taux_micro,
            source: if source.trim().is_empty() {
                "manuel".to_string()
            } else {
                source.to_string()
            },
        };
        self.taux.retain(|t| !(t.devise == taux.devise && t.date_taux == taux.date_taux));
        self.taux.push(taux.clone());
        self.taux.sort_by(|a, b| a.devise.cmp(&b.devise).then(a.date_taux.cmp(&b.date_taux)));
        self.persister()?;
        Ok(taux)
    }

    /// Taux applicable à une date : le plus récent connu avant (ou à) cette date.
    pub fn taux(&self, devise: &str, date: &str) -> Option<TauxChange> {
        let devise = devise.trim().to_uppercase();
        if devise == DEVISE_REFERENCE {
            return Some(TauxChange {
                id: 0,
                devise,
                date_taux: date.to_string(),
                taux_micro: 1_000_000,
                source: "référence".to_string(),
            });
        }
        self.taux
            .iter()
            .filter(|t| t.devise == devise && t.date_taux.as_str() <= date)
            .max_by(|a, b| a.date_taux.cmp(&b.date_taux))
            .cloned()
    }

    pub fn taux_liste(&self) -> Vec<TauxChange> {
        self.taux.clone()
    }

    // ---------------------------------------------------------- Utilisateurs / session (SEC-01..04)

    pub fn utilisateurs(&self) -> Vec<Utilisateur> {
        self.utilisateurs.clone()
    }

    pub fn utilisateur(&self, nom: &str) -> Option<Utilisateur> {
        self.utilisateurs
            .iter()
            .find(|u| u.nom_utilisateur.eq_ignore_ascii_case(nom.trim()))
            .cloned()
    }

    pub fn creer_utilisateur(&mut self, mut utilisateur: Utilisateur) -> Resultat<Utilisateur> {
        if utilisateur.nom_utilisateur.trim().is_empty() {
            return Err("nom d'utilisateur obligatoire".into());
        }
        if self.utilisateur(&utilisateur.nom_utilisateur).is_some() {
            return Err(format!(
                "l'utilisateur « {} » existe déjà",
                utilisateur.nom_utilisateur
            ));
        }
        if self.utilisateurs.is_empty() && !utilisateur.role.est_admin() {
            return Err("le premier compte créé doit être administrateur".into());
        }
        utilisateur.id = self.alloc_id();
        if utilisateur.cree_le.is_empty() {
            utilisateur.cree_le = aujourdhui();
        }
        self.utilisateurs.push(utilisateur.clone());
        self.persister()?;
        Ok(utilisateur)
    }

    /// Renomme l'adresse e-mail d'un compte (nécessaire au code de connexion).
    pub fn maj_email_utilisateur(&mut self, nom: &str, email: &str) -> Resultat<Utilisateur> {
        let email = email.trim();
        if !email.is_empty() && !email.contains('@') {
            return Err(format!("adresse e-mail invalide : « {email} »"));
        }
        let mut utilisateur = self
            .utilisateur(nom)
            .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
        utilisateur.email = email.to_string();
        self.maj_utilisateur(utilisateur.clone())?;
        Ok(utilisateur)
    }

    /// Prépare un code de connexion à usage unique et renvoie le code **en
    /// clair** : il part dans l'e-mail, jamais dans la sortie du CLI (SEC-03).
    /// Seule son empreinte est conservée, avec une échéance de 15 minutes.
    pub fn ouvrir_code_connexion(&mut self, nom: &str) -> Resultat<String> {
        let mut utilisateur = self
            .utilisateur(nom)
            .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
        if utilisateur.email.trim().is_empty() {
            return Err(format!(
                "le compte « {nom} » n'a pas d'adresse e-mail — renseignez-la d'abord : \
                 /utilisateurs email {nom} --email vous@exemple.fr"
            ));
        }
        let code = crate::securite::nouveau_code_connexion();
        utilisateur.code_connexion = crate::securite::empreinte_code(&code);
        utilisateur.code_connexion_expire =
            crate::securite::maintenant_secondes() as i64 + Utilisateur::CODE_CONNEXION_VALIDITE;
        utilisateur.code_connexion_essais = 0;
        self.maj_utilisateur(utilisateur)?;
        Ok(code)
    }

    /// Vérifie puis **consomme** le code de connexion envoyé par e-mail.
    ///
    /// Le code est à usage unique : il est effacé dès qu'il est accepté, et
    /// invalidé après `CODE_CONNEXION_ESSAIS_MAX` tentatives erronées ou à
    /// l'expiration (SEC-03).
    pub fn verifier_code_connexion(&mut self, nom: &str, code: &str) -> Resultat<()> {
        let mut utilisateur = self
            .utilisateur(nom)
            .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
        let maintenant = crate::securite::maintenant_secondes() as i64;
        if utilisateur.code_connexion.is_empty() {
            return Err(
                "aucun code de connexion en attente — demandez-en un : /utilisateurs code <compte>"
                    .into(),
            );
        }
        if utilisateur.code_connexion_expire <= maintenant {
            utilisateur.code_connexion.clear();
            utilisateur.code_connexion_expire = 0;
            utilisateur.code_connexion_essais = 0;
            self.maj_utilisateur(utilisateur)?;
            return Err("code de connexion expiré — demandez-en un nouveau".into());
        }
        if utilisateur.code_connexion_essais >= Utilisateur::CODE_CONNEXION_ESSAIS_MAX {
            utilisateur.code_connexion.clear();
            utilisateur.code_connexion_expire = 0;
            utilisateur.code_connexion_essais = 0;
            self.maj_utilisateur(utilisateur)?;
            return Err(format!(
                "trop de tentatives : code invalidé — demandez-en un nouveau \
                 (/utilisateurs code {nom})"
            ));
        }
        let empreinte = crate::securite::empreinte_code(code.trim());
        if !crate::securite::egalite_constante(
            empreinte.as_bytes(),
            utilisateur.code_connexion.as_bytes(),
        ) {
            utilisateur.code_connexion_essais += 1;
            let restants =
                Utilisateur::CODE_CONNEXION_ESSAIS_MAX - utilisateur.code_connexion_essais;
            self.maj_utilisateur(utilisateur)?;
            return Err(format!(
                "code de connexion invalide ({restants} tentative(s) restante(s))"
            ));
        }
        // Code accepté : il ne resservira jamais.
        utilisateur.code_connexion.clear();
        utilisateur.code_connexion_expire = 0;
        utilisateur.code_connexion_essais = 0;
        self.maj_utilisateur(utilisateur)?;
        Ok(())
    }

    pub fn maj_utilisateur(&mut self, utilisateur: Utilisateur) -> Resultat<()> {
        let idx = self
            .utilisateurs
            .iter()
            .position(|u| u.id == utilisateur.id)
            .ok_or_else(|| "utilisateur introuvable".to_string())?;
        self.utilisateurs[idx] = utilisateur;
        self.persister()
    }

    /// Utilisateur courant (« local » tant que personne ne s'authentifie).
    pub fn session(&self) -> String {
        self.session.clone()
    }

    pub fn role_courant(&self) -> Role {
        self.utilisateur(&self.session)
            .map(|u| u.role)
            .unwrap_or(Role::Utilisateur)
    }

    pub fn etablir_session(&mut self, nom: &str) -> Resultat<()> {
        if self.utilisateur(nom).is_none() {
            return Err(format!("utilisateur inconnu : « {nom} »"));
        }
        self.session = self
            .utilisateur(nom)
            .map(|u| u.nom_utilisateur)
            .unwrap_or_else(|| "local".to_string());
        self.persister()
    }

    pub fn fermer_session(&mut self) -> Resultat<()> {
        self.session = "local".to_string();
        self.persister()
    }

    /// Contrôle de rôle : en mode mono-utilisateur (aucun compte créé), tout
    /// est permis ; dès qu'un compte existe, les actions sensibles exigent admin.
    pub fn exiger_admin(&self) -> Resultat<()> {
        if self.utilisateurs.is_empty() {
            return Ok(());
        }
        if self.role_courant().est_admin() {
            Ok(())
        } else {
            Err(format!(
                "action réservée aux administrateurs (session « {} ») — connectez-vous avec `/auth connexion <compte>`",
                self.session
            ))
        }
    }

    // ---------------------------------------------------------- Clients

    pub fn clients(&self) -> Vec<Client> {
        let id = self.societe_courante;
        self.clients
            .iter()
            .filter(|c| id == 0 || c.societe_id == id)
            .cloned()
            .collect()
    }

    /// Clients filtrés par statut (prospect ou client).
    pub fn clients_par_statut(&self, statut: StatutClient) -> Vec<Client> {
        self.clients()
            .into_iter()
            .filter(|c| c.statut == statut)
            .collect()
    }

    pub fn prospects(&self) -> Vec<Client> {
        self.clients_par_statut(StatutClient::Prospect)
    }

    pub fn clients_actifs(&self) -> Vec<Client> {
        self.clients_par_statut(StatutClient::Client)
    }

    /// Convertit un prospect en client (CDC Module A). Sans effet s'il l'est
    /// déjà : le message le dit plutôt que de faire semblant.
    pub fn convertir_client(&mut self, id: i64) -> Resultat<Client> {
        let mut client = self.client_par_id(id)?;
        if !client.est_prospect() {
            return Err(format!("{} est déjà un client", client.nom));
        }
        client.statut = StatutClient::Client;
        client.modifie_le = aujourdhui();
        let client = self.modifier_client(client)?;
        Ok(client)
    }

    /// Date du dernier contact connu avec une fiche : dernière pièce émise
    /// (devis, facture…), dernier paiement, sinon création/modification.
    ///
    /// C'est cette date qui borne la conservation RGPD-02 (« prospects :
    /// 3 ans après le dernier contact »).
    pub fn dernier_contact(&self, client_id: i64) -> String {
        let mut dates: Vec<&str> = Vec::new();
        if let Some(client) = self.clients.iter().find(|c| c.id == client_id) {
            dates.push(client.cree_le.as_str());
            dates.push(client.modifie_le.as_str());
        }
        // Une note (appel, relance) est un contact au sens RGPD-02.
        for note in self
            .notes
            .iter()
            .filter(|n| n.client_id == client_id && n.societe_id == self.societe_courante)
        {
            dates.push(note.date_note.as_str());
        }
        let documents: Vec<&Document> = self
            .documents
            .iter()
            .filter(|d| d.client_id == client_id && d.societe_id == self.societe_courante)
            .collect();
        for document in &documents {
            dates.push(document.date_doc.as_str());
            dates.push(document.envoye_le.as_str());
            dates.push(document.date_prestation.as_str());
        }
        for paiement in self
            .paiements
            .iter()
            .filter(|p| documents.iter().any(|d| d.id == p.document_id))
        {
            dates.push(paiement.date_paiement.as_str());
        }
        dates
            .into_iter()
            .filter(|d| crate::domain::date_valide(d))
            .max()
            .unwrap_or("")
            .to_string()
    }

    /// Prospects dont le dernier contact est antérieur à la durée de
    /// conservation (3 ans, RGPD-02) : à anonymiser ou à convertir.
    ///
    /// Renvoie des couples (prospect, date de dernier contact).
    pub fn prospects_perimes(&self, reference: &str) -> Vec<(Client, String)> {
        let limite = crate::domain::retirer_annees(
            reference,
            StatutClient::Prospect.conservation_annees(),
        );
        let mut sortie: Vec<(Client, String)> = self
            .prospects()
            .into_iter()
            .map(|client| {
                let contact = self.dernier_contact(client.id);
                (client, contact)
            })
            .filter(|(_, contact)| !contact.is_empty() && *contact < limite)
            .collect();
        sortie.sort_by(|a, b| a.1.cmp(&b.1));
        sortie
    }

    // ---------------------------------------------------------- Notes de fiche

    /// Ajoute une note datée à une fiche (prospect ou client) et renvoie la note
    /// créée. L'auteur est la session courante : le journal dit qui a écrit.
    pub fn noter_client(&mut self, client_id: i64, texte: &str) -> Resultat<Note> {
        let texte = texte.trim();
        if texte.is_empty() {
            return Err("la note est vide — précisez le texte à conserver".into());
        }
        let client = self.client_par_id(client_id)?;
        let societe_id = self.societe()?.id;
        let note = Note {
            id: self.alloc_id(),
            societe_id,
            client_id,
            date_note: aujourdhui(),
            texte: texte.to_string(),
            auteur: self.session.clone(),
            cree_le: horodatage(),
        };
        self.notes.push(note.clone());
        self.persister()?;
        self.journaliser("client.note", &format!("{} (n°{})", client.nom, client.id))?;
        Ok(note)
    }

    /// Notes d'une fiche, de la plus ancienne à la plus récente (lecture du
    /// journal), limitées à la société courante.
    pub fn notes_client(&self, client_id: i64) -> Vec<Note> {
        let mut notes: Vec<Note> = self
            .notes
            .iter()
            .filter(|n| n.client_id == client_id && n.societe_id == self.societe_courante)
            .cloned()
            .collect();
        notes.sort_by_key(|n| n.id);
        notes
    }

    /// Toutes les notes (export, `/admin sante`).
    pub fn notes(&self) -> Vec<Note> {
        self.notes
            .iter()
            .filter(|n| n.societe_id == self.societe_courante)
            .cloned()
            .collect()
    }

    /// Supprime une note (erreur de saisie). Les pièces comptables, elles, ne se
    /// suppriment jamais : une note n'en est pas une.
    pub fn supprimer_note(&mut self, id: i64) -> Resultat<Note> {
        let idx = self
            .notes
            .iter()
            .position(|n| n.id == id)
            .ok_or_else(|| format!("note n°{id} introuvable"))?;
        let note = self.notes.remove(idx);
        self.persister()?;
        self.journaliser("client.note-suppression", &format!("note n°{id}"))?;
        Ok(note)
    }

    pub fn creer_client(&mut self, mut client: Client) -> Resultat<Client> {
        if client.nom.trim().is_empty() {
            return Err("le nom du client est obligatoire".into());
        }
        if client.societe_id == 0 {
            client.societe_id = self.societe()?.id;
        }
        client.id = self.alloc_id();
        if client.cree_le.is_empty() {
            client.cree_le = aujourdhui();
        }
        client.modifie_le = client.cree_le.clone();
        if client.pays.is_empty() {
            client.pays = "FR".to_string();
        }
        self.clients.push(client.clone());
        self.persister()?;
        Ok(client)
    }

    pub fn chercher_clients(&self, recherche: &str) -> Vec<Client> {
        let recherche = recherche.trim();
        let mut trouves: Vec<Client> = self
            .clients()
            .into_iter()
            .filter(|c| {
                correspond(&c.nom, recherche)
                    || correspond(&c.email, recherche)
                    || correspond(&c.ville, recherche)
                    || correspond(&c.siren, recherche)
                    || c.etiquettes.iter().any(|e| correspond(e, recherche))
            })
            .collect();
        // Correspondance exacte d'abord, puis ordre alphabétique.
        trouves.sort_by(|a, b| {
            let exact_a = a.nom.eq_ignore_ascii_case(recherche);
            let exact_b = b.nom.eq_ignore_ascii_case(recherche);
            exact_b.cmp(&exact_a).then(a.nom.cmp(&b.nom))
        });
        trouves
    }

    /// Clients portant une étiquette (EF-CRM-03 : `clients --tag maintenance`).
    pub fn clients_par_etiquette(&self, etiquette: &str) -> Vec<Client> {
        self.clients()
            .into_iter()
            .filter(|c| c.etiquette_presente(etiquette))
            .collect()
    }

    pub fn client_par_id(&self, id: i64) -> Resultat<Client> {
        self.clients
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| format!("client n°{id} introuvable"))
    }

    pub fn modifier_client(&mut self, client: Client) -> Resultat<Client> {
        let idx = self
            .clients
            .iter()
            .position(|c| c.id == client.id)
            .ok_or_else(|| format!("client n°{} introuvable", client.id))?;
        if client.nom.trim().is_empty() {
            return Err("le nom du client est obligatoire".into());
        }
        let mut client = client;
        client.modifie_le = aujourdhui();
        self.clients[idx] = client.clone();
        self.persister()?;
        Ok(client)
    }

    pub fn etiqueter(&mut self, client_id: i64, etiquette: &str) -> Resultat<Client> {
        let mut client = self.client_par_id(client_id)?;
        let etiquette = etiquette.trim();
        if etiquette.is_empty() {
            return Err("étiquette vide".into());
        }
        if !client.etiquette_presente(etiquette) {
            client.etiquettes.push(etiquette.to_string());
        }
        self.modifier_client(client)
    }

    pub fn retirer_etiquette(&mut self, client_id: i64, etiquette: &str) -> Resultat<Client> {
        let mut client = self.client_par_id(client_id)?;
        client.etiquettes.retain(|e| !e.eq_ignore_ascii_case(etiquette.trim()));
        self.modifier_client(client)
    }

    /// RGPD-02 : droit à l'effacement, sans jamais détruire les pièces comptables
    /// (conservation 10 ans, Code de commerce).
    pub fn effacer_client_rgpd(&mut self, client_id: i64) -> Resultat<String> {
        let mut client = self.client_par_id(client_id)?;
        let documents = self
            .documents
            .iter()
            .filter(|d| d.client_id == client_id)
            .count();
        client.nom = format!("Client anonymisé n°{}", client_id);
        client.email.clear();
        client.telephone.clear();
        client.adresse.clear();
        client.code_postal.clear();
        client.ville.clear();
        client.siren.clear();
        client.tva_intracom.clear();
        client.notes.clear();
        client.etiquettes.clear();
        client.reference_externe.clear();
        client.rgpd_efface_le = aujourdhui();
        // Les notes sont du texte libre (donc des données personnelles) : elles
        // partent avec l'anonymisation, seules les pièces comptables restent.
        let notes_supprimees = self.notes.iter().filter(|n| n.client_id == client_id).count();
        self.notes.retain(|n| n.client_id != client_id);
        self.modifier_client(client)?;
        self.journaliser(
            "rgpd.effacement",
            &format!(
                "client n°{client_id} anonymisé ({documents} pièces conservées, \
                 {notes_supprimees} note(s) supprimée(s))"
            ),
        )?;
        Ok(format!(
            "client n°{client_id} anonymisé — {documents} pièce(s) comptable(s) conservée(s) 10 ans (obligation légale)"
        ))
    }

    /// Export RGPD des données d'un client (droit d'accès et de portabilité).
    pub fn exporter_client(&self, client_id: i64) -> Resultat<String> {
        let client = self.client_par_id(client_id)?;
        let mut texte = String::new();
        texte.push_str("nom;type;email;telephone;adresse;code_postal;ville;siren;tva_intracom;etiquettes;rgpd_efface_le\n");
        texte.push_str(&format!(
            "{};{};{};{};{};{};{};{};{};{};{}\n",
            client.nom,
            client.type_client.as_str(),
            client.email,
            client.telephone,
            client.adresse,
            client.code_postal,
            client.ville,
            client.siren,
            client.tva_intracom,
            etiquettes_vers_texte(&client.etiquettes),
            client.rgpd_efface_le
        ));
        let documents: Vec<Document> = self
            .documents
            .iter()
            .filter(|d| d.client_id == client_id)
            .cloned()
            .collect();
        let regime = self.societe_regime();
        texte.push_str("\ndocument;date;ttc;paye;statut\n");
        for d in documents {
            texte.push_str(&format!(
                "{};{};{},{:02};{},{:02};{}\n",
                d.numero,
                d.date_doc,
                d.totaux(regime).ttc / 100,
                d.totaux(regime).ttc % 100,
                d.paye / 100,
                d.paye % 100,
                d.statut.as_str()
            ));
        }
        Ok(texte)
    }

    fn societe_regime(&self) -> RegimeTva {
        let id = self.societe_courante;
        self.societes
            .iter()
            .find(|s| s.id == id)
            .or_else(|| self.societes.first())
            .map(|s| s.regime)
            .unwrap_or(RegimeTva::Franchise)
    }

    /// Historique unifié : devis, factures, avoirs, paiements, ventes (EF-CRM-02).
    pub fn historique_client(&self, client_id: i64) -> Resultat<HistoriqueClient> {
        let client = self.client_par_id(client_id)?;
        let regime = self.societe_regime();
        let documents: Vec<Document> = self
            .documents
            .iter()
            .filter(|d| d.client_id == client_id)
            .cloned()
            .collect();
        let mut paiements = Vec::new();
        let mut total_facture = 0;
        let mut total_paye = 0;
        for doc in &documents {
            if matches!(doc.type_doc, TypeDocument::Facture) {
                total_facture += doc.totaux(regime).ttc;
                total_paye += doc.paye;
            }
            for p in self.paiements.iter().filter(|p| p.document_id == doc.id) {
                paiements.push(p.clone());
            }
        }
        let ventes: Vec<Vente> = self
            .ventes
            .iter()
            .filter(|v| {
                !v.acheteur.trim().is_empty() && correspond(&v.acheteur, &client.nom)
            })
            .cloned()
            .collect();
        Ok(HistoriqueClient {
            documents,
            paiements,
            ventes,
            total_facture,
            total_paye,
            total_du: (total_facture - total_paye).max(0),
        })
    }

    // ---------------------------------------------------------- Documents

    pub fn documents(&self, type_doc: Option<TypeDocument>) -> Vec<Document> {
        let id = self.societe_courante;
        self.documents
            .iter()
            .filter(|d| id == 0 || d.societe_id == id)
            .filter(|d| type_doc.map(|t| d.type_doc == t).unwrap_or(true))
            .cloned()
            .collect()
    }

    pub fn document_par_numero(&self, numero: &str) -> Resultat<Document> {
        let recherche = numero.trim();
        let societe_id = self.societe_courante;
        self.documents
            .iter()
            .find(|d| d.societe_id == societe_id && d.numero.eq_ignore_ascii_case(recherche))
            .or_else(|| {
                // Forme courte « 2026/0042 » ou « 0042 » du CDC §6.1.
                let propre: String = recherche.chars().filter(|c| c.is_ascii_digit()).collect();
                self.documents.iter().find(|d| {
                    if d.societe_id != societe_id {
                        return false;
                    }
                    let chiffres: String = d.numero.chars().filter(|c| c.is_ascii_digit()).collect();
                    !propre.is_empty() && chiffres == propre
                })
            })
            .cloned()
            .ok_or_else(|| format!("document « {numero} » introuvable"))
    }

    /// Résout `/factures/2026/0042` (année + séquence).
    pub fn document_par_annee_sequence(
        &self,
        type_doc: TypeDocument,
        annee: &str,
        sequence: &str,
    ) -> Resultat<Document> {
        let seq: u32 = sequence
            .parse()
            .map_err(|_| format!("numéro de séquence illisible : « {sequence} »"))?;
        let annee: u32 = annee
            .parse()
            .map_err(|_| format!("année illisible : « {annee} »"))?;
        let numero = numero_document(type_doc, annee, seq);
        self.document_par_numero(&numero)
    }

    pub fn documents_impayes(&self) -> Vec<Document> {
        self.documents(Some(TypeDocument::Facture))
            .into_iter()
            .filter(|d| d.est_impaye())
            .collect()
    }

    pub fn documents_en_retard(&self, reference: &str) -> Vec<Document> {
        self.documents(Some(TypeDocument::Facture))
            .into_iter()
            .filter(|d| d.en_retard(reference))
            .collect()
    }

    pub fn creer_document(&mut self, nouveau: NouveauDocument) -> Resultat<Document> {
        let societe = self.societe()?;
        self.client_par_id(nouveau.client_id)?;
        if nouveau.lignes.is_empty() {
            return Err("un document sans ligne n'a pas de sens".into());
        }
        if !date_valide(&nouveau.date_doc) {
            return Err(format!(
                "date d'émission invalide : « {} » (AAAA-MM-JJ)",
                nouveau.date_doc
            ));
        }
        for ligne in &nouveau.lignes {
            if ligne.designation.trim().is_empty() {
                return Err("chaque ligne doit avoir une désignation".into());
            }
            if ligne.quantite_milli == 0 {
                return Err(format!("quantité nulle sur « {} »", ligne.designation));
            }
        }
        let annee = annee_de(&nouveau.date_doc);
        let sequence = self.prochaine_sequence(societe.id, nouveau.type_doc, annee);
        let numero = numero_document(nouveau.type_doc, annee, sequence);
        // La numérotation est propre à chaque société : le même numéro peut
        // exister dans deux sociétés, mais jamais deux fois dans la même.
        if self
            .documents
            .iter()
            .any(|d| d.societe_id == societe.id && d.numero.eq_ignore_ascii_case(&numero))
        {
            return Err(format!(
                "numéro {numero} déjà utilisé : la numérotation doit rester sans trou (EF-FAC-02)"
            ));
        }
        let delai = societe.delai_paiement_jours;
        let echeance = if nouveau.echeance.trim().is_empty() {
            ajouter_jours(&nouveau.date_doc, delai)
        } else {
            nouveau.echeance.clone()
        };
        if !date_valide(&echeance) {
            return Err(format!("échéance invalide : « {echeance} »"));
        }
        let id = self.alloc_id();
        let mut document = Document {
            id,
            societe_id: societe.id,
            numero,
            type_doc: nouveau.type_doc,
            statut: StatutDocument::Brouillon,
            client_id: nouveau.client_id,
            date_doc: nouveau.date_doc.clone(),
            date_prestation: if nouveau.date_prestation.trim().is_empty() {
                nouveau.date_doc.clone()
            } else {
                nouveau.date_prestation.clone()
            },
            echeance,
            activite: if nouveau.activite.trim().is_empty() {
                "services".to_string()
            } else {
                nouveau.activite.clone()
            },
            devise: if nouveau.devise.trim().is_empty() {
                societe.devise.clone()
            } else {
                nouveau.devise.clone()
            },
            taux_change_micro: if nouveau.taux_change_micro <= 0 {
                1_000_000
            } else {
                nouveau.taux_change_micro
            },
            conditions_paiement: societe.conditions_paiement(),
            delai_paiement_jours: delai,
            penalites_taux_bp: societe.penalites_taux_bp,
            indemnite_forfaitaire: societe.indemnite_forfaitaire,
            lignes: Vec::new(),
            paye: 0,
            envoye_le: String::new(),
            motif: nouveau.motif.clone(),
            cree_le: aujourdhui(),
        };
        for (position, ligne) in nouveau.lignes.into_iter().enumerate() {
            let mut ligne = ligne;
            ligne.id = self.alloc_id();
            ligne.document_id = id;
            ligne.position = position as i64;
            document.lignes.push(ligne);
        }
        self.documents.push(document.clone());
        self.persister()?;
        Ok(document)
    }

    fn prochaine_sequence(&mut self, societe_id: i64, type_doc: TypeDocument, annee: u32) -> u32 {
        let code = type_doc.as_str().to_string();
        if let Some(seq) = self.sequences.iter_mut().find(|s| {
            s.societe_id == societe_id && s.type_doc == code && s.annee == i64::from(annee)
        }) {
            seq.valeur += 1;
            return seq.valeur as u32;
        }
        let id = self.alloc_id();
        self.sequences.push(Sequence {
            id,
            societe_id,
            type_doc: code,
            annee: i64::from(annee),
            valeur: 1,
        });
        1
    }

    fn index_document(&self, numero: &str) -> Resultat<usize> {
        let societe_id = self.societe_courante;
        self.documents
            .iter()
            .position(|d| d.societe_id == societe_id && d.numero.eq_ignore_ascii_case(numero.trim()))
            .ok_or_else(|| format!("document « {numero} » introuvable"))
    }

    /// Transitions autorisées (EF-FAC-01/02) — aucune modification après validation.
    fn transition_autorisee(&self, actuel: StatutDocument, cible: StatutDocument) -> bool {
        use StatutDocument::*;
        match (actuel, cible) {
            (Brouillon, Envoye) | (Brouillon, Valide) | (Brouillon, Annule) => true,
            (Envoye, Accepte) | (Envoye, Refuse) | (Envoye, Expire) | (Envoye, Valide) => true,
            (Envoye, Annule) => true,
            (Valide, Annule) => true,
            (PartiellementPaye, Paye) => true,
            (PartiellementPaye, Annule) => true,
            (a, b) => a == b,
        }
    }

    /// Validation : contrôle bloquant des mentions obligatoires (CDC §11.1).
    pub fn valider_document(&mut self, numero: &str) -> Resultat<Document> {
        let idx = self.index_document(numero)?;
        let document = self.documents[idx].clone();
        if !peut_modifier(document.statut) {
            return Err(format!(
                "{} est déjà {} : un document validé ne se modifie plus (créez un avoir)",
                document.numero,
                document.statut.as_str()
            ));
        }
        let societe = self.societe()?;
        let client = self.client_par_id(document.client_id)?;
        // Une facture est une pièce comptable : on ne facture pas un prospect
        // (CDC Module A). Le devis, lui, est justement l'outil de prospection.
        if document.type_doc == TypeDocument::Facture && client.est_prospect() {
            return Err(format!(
                "{} est encore un prospect : convertissez la fiche en client avant de \
                 valider une facture — /clients {} convertir",
                client.nom, client.nom
            ));
        }
        let controle = controle_mentions(&societe, &client, &document);
        if !controle.conforme() {
            return Err(format!(
                "validation refusée — mentions obligatoires manquantes : {} \
                 (complétez avec `/societe modifier --siren …` ou `/clients <nom> modifier …`)",
                controle.manquants.join(", ")
            ));
        }
        self.documents[idx].statut = StatutDocument::Valide;
        let document = self.documents[idx].clone();
        self.persister()?;
        Ok(document)
    }

    pub fn changer_statut(&mut self, numero: &str, statut: StatutDocument) -> Resultat<Document> {
        if statut == StatutDocument::Valide {
            return self.valider_document(numero);
        }
        if statut == StatutDocument::Paye {
            return Err("le statut « payée » est calculé à partir des paiements : `/factures <n°> payer`".into());
        }
        let idx = self.index_document(numero)?;
        let actuel = self.documents[idx].statut;
        if !self.transition_autorisee(actuel, statut) {
            return Err(format!(
                "transition impossible : {} → {} ({} reste immuable)",
                actuel.as_str(),
                statut.as_str(),
                self.documents[idx].numero
            ));
        }
        if statut == StatutDocument::Expire && self.documents[idx].type_doc != TypeDocument::Devis {
            return Err("seul un devis peut expirer".into());
        }
        self.documents[idx].statut = statut;
        if statut == StatutDocument::Envoye && self.documents[idx].envoye_le.is_empty() {
            self.documents[idx].envoye_le = aujourdhui();
        }
        let document = self.documents[idx].clone();
        self.persister()?;
        Ok(document)
    }

    pub fn annuler_document(&mut self, numero: &str, motif: &str) -> Resultat<Document> {
        let idx = self.index_document(numero)?;
        if matches!(
            self.documents[idx].statut,
            StatutDocument::Paye | StatutDocument::Annule
        ) {
            return Err(format!(
                "{} est {} : annulation impossible (créez un avoir)",
                numero,
                self.documents[idx].statut.as_str()
            ));
        }
        self.documents[idx].statut = StatutDocument::Annule;
        self.documents[idx].motif = motif.to_string();
        let document = self.documents[idx].clone();
        self.persister()?;
        Ok(document)
    }

    /// Devis → facture en une commande (EF-FAC-01).
    pub fn transformer_devis(&mut self, numero: &str, date: &str) -> Resultat<Document> {
        let devis = self.document_par_numero(numero)?;
        if devis.type_doc != TypeDocument::Devis {
            return Err("seul un devis peut être transformé en facture".into());
        }
        if matches!(
            devis.statut,
            StatutDocument::Refuse | StatutDocument::Annule | StatutDocument::Expire
        ) {
            return Err(format!(
                "devis {} : aucune transformation possible",
                devis.statut.as_str()
            ));
        }
        // Une facture ne se crée pas pour un prospect : le devis reste une
        // proposition commerciale tant que la fiche n'est pas convertie.
        let client = self.client_par_id(devis.client_id)?;
        if client.est_prospect() {
            return Err(format!(
                "{} est encore un prospect : convertissez la fiche en client avant de \
                 transformer le devis — /clients {} convertir",
                client.nom, client.nom
            ));
        }
        self.changer_statut(numero, StatutDocument::Accepte)?;
        let facture = self.creer_document(
            NouveauDocument {
                type_doc: TypeDocument::Facture,
                client_id: devis.client_id,
                date_doc: date.to_string(),
                date_prestation: devis.date_prestation.clone(),
                echeance: String::new(),
                activite: devis.activite.clone(),
                devise: devis.devise.clone(),
                taux_change_micro: devis.taux_change_micro,
                lignes: devis.lignes.clone(),
                motif: format!("suite au devis {}", devis.numero),
            },
        )?;
        Ok(facture)
    }

    /// Avoir corrigeant une facture validée (EF-FAC-02).
    pub fn creer_avoir(&mut self, numero: &str, date: &str) -> Resultat<Document> {
        let facture = self.document_par_numero(numero)?;
        if facture.type_doc != TypeDocument::Facture {
            return Err("un avoir ne corrige qu'une facture".into());
        }
        if !matches!(
            facture.statut,
            StatutDocument::Valide | StatutDocument::PartiellementPaye | StatutDocument::Paye
        ) {
            return Err(format!(
                "{} est {} : un avoir ne corrige qu'une facture validée",
                facture.numero,
                facture.statut.as_str()
            ));
        }
        let lignes: Vec<Ligne> = facture
            .lignes
            .iter()
            .map(|l| Ligne {
                designation: format!("Avoir sur {} — {}", facture.numero, l.designation),
                prix_unitaire_ht: -l.prix_unitaire_ht,
                ..l.clone()
            })
            .collect();
        let avoir = self.creer_document(NouveauDocument {
            type_doc: TypeDocument::Avoir,
            client_id: facture.client_id,
            date_doc: date.to_string(),
            date_prestation: facture.date_prestation.clone(),
            echeance: date.to_string(),
            activite: facture.activite.clone(),
            devise: facture.devise.clone(),
            taux_change_micro: facture.taux_change_micro,
            lignes,
            motif: format!("avoir sur facture {}", facture.numero),
        })?;
        self.changer_statut(&avoir.numero, StatutDocument::Valide)?;
        Ok(avoir)
    }

    pub fn ajouter_paiement(
        &mut self,
        numero: &str,
        montant: Cents,
        date_paiement: &str,
        moyen: &str,
        reference: &str,
    ) -> Resultat<Document> {
        if montant <= 0 {
            return Err("le montant du paiement doit être positif".into());
        }
        if !date_valide(date_paiement) {
            return Err(format!("date de paiement invalide : « {date_paiement} »"));
        }
        let idx = self.index_document(numero)?;
        if matches!(
            self.documents[idx].statut,
            StatutDocument::Brouillon
                | StatutDocument::Annule
                | StatutDocument::Refuse
                | StatutDocument::Expire
        ) {
            return Err(format!(
                "{} est {} : validez la facture avant d'enregistrer un paiement",
                numero,
                self.documents[idx].statut.as_str()
            ));
        }
        let id = self.alloc_id();
        let document_id = self.documents[idx].id;
        self.paiements.push(Paiement {
            id,
            document_id,
            date_paiement: date_paiement.to_string(),
            montant,
            moyen: if moyen.trim().is_empty() {
                "virement".to_string()
            } else {
                moyen.to_string()
            },
            reference: reference.to_string(),
            rapproche: false,
        });
        self.documents[idx].paye += montant;
        let regime = self.societe_regime();
        let total = self.documents[idx].totaux(regime).ttc;
        let paye = self.documents[idx].paye;
        self.documents[idx].statut = statut_apres_paiement(total, paye);
        let document = self.documents[idx].clone();
        self.persister()?;
        Ok(document)
    }

    pub fn paiements(&self, document_id: i64) -> Vec<Paiement> {
        self.paiements
            .iter()
            .filter(|p| p.document_id == document_id)
            .cloned()
            .collect()
    }

    pub fn tous_paiements(&self) -> Vec<Paiement> {
        self.paiements.clone()
    }

    pub fn marquer_envoye(&mut self, numero: &str) -> Resultat<Document> {
        let document = self.changer_statut(numero, StatutDocument::Envoye)?;
        self.journaliser("document.envoi", &document.numero)?;
        Ok(document)
    }

    // ---------------------------------------------------------- Vinted

    pub fn creer_article(&mut self, mut article: Article) -> Resultat<Article> {
        if article.composition.trim().is_empty() {
            return Err(
                "composition textile obligatoire (règlement UE 1007/2011, CDC §11.3)".into(),
            );
        }
        if article.description.trim().is_empty() {
            return Err("description de l'article obligatoire".into());
        }
        if article.societe_id == 0 {
            article.societe_id = self.societe()?.id;
        }
        article.id = self.alloc_id();
        if article.statut.is_empty() {
            article.statut = "en_stock".to_string();
        }
        if article.cree_le.is_empty() {
            article.cree_le = aujourdhui();
        }
        if article.quantite <= 0 {
            article.quantite = 1;
        }
        self.articles.push(article.clone());
        self.persister()?;
        Ok(article)
    }

    pub fn articles(&self) -> Vec<Article> {
        let id = self.societe_courante;
        self.articles
            .iter()
            .filter(|a| id == 0 || a.societe_id == id)
            .cloned()
            .collect()
    }

    pub fn article_par_id(&self, id: i64) -> Resultat<Article> {
        self.articles
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| format!("article n°{id} introuvable"))
    }

    pub fn modifier_article(&mut self, article: Article) -> Resultat<Article> {
        let idx = self
            .articles
            .iter()
            .position(|a| a.id == article.id)
            .ok_or_else(|| format!("article n°{} introuvable", article.id))?;
        if article.composition.trim().is_empty() {
            return Err("composition textile obligatoire (règlement UE 1007/2011)".into());
        }
        self.articles[idx] = article.clone();
        self.persister()?;
        Ok(article)
    }

    pub fn enregistrer_vente(&mut self, mut vente: Vente) -> Resultat<Vente> {
        let idx = self
            .articles
            .iter()
            .position(|a| a.id == vente.article_id)
            .ok_or_else(|| format!("article n°{} introuvable", vente.article_id))?;
        if vente.prix_vente <= 0 {
            return Err("le prix de vente doit être positif".into());
        }
        if !date_valide(&vente.date_vente) {
            return Err(format!("date de vente invalide : « {} »", vente.date_vente));
        }
        vente.id = self.alloc_id();
        if vente.societe_id == 0 {
            vente.societe_id = self.societe_courante;
        }
        if vente.regime == RegimeVente::AchatRevente && self.articles[idx].regime == RegimeVente::Personnel
        {
            vente.regime = RegimeVente::Personnel;
        }
        if vente.cree_le.is_empty() {
            vente.cree_le = aujourdhui();
        }
        self.articles[idx].statut = "vendu".to_string();
        self.ventes.push(vente.clone());
        self.persister()?;
        Ok(vente)
    }

    pub fn ventes(&self) -> Vec<Vente> {
        let id = self.societe_courante;
        self.ventes
            .iter()
            .filter(|v| id == 0 || v.societe_id == id)
            .cloned()
            .collect()
    }

    pub fn ventes_annee(&self, annee: u32) -> Vec<Vente> {
        self.ventes()
            .into_iter()
            .filter(|v| annee_de(&v.date_vente) == annee)
            .collect()
    }

    /// Statistiques Vinted : marges par marque/taille, stock, DAC7 (EF-VIN-04).
    pub fn stats_vinted(&self, annee: u32) -> StatsVinted {
        let mut stats = StatsVinted::default();
        let mut marques: Vec<(String, u32, Cents)> = Vec::new();
        let mut tailles: Vec<(String, u32, Cents)> = Vec::new();
        for article in self.articles() {
            if article.en_stock() {
                stats.articles_en_stock += 1;
                stats.valeur_stock += article.prix_achat * article.quantite.max(1);
            } else {
                stats.articles_vendus += 1;
            }
        }
        let mut delais: Vec<i64> = Vec::new();
        for vente in self.ventes_annee(annee) {
            let article = match self.article_par_id(vente.article_id) {
                Ok(a) => a,
                Err(_) => continue,
            };
            if date_valide(&article.date_achat) && date_valide(&vente.date_vente) {
                let jours = jours_entre(&article.date_achat, &vente.date_vente);
                if jours >= 0 {
                    delais.push(jours);
                }
            }
            stats.nb_ventes += 1;
            match vente.regime {
                RegimeVente::AchatRevente => {
                    stats.ca += vente.prix_vente;
                    stats.marge += marge_nette(&article, &vente);
                }
                RegimeVente::Personnel => {
                    stats.ca_personnel += vente.prix_vente;
                }
            }
            let marge = marge_nette(&article, &vente);
            ajouter_stat(&mut marques, &article.marque, marge);
            ajouter_stat(&mut tailles, &article.taille, marge);
        }
        marques.sort_by(|a, b| b.2.cmp(&a.2));
        tailles.sort_by(|a, b| b.2.cmp(&a.2));
        stats.par_marque = marques;
        stats.par_taille = tailles;
        // Rotation (EF-VIN-04) : quelle part du stock a tourné, et en combien
        // de temps en moyenne. Deux indicateurs simples, lisibles au quotidien.
        let total = stats.articles_en_stock + stats.articles_vendus;
        stats.taux_ecoulement = if total == 0 {
            0
        } else {
            (stats.articles_vendus as i64 * 100) / total as i64
        };
        stats.delai_moyen_jours = if delais.is_empty() {
            0
        } else {
            delais.iter().sum::<i64>() / delais.len() as i64
        };
        stats
    }

    // ---------------------------------------------------------- Comptabilité (EF-CPT-01/02/03)

    pub fn ecritures(&self) -> Vec<Ecriture> {
        let id = self.societe_courante;
        self.ecritures
            .iter()
            .filter(|e| id == 0 || e.societe_id == id)
            .cloned()
            .collect()
    }

    /// (Re)génère toutes les écritures depuis les documents et paiements.
    pub fn regenerer_ecritures(&mut self) -> Resultat<usize> {
        let societe = self.societe()?;
        let clients = self.clients.clone();
        let documents = self.documents.clone();
        let paiements = self.paiements.clone();
        let ventes = self.ventes.clone();
        let articles = self.articles.clone();
        let mut ecritures = compta::ecritures_depuis(
            &societe,
            &clients,
            &documents,
            &paiements,
            &ventes,
            &articles,
        );
        for e in ecritures.iter_mut() {
            e.id = self.alloc_id();
        }
        let nombre = ecritures.len();
        self.ecritures = ecritures;
        self.persister()?;
        Ok(nombre)
    }

    pub fn operations(&self) -> Vec<OperationBancaire> {
        let id = self.societe_courante;
        self.operations
            .iter()
            .filter(|o| id == 0 || o.societe_id == id)
            .cloned()
            .collect()
    }

    pub fn importer_releve(&mut self, operations: Vec<OperationBancaire>) -> Resultat<usize> {
        let societe_id = self.societe()?.id;
        let mut importees = 0;
        for mut operation in operations {
            operation.id = self.alloc_id();
            operation.societe_id = societe_id;
            self.operations.push(operation);
            importees += 1;
        }
        self.persister()?;
        Ok(importees)
    }

    /// Rapprochement bancaire simple : montant identique, date à ±10 jours.
    pub fn lettrer(&mut self) -> Resultat<usize> {
        let mut lettres = 0;
        let operations = self.operations.clone();
        for operation in operations {
            if operation.paiement_id != 0 {
                continue;
            }
            let candidat = self
                .paiements
                .iter()
                .filter(|p| !p.rapproche && p.montant == operation.montant)
                .find(|p| jours_entre(&p.date_paiement, &operation.date_operation).abs() <= 10)
                .cloned();
            if let Some(paiement) = candidat {
                if let Some(p) = self.paiements.iter_mut().find(|p| p.id == paiement.id) {
                    p.rapproche = true;
                    let lettrage = lettre_auto(p.id);
                    p.reference = if p.reference.is_empty() {
                        lettrage.clone()
                    } else {
                        p.reference.clone()
                    };
                }
                if let Some(o) = self.operations.iter_mut().find(|o| o.id == operation.id) {
                    o.paiement_id = paiement.id;
                }
                lettres += 1;
            }
        }
        if lettres > 0 {
            self.persister()?;
        }
        Ok(lettres)
    }

    /// Chiffre d'affaires annuel (ventes, services) en devise de référence.
    pub fn ca_annuel(&self, annee: u32) -> (Cents, Cents) {
        let regime = self.societe_regime();
        let mut ventes_ca: Cents = 0;
        let mut services_ca: Cents = 0;
        for d in self.documents(None) {
            if annee_de(&d.date_doc) != annee
                || matches!(
                    d.statut,
                    StatutDocument::Annule | StatutDocument::Brouillon | StatutDocument::Refuse
                )
            {
                continue;
            }
            let total = d.totaux_reference(regime).ttc;
            if d.activite == "ventes" {
                ventes_ca += total;
            } else {
                services_ca += total;
            }
        }
        for v in self.ventes_annee(annee) {
            if v.regime == RegimeVente::AchatRevente {
                ventes_ca += v.prix_vente;
            }
        }
        (ventes_ca, services_ca)
    }

    /// CA mois par mois (EF-CPT-05).
    pub fn ca_mensuel(&self, annee: u32) -> Vec<(u32, Cents, Cents)> {
        let regime = self.societe_regime();
        let mut mois: Vec<(u32, Cents, Cents)> = (1..=12).map(|m| (m, 0, 0)).collect();
        for d in self.documents(None) {
            if annee_de(&d.date_doc) != annee
                || matches!(
                    d.statut,
                    StatutDocument::Annule | StatutDocument::Brouillon | StatutDocument::Refuse
                )
            {
                continue;
            }
            let m = mois_de(&d.date_doc);
            let total = d.totaux_reference(regime).ttc;
            if let Some(entree) = mois.iter_mut().find(|e| e.0 == m) {
                if d.activite == "ventes" {
                    entree.1 += total;
                } else {
                    entree.2 += total;
                }
            }
        }
        for v in self.ventes_annee(annee) {
            if v.regime == RegimeVente::AchatRevente {
                let m = mois_de(&v.date_vente);
                if let Some(entree) = mois.iter_mut().find(|e| e.0 == m) {
                    entree.1 += v.prix_vente;
                }
            }
        }
        mois
    }

    /// Encours client (reste à encaisser, converti en euros).
    pub fn encours(&self) -> Cents {
        let regime = self.societe_regime();
        self.documents_impayes()
            .iter()
            .map(|d| convertir(d.reste(regime), d.taux_change_micro))
            .sum()
    }

    /// Encaissements cumulés (trésorerie simple, EF-CPT-05).
    pub fn tresorerie(&self, annee: Option<u32>) -> Cents {
        let mut total = 0;
        for paiement in &self.paiements {
            if let Some(annee) = annee {
                if annee_de(&paiement.date_paiement) != annee {
                    continue;
                }
            }
            if let Some(doc) = self.documents.iter().find(|d| d.id == paiement.document_id) {
                if doc.societe_id != self.societe_courante && self.societe_courante != 0 {
                    continue;
                }
                total += convertir(paiement.montant, doc.taux_change_micro);
            }
        }
        total
    }

    // ---------------------------------------------------------- Récurrences (EF-FAC-08)

    pub fn creer_recurrence(&mut self, mut recurrence: Recurrence) -> Resultat<Recurrence> {
        if recurrence.lignes.is_empty() {
            return Err("une facture récurrente doit avoir au moins une ligne".into());
        }
        self.client_par_id(recurrence.client_id)?;
        if !date_valide(&recurrence.prochaine_le) {
            return Err(format!(
                "date de prochaine génération invalide : « {} »",
                recurrence.prochaine_le
            ));
        }
        recurrence.id = self.alloc_id();
        recurrence.societe_id = self.societe_courante;
        if recurrence.periodicite.trim().is_empty() {
            recurrence.periodicite = "mensuelle".to_string();
        }
        self.recurrences.push(recurrence.clone());
        self.persister()?;
        Ok(recurrence)
    }

    pub fn recurrences(&self) -> Vec<Recurrence> {
        let id = self.societe_courante;
        self.recurrences
            .iter()
            .filter(|r| id == 0 || r.societe_id == id)
            .cloned()
            .collect()
    }

    pub fn basculer_recurrence(&mut self, id: i64, actif: bool) -> Resultat<Recurrence> {
        let idx = self
            .recurrences
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("facture récurrente n°{id} introuvable"))?;
        self.recurrences[idx].actif = actif;
        let recurrence = self.recurrences[idx].clone();
        self.persister()?;
        Ok(recurrence)
    }

    /// Génère les brouillons dus jusqu'à la date demandée et avance l'échéancier.
    pub fn generer_recurrences(&mut self, jusqu_a: &str) -> Resultat<Vec<Document>> {
        let mut crees = Vec::new();
        let mut a_traiter: Vec<Recurrence> = self
            .recurrences()
            .into_iter()
            .filter(|r| r.actif && r.prochaine_le.as_str() <= jusqu_a)
            .collect();
        a_traiter.sort_by(|a, b| a.prochaine_le.cmp(&b.prochaine_le));
        for recurrence in a_traiter {
            let mut date = recurrence.prochaine_le.clone();
            let mut garde = 0;
            while date.as_str() <= jusqu_a && garde < 36 {
                let document = self.creer_document(NouveauDocument {
                    type_doc: recurrence.type_doc,
                    client_id: recurrence.client_id,
                    date_doc: date.clone(),
                    date_prestation: date.clone(),
                    echeance: String::new(),
                    activite: recurrence.activite.clone(),
                    devise: DEVISE_REFERENCE.to_string(),
                    taux_change_micro: 1_000_000,
                    lignes: recurrence.lignes.clone(),
                    motif: format!("récurrence #{} ({})", recurrence.id, recurrence.libelle),
                })?;
                crees.push(document);
                date = avancer_periode(&date, &recurrence.periodicite);
                garde += 1;
            }
            if let Some(r) = self.recurrences.iter_mut().find(|r| r.id == recurrence.id) {
                r.prochaine_le = date;
            }
        }
        if !crees.is_empty() {
            self.persister()?;
        }
        Ok(crees)
    }

    // ---------------------------------------------------------- Audit (SEC-05)

    pub fn journaliser(&mut self, action: &str, detail: &str) -> Resultat<()> {
        let societe_id = self.societe_courante;
        let precedent = self
            .audit
            .last()
            .map(|e| e.empreinte.clone())
            .unwrap_or_default();
        let horodatage = horodatage();
        let utilisateur = self.session.clone();
        let empreinte = orm::empreinte(&format!(
            "{precedent}|{horodatage}|{utilisateur}|{action}|{detail}"
        ));
        let id = self.alloc_id();
        self.audit.push(EntreeAudit {
            id,
            societe_id,
            horodatage,
            utilisateur,
            action: action.to_string(),
            detail: detail.to_string(),
            empreinte_precedente: precedent,
            empreinte,
        });
        self.persister()
    }

    /// Journal d'audit, du plus récent au plus ancien.
    pub fn audit(&self, limite: usize) -> Vec<EntreeAudit> {
        let mut entrees = self.audit.clone();
        entrees.reverse();
        entrees.truncate(limite);
        entrees
    }

    pub fn audit_complet(&self) -> Vec<EntreeAudit> {
        self.audit.clone()
    }

    /// Vérifie la chaîne d'empreintes : toute altération est détectée.
    pub fn verifier_chaine_audit(&self) -> Resultat<usize> {
        let mut precedent = String::new();
        for (index, entree) in self.audit.iter().enumerate() {
            if entree.empreinte_precedente != precedent {
                return Err(format!(
                    "journal d'audit altéré à l'entrée n°{} ({}) : chaîne rompue",
                    index + 1,
                    entree.horodatage
                ));
            }
            let attendu = orm::empreinte(&format!(
                "{}|{}|{}|{}|{}",
                entree.empreinte_precedente,
                entree.horodatage,
                entree.utilisateur,
                entree.action,
                entree.detail
            ));
            if attendu != entree.empreinte {
                return Err(format!(
                    "journal d'audit altéré à l'entrée n°{} ({}) : empreinte invalide",
                    index + 1,
                    entree.horodatage
                ));
            }
            precedent = entree.empreinte.clone();
        }
        Ok(self.audit.len())
    }

    // ---------------------------------------------------------- Sauvegardes (CDC §14)

    pub fn creer_sauvegarde(&mut self) -> Resultat<Sauvegarde> {
        if self.en_base() {
            return Err("en mode PostgreSQL, la sauvegarde est prise par `pg_dump` \
                 (hors du binaire : copie à chaud gérée par le serveur) — \
                 /admin export reste disponible pour un état daté en TSV"
                .into());
        }
        let dossier = self
            .disque
            .clone()
            .ok_or_else(|| "sauvegarde impossible en mode mémoire".to_string())?;
        self.persister()?;
        let nom = horodatage().replace(':', "-").replace(' ', "_");
        let cible = dossier.join("sauvegardes").join(&nom);
        std::fs::create_dir_all(&cible)
            .map_err(|e| format!("création de la sauvegarde : {e}"))?;
        let mut empreinte_source = String::new();
        let mut taille = 0i64;
        for (table, _) in orm::tables() {
            let source = fichier::chemin_table(&dossier, table);
            if let Ok(contenu) = std::fs::read_to_string(&source) {
                taille += contenu.len() as i64;
                empreinte_source.push_str(&contenu);
                std::fs::write(cible.join(format!("{table}.tsv")), contenu)
                    .map_err(|e| format!("copie de {table} : {e}"))?;
            }
        }
        if let Ok(meta) = std::fs::read_to_string(dossier.join("meta.tsv")) {
            taille += meta.len() as i64;
            empreinte_source.push_str(&meta);
            let _ = std::fs::write(cible.join("meta.tsv"), meta);
        }
        let sauvegarde = Sauvegarde {
            id: self.alloc_id(),
            societe_id: self.societe_courante,
            date_sauvegarde: nom,
            chemin: cible.to_string_lossy().to_string(),
            empreinte: orm::empreinte(&empreinte_source),
            taille_octets: taille,
        };
        self.sauvegardes.push(sauvegarde.clone());
        self.persister()?;
        Ok(sauvegarde)
    }

    pub fn sauvegardes(&self) -> Vec<Sauvegarde> {
        let mut liste = self.sauvegardes.clone();
        liste.sort_by(|a, b| b.date_sauvegarde.cmp(&a.date_sauvegarde));
        liste
    }

    /// Restauration complète (OBJ-5 : ≤ 30 min).
    pub fn restaurer_sauvegarde(&mut self, nom: &str) -> Resultat<String> {
        if self.en_base() {
            return Err("en mode PostgreSQL, la restauration se fait par `pg_restore` \
                 ou une transaction SQL — la commande n'est pas exposée par l'ERP"
                .into());
        }
        let dossier = self
            .disque
            .clone()
            .ok_or_else(|| "restauration impossible en mode mémoire".to_string())?;
        let sauvegarde = self
            .sauvegardes
            .iter()
            .find(|s| s.date_sauvegarde == nom)
            .cloned()
            .ok_or_else(|| format!("sauvegarde « {nom} » inconnue"))?;
        let source = PathBuf::from(&sauvegarde.chemin);
        if !source.exists() {
            return Err(format!("dossier de sauvegarde absent : {}", source.display()));
        }
        for (table, _) in orm::tables() {
            let fichier_source = source.join(format!("{table}.tsv"));
            if fichier_source.exists() {
                let cible = fichier::chemin_table(&dossier, table);
                std::fs::create_dir_all(fichier::dossier_tables(&dossier))
                    .map_err(|e| format!("restauration : {e}"))?;
                std::fs::copy(&fichier_source, &cible)
                    .map_err(|e| format!("restauration de {table} : {e}"))?;
            }
        }
        if source.join("meta.tsv").exists() {
            let _ = std::fs::copy(source.join("meta.tsv"), dossier.join("meta.tsv"));
        }
        let mut restaure = Store::ouvrir(&dossier)?;
        restaure
            .journaliser("sauvegarde.restauration", &sauvegarde.date_sauvegarde)
            .ok();
        *self = restaure;
        Ok(format!(
            "sauvegarde {} restaurée ({} octets, empreinte {})",
            sauvegarde.date_sauvegarde, sauvegarde.taille_octets, sauvegarde.empreinte
        ))
    }

    // ---------------------------------------------------------- Recherche globale (EF-SOC-03)

    pub fn recherche_globale(&self, recherche: &str) -> ResultatsRecherche {
        let recherche = recherche.trim();
        let mut resultats = ResultatsRecherche::default();
        if recherche.is_empty() {
            return resultats;
        }
        resultats.clients = self.chercher_clients(recherche);
        resultats.documents = self
            .documents(None)
            .into_iter()
            .filter(|d| {
                correspond(&d.numero, recherche)
                    || d.lignes
                        .iter()
                        .any(|l| correspond(&l.designation, recherche))
            })
            .collect();
        resultats.articles = self
            .articles()
            .into_iter()
            .filter(|a| {
                correspond(&a.marque, recherche)
                    || correspond(&a.description, recherche)
                    || correspond(&a.taille, recherche)
                    || correspond(&a.composition, recherche)
            })
            .collect();
        resultats.ventes = self
            .ventes()
            .into_iter()
            .filter(|v| {
                correspond(&v.reference, recherche)
                    || correspond(&v.acheteur, recherche)
                    || correspond(&v.plateforme, recherche)
            })
            .collect();
        resultats
    }

    // ---------------------------------------------------------- Reprise de la v1 (etat.tsv)

    fn importer_etat_v1(&mut self, chemin: &Path) -> Resultat<usize> {
        let contenu = std::fs::read_to_string(chemin)
            .map_err(|e| format!("lecture de {} : {e}", chemin.display()))?;
        let mut compte = 0;
        let mut societe = Societe::default();
        societe.id = self.alloc_id();
        let mut lignes_documents: Vec<(i64, Ligne)> = Vec::new();
        let mut documents: Vec<Document> = Vec::new();
        for brute in contenu.lines() {
            if brute.trim().is_empty() {
                continue;
            }
            let champs: Vec<String> = brute.split('\t').map(desechapper_v1).collect();
            let g = |i: usize| champs.get(i).cloned().unwrap_or_default();
            let gi = |i: usize| g(i).parse::<i64>().unwrap_or(0);
            match champs.first().map(|s| s.as_str()) {
                Some("SOC") => {
                    societe.nom = g(1);
                    societe.regime = RegimeTva::from_str(&g(2)).unwrap_or(RegimeTva::Franchise);
                    if !g(3).is_empty() {
                        societe.devise = g(3);
                    }
                    societe.siren = g(4);
                    compte += 1;
                }
                Some("META") => {
                    self.prochain_id = self.prochain_id.max(gi(1));
                }
                Some("SEQ") => {
                    let cle = g(1);
                    let valeur = g(2).parse::<i64>().unwrap_or(0);
                    if let Some((type_doc, annee)) = cle.split_once('-') {
                        let annee: i64 = annee.parse().unwrap_or(0);
                        let id = self.alloc_id();
                        self.sequences.push(Sequence {
                            id,
                            societe_id: societe.id,
                            type_doc: type_doc.to_string(),
                            annee,
                            valeur,
                        });
                    }
                }
                Some("CLI") => {
                    let mut client = Client::default();
                    client.id = gi(1);
                    self.prochain_id = self.prochain_id.max(client.id + 1);
                    client.societe_id = societe.id;
                    client.nom = g(2);
                    client.type_client =
                        TypeClient::from_str(&g(3)).unwrap_or(TypeClient::Professionnel);
                    client.email = g(4);
                    client.telephone = g(5);
                    client.adresse = g(6);
                    client.siren = g(7);
                    client.tva_intracom = g(8);
                    client.notes = g(9);
                    self.clients.push(client);
                    compte += 1;
                }
                Some("DOC") => {
                    let mut doc = Document::default();
                    doc.id = gi(1);
                    self.prochain_id = self.prochain_id.max(doc.id + 1);
                    doc.societe_id = societe.id;
                    doc.numero = g(2);
                    doc.type_doc =
                        TypeDocument::from_str(&g(3)).unwrap_or(TypeDocument::Facture);
                    doc.statut = statut_v1(&g(4));
                    doc.client_id = gi(5);
                    doc.date_doc = g(6);
                    doc.date_prestation = g(6);
                    doc.echeance = g(7);
                    doc.activite = if g(8).is_empty() { "services".into() } else { g(8) };
                    doc.paye = gi(9);
                    documents.push(doc);
                    compte += 1;
                }
                Some("LIG") => {
                    let document_id = gi(1);
                    let mut ligne = Ligne::default();
                    ligne.document_id = document_id;
                    ligne.designation = g(2);
                    ligne.quantite_milli = gi(3);
                    ligne.prix_unitaire_ht = gi(4);
                    ligne.tva_bp = gi(5) as i32;
                    lignes_documents.push((document_id, ligne));
                }
                Some("PAI") => {
                    let mut paiement = Paiement {
                        id: gi(1),
                        document_id: gi(2),
                        date_paiement: g(3),
                        montant: gi(4),
                        moyen: g(5),
                        reference: String::new(),
                        rapproche: false,
                    };
                    if paiement.id == 0 {
                        paiement.id = self.alloc_id();
                    } else {
                        self.prochain_id = self.prochain_id.max(paiement.id + 1);
                    }
                    self.paiements.push(paiement);
                    compte += 1;
                }
                Some("ART") => {
                    let mut article = Article::default();
                    article.id = gi(1);
                    self.prochain_id = self.prochain_id.max(article.id + 1);
                    article.societe_id = societe.id;
                    article.marque = g(2);
                    article.description = g(3);
                    article.taille = g(4);
                    article.composition = g(5);
                    article.etat = g(6);
                    article.prix_achat = gi(7);
                    article.date_achat = g(8);
                    article.statut = g(9);
                    self.articles.push(article);
                    compte += 1;
                }
                Some("VEN") => {
                    let mut vente = Vente::default();
                    vente.id = gi(1);
                    self.prochain_id = self.prochain_id.max(vente.id + 1);
                    vente.societe_id = societe.id;
                    vente.article_id = gi(2);
                    vente.date_vente = g(3);
                    vente.prix_vente = gi(4);
                    vente.frais = gi(5);
                    vente.plateforme = g(6);
                    self.ventes.push(vente);
                    compte += 1;
                }
                Some("AUD") => {
                    let id = self.alloc_id();
                    self.audit.push(EntreeAudit {
                        id,
                        societe_id: societe.id,
                        horodatage: g(1),
                        utilisateur: "import-v1".to_string(),
                        action: g(2),
                        detail: g(3),
                        empreinte_precedente: String::new(),
                        empreinte: String::new(),
                    });
                }
                _ => {}
            }
        }
        for mut doc in documents {
            for (document_id, ligne) in &lignes_documents {
                if *document_id == doc.id {
                    let mut ligne = ligne.clone();
                    ligne.id = self.alloc_id();
                    doc.lignes.push(ligne);
                }
            }
            self.documents.push(doc);
        }
        if self.societes.is_empty() {
            societe.cree_le = aujourdhui();
            self.societe_courante = societe.id;
            self.societes.push(societe);
        }
        // Les empreintes de l'ancien journal ne sont pas chaînées : on repart
        // d'une chaîne neuve tout en conservant les lignes importées.
        for entree in self.audit.iter_mut() {
            let precedent = entree.empreinte_precedente.clone();
            entree.empreinte = orm::empreinte(&format!(
                "{}|{}|{}|{}|{}",
                precedent, entree.horodatage, entree.utilisateur, entree.action, entree.detail
            ));
        }
        Ok(compte)
    }
}

// ------------------------------------------------------------ Aides internes

fn ajouter_stat(liste: &mut Vec<(String, u32, Cents)>, cle: &str, montant: Cents) {
    let cle = if cle.trim().is_empty() {
        "inconnu".to_string()
    } else {
        cle.to_string()
    };
    match liste.iter_mut().find(|e| e.0 == cle) {
        Some(entree) => {
            entree.1 += 1;
            entree.2 += montant;
        }
        None => liste.push((cle, 1, montant)),
    }
}

/// Lettrage lisible : L + 6 chiffres.
fn lettre_auto(paiement_id: i64) -> String {
    format!("L{:06}", paiement_id % 1_000_000)
}

/// Avance une date selon la périodicité d'une facture récurrente.
pub fn avancer_periode(date: &str, periodicite: &str) -> String {
    let mois = match periodicite.trim().to_lowercase().as_str() {
        "annuelle" | "annuel" => 12,
        "trimestrielle" | "trimestriel" => 3,
        _ => 1,
    };
    ajouter_mois(date, mois)
}

/// Ajoute des mois en gardant le jour quand il existe (31 → 28/29/30).
pub fn ajouter_mois(date: &str, mois: i64) -> String {
    let annee = annee_de(date) as i64;
    let mois_actuel = mois_de(date) as i64;
    let jour = date.get(8..10).and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
    let index = annee * 12 + (mois_actuel - 1) + mois;
    let nouvelle_annee = index.div_euclid(12);
    let nouveau_mois = index.rem_euclid(12) + 1;
    let jour = jour.min(i64::from(jours_du_mois(nouvelle_annee as u32, nouveau_mois as u32)));
    format!("{nouvelle_annee:04}-{nouveau_mois:02}-{jour:02}")
}

fn desechapper_v1(s: &str) -> String {
    s.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%09", "\t")
        .replace("%25", "%")
}

fn statut_v1(texte: &str) -> StatutDocument {
    match texte {
        "envoyé" => StatutDocument::Envoye,
        "accepté" => StatutDocument::Accepte,
        "refusé" => StatutDocument::Refuse,
        "validée" => StatutDocument::Valide,
        "payée" => StatutDocument::Paye,
        "partiellement payée" => StatutDocument::PartiellementPaye,
        "annulée" => StatutDocument::Annule,
        "expiré" => StatutDocument::Expire,
        autre => StatutDocument::depuis_code(autre),
    }
}

// ------------------------------------------------------------ Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn client_test(s: &mut Store) -> Client {
        s.creer_client(Client {
            nom: "Atelier Dupont".into(),
            email: "contact@example.fr".into(),
            siren: "123456789".into(),
            ..Client::default()
        })
        .unwrap()
    }

    fn societe_complete(s: &mut Store) {
        let mut societe = s.societe().unwrap();
        societe.siren = "123456789".into();
        societe.adresse = "1 rue de la Paix".into();
        societe.ville = "Nice".into();
        societe.code_postal = "06000".into();
        s.modifier_societe(societe).unwrap();
    }

    fn facture_test(s: &mut Store, client_id: i64) -> Document {
        s.creer_document(
            NouveauDocument::nouveau(TypeDocument::Facture, client_id, "2026-10-30")
                .avec_activite("services")
                .avec_lignes(vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)]),
        )
        .unwrap()
    }

    #[test]
    fn parcours_facture_complet() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        assert_eq!(doc.numero, "FA-2026-0001");
        assert_eq!(doc.echeance, "2026-11-29"); // 30 jours par défaut
        s.valider_document(&doc.numero).unwrap();
        let doc = s
            .ajouter_paiement(&doc.numero, 50_000, "2026-11-02", "virement", "")
            .unwrap();
        assert_eq!(doc.statut, StatutDocument::PartiellementPaye);
        let doc = s
            .ajouter_paiement(&doc.numero, 50_000, "2026-11-05", "virement", "")
            .unwrap();
        assert_eq!(doc.statut, StatutDocument::Paye);
        let (ventes, services) = s.ca_annuel(2026);
        assert_eq!(services, 100_000);
        assert_eq!(ventes, 0);
        assert_eq!(s.encours(), 0);
        assert_eq!(s.tresorerie(None), 100_000);
    }

    #[test]
    fn numerotation_sans_trou_et_immuabilite() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let d1 = facture_test(&mut s, c.id);
        let d2 = facture_test(&mut s, c.id);
        let d3 = facture_test(&mut s, c.id);
        assert_eq!(d1.numero, "FA-2026-0001");
        assert_eq!(d2.numero, "FA-2026-0002");
        assert_eq!(d3.numero, "FA-2026-0003");
        s.valider_document(&d1.numero).unwrap();
        let erreur = s.valider_document(&d1.numero).unwrap_err();
        assert!(erreur.contains("immuable") || erreur.contains("déjà"));
        // Une facture validée ne peut plus être annulée pour être refaite : avoir obligatoire.
        let avoir = s.creer_avoir(&d1.numero, "2026-11-02").unwrap();
        assert_eq!(avoir.numero, "AV-2026-0001");
        assert_eq!(avoir.lignes[0].total_ht(), -100_000);
        let (_, services) = s.ca_annuel(2026);
        assert_eq!(services, 0); // la facture est annulée par l'avoir
    }

    #[test]
    fn validation_refusee_sans_mentions_obligatoires() {
        let mut s = Store::memoire();
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        let erreur = s.valider_document(&doc.numero).unwrap_err();
        assert!(erreur.contains("SIREN"), "{erreur}");
        societe_complete(&mut s);
        assert!(s.valider_document(&doc.numero).is_ok());
    }

    #[test]
    fn paiement_refuse_sur_brouillon() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        assert!(s
            .ajouter_paiement(&doc.numero, 10_000, "2026-11-02", "virement", "")
            .is_err());
    }

    #[test]
    fn devis_transforme_en_facture() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let devis = s
            .creer_document(
                NouveauDocument::nouveau(TypeDocument::Devis, c.id, "2026-10-01")
                    .avec_lignes(vec![Ligne::new("Audit", 1.0, 90_000, 2000)]),
            )
            .unwrap();
        assert_eq!(devis.numero, "DEV-2026-0001");
        assert!(s.marquer_envoye(&devis.numero).is_ok());
        let facture = s.transformer_devis(&devis.numero, "2026-10-15").unwrap();
        assert_eq!(facture.numero, "FA-2026-0001");
        assert_eq!(facture.lignes[0].prix_unitaire_ht, 90_000);
        let devis = s.document_par_numero(&devis.numero).unwrap();
        assert_eq!(devis.statut, StatutDocument::Accepte);
    }

    #[test]
    fn recherche_client_floue_et_etiquettes() {
        let mut s = Store::memoire();
        client_test(&mut s);
        assert_eq!(s.chercher_clients("dupont").len(), 1);
        assert_eq!(s.chercher_clients("DUPONT").len(), 1);
        assert_eq!(s.chercher_clients("xyz").len(), 0);
        let c = s.chercher_clients("dupont")[0].clone();
        s.etiqueter(c.id, "maintenance").unwrap();
        assert_eq!(s.clients_par_etiquette("MAINTENANCE").len(), 1);
        s.retirer_etiquette(c.id, "maintenance").unwrap();
        assert_eq!(s.clients_par_etiquette("maintenance").len(), 0);
    }

    #[test]
    fn article_sans_composition_refuse() {
        let mut s = Store::memoire();
        let mut article = Article::nouveau("X", "T-shirt");
        article.composition = String::new();
        assert!(s.creer_article(article).is_err());
    }

    #[test]
    fn ventes_vinted_et_stats() {
        let mut s = Store::memoire();
        let mut article = Article::nouveau("Zara", "Veste");
        article.composition = "100 % coton".into();
        article.prix_achat = 800;
        let article = s.creer_article(article).unwrap();
        let vente = s
            .enregistrer_vente(Vente {
                article_id: article.id,
                prix_vente: 2_500,
                frais: 200,
                date_vente: "2026-10-20".into(),
                ..Vente::default()
            })
            .unwrap();
        assert_eq!(marge_nette(&s.article_par_id(article.id).unwrap(), &vente), 1_500);
        let stats = s.stats_vinted(2026);
        assert_eq!(stats.nb_ventes, 1);
        assert_eq!(stats.marge, 1_500);
        assert_eq!(stats.articles_vendus, 1);
        assert_eq!(stats.par_marque[0].0, "Zara");
        let (ventes, _) = s.ca_annuel(2026);
        assert_eq!(ventes, 2_500);
    }

    #[test]
    fn multi_societes_cloisonnees() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        facture_test(&mut s, c.id);
        let mut seconde = Societe::default();
        seconde.nom = "Vinted Pro".into();
        s.creer_societe(seconde).unwrap();
        assert_eq!(s.clients().len(), 0); // cloisonnement
        assert_eq!(s.documents(None).len(), 0);
        s.basculer_societe("micro").unwrap();
        assert_eq!(s.clients().len(), 1);
        // La numérotation est propre à chaque société.
        s.basculer_societe("vinted").unwrap();
        let c2 = client_test(&mut s);
        let doc = facture_test(&mut s, c2.id);
        assert_eq!(doc.numero, "FA-2026-0001");
    }

    #[test]
    fn audit_chaine_et_alteration_detectee() {
        let mut s = Store::memoire();
        s.journaliser("test", "première").unwrap();
        s.journaliser("test", "seconde").unwrap();
        assert_eq!(s.verifier_chaine_audit().unwrap(), 2);
        s.audit[1].detail = "falsifié".to_string();
        assert!(s.verifier_chaine_audit().is_err());
    }

    #[test]
    fn devises_et_conversion() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        s.enregistrer_taux("USD", "2026-10-01", 920_000, "BCE").unwrap();
        let taux = s.taux("USD", "2026-10-15").unwrap();
        assert_eq!(taux.taux_micro, 920_000);
        // 100 USD facturés = 92 EUR de CA.
        let doc = s
            .creer_document(
                NouveauDocument::nouveau(TypeDocument::Facture, c.id, "2026-10-15")
                    .avec_devise("USD", 920_000)
                    .avec_lignes(vec![Ligne::new("Prestation", 1.0, 10_000, 0)]),
            )
            .unwrap();
        s.valider_document(&doc.numero).unwrap();
        let (_, services) = s.ca_annuel(2026);
        assert_eq!(services, 9_200);
        assert_eq!(s.taux("USD", "2026-09-01"), None);
    }

    #[test]
    fn ecritures_et_fec_complets() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        s.valider_document(&doc.numero).unwrap();
        s.ajouter_paiement(&doc.numero, 100_000, "2026-11-02", "virement", "")
            .unwrap();
        let nombre = s.regenerer_ecritures().unwrap();
        assert!(nombre >= 4, "écritures : {nombre}");
        let ecritures = s.ecritures();
        let debit: Cents = ecritures.iter().map(|e| e.debit).sum();
        let credit: Cents = ecritures.iter().map(|e| e.credit).sum();
        assert_eq!(debit, credit, "la balance doit être équilibrée");
        let fec = compta::fec(&ecritures);
        assert!(fec[0].starts_with("JournalCode|JournalLib|EcritureNum"));
        assert!(fec.iter().any(|l| l.contains("411000")));
    }

    #[test]
    fn rapprochement_bancaire() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        s.valider_document(&doc.numero).unwrap();
        s.ajouter_paiement(&doc.numero, 100_000, "2026-11-02", "virement", "")
            .unwrap();
        s.importer_releve(vec![OperationBancaire {
            id: 0,
            societe_id: 0,
            date_operation: "2026-11-04".into(),
            libelle: "VIR ATELIER DUPONT".into(),
            montant: 100_000,
            reference: "VIR-1".into(),
            paiement_id: 0,
        }])
        .unwrap();
        assert_eq!(s.lettrer().unwrap(), 1);
        assert!(s.tous_paiements()[0].rapproche);
    }

    #[test]
    fn recurrences_generees() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        s.creer_recurrence(Recurrence {
            id: 0,
            societe_id: 0,
            client_id: c.id,
            type_doc: TypeDocument::Facture,
            activite: "services".into(),
            periodicite: "mensuelle".into(),
            prochaine_le: "2026-01-15".into(),
            lignes: vec![Ligne::new("Maintenance mensuelle", 1.0, 50_000, 2000)],
            actif: true,
            libelle: "contrat maintenance".into(),
        })
        .unwrap();
        let crees = s.generer_recurrences("2026-03-31").unwrap();
        assert_eq!(crees.len(), 3);
        assert_eq!(crees[0].numero, "FA-2026-0001");
        assert_eq!(crees[2].numero, "FA-2026-0003");
        assert_eq!(s.recurrences()[0].prochaine_le, "2026-04-15");
    }

    #[test]
    fn rgpd_effacement_conserve_les_pieces() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        let doc = facture_test(&mut s, c.id);
        s.valider_document(&doc.numero).unwrap();
        let resume = s.effacer_client_rgpd(c.id).unwrap();
        assert!(resume.contains("anonymisé"));
        let client = s.client_par_id(c.id).unwrap();
        assert!(client.email.is_empty());
        assert!(client.nom.contains("anonymisé"));
        assert!(!client.rgpd_efface_le.is_empty());
        assert_eq!(s.documents(None).len(), 1); // pièce comptable conservée
    }

    #[test]
    fn utilisateurs_et_roles() {
        let mut s = Store::memoire();
        let mut admin = Utilisateur::nouveau("jojo", Role::Admin);
        admin.empreinte = crate::securite::hacher_mot_de_passe("Correct-Horse-2026!");
        let admin = s.creer_utilisateur(admin).unwrap();
        assert!(s.exiger_admin().is_err()); // session « local » ≠ admin
        s.etablir_session(&admin.nom_utilisateur).unwrap();
        assert!(s.exiger_admin().is_ok());
        let mut utilisateur = Utilisateur::nouveau("stagiaire", Role::Utilisateur);
        utilisateur.empreinte = crate::securite::hacher_mot_de_passe("Autre-Mot-2026!");
        s.creer_utilisateur(utilisateur).unwrap();
        s.etablir_session("stagiaire").unwrap();
        assert!(s.exiger_admin().is_err());
        s.fermer_session().unwrap();
        assert_eq!(s.session(), "local");
    }

    #[test]
    fn prospects_clients_conversion_et_filtres() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let prospect = s
            .creer_client(Client::prospect("Piste Nice", TypeClient::Professionnel))
            .unwrap();
        s.creer_client(Client::nouveau("Client Nice", TypeClient::Professionnel))
            .unwrap();
        assert_eq!(s.prospects().len(), 1);
        assert_eq!(s.clients_actifs().len(), 1);
        assert_eq!(s.clients().len(), 2, "les deux fiches sont listées");
        assert_eq!(s.prospects()[0].nom, "Piste Nice");

        // Un prospect peut recevoir un devis…
        let devis = s
            .creer_document(
                NouveauDocument::nouveau(TypeDocument::Devis, prospect.id, "2026-10-30")
                    .avec_activite("services")
                    .avec_lignes(vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)]),
            )
            .unwrap();
        assert!(s.valider_document(&devis.numero).is_ok());

        // …mais pas une facture, même en forçant la main (le refus porte sur le
        // statut, pas sur les mentions : la société est complète).
        let facture = facture_test(&mut s, prospect.id);
        let erreur = s.valider_document(&facture.numero).unwrap_err();
        assert!(erreur.contains("encore un prospect"), "{erreur}");
        assert!(erreur.contains("convertir"), "{erreur}");

        // La conversion débloque la facturation et n'est pas rejouable.
        let converti = s.convertir_client(prospect.id).unwrap();
        assert!(!converti.est_prospect());
        assert!(!converti.modifie_le.is_empty());
        assert!(s.valider_document(&facture.numero).is_ok());
        assert_eq!(s.prospects().len(), 0);
        assert_eq!(s.clients_actifs().len(), 2);
        let erreur = s.convertir_client(prospect.id).unwrap_err();
        assert!(erreur.contains("déjà un client"), "{erreur}");
    }

    #[test]
    fn dernier_contact_et_prospects_perimes_rgpd() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        // Prospect vivant : un devis daté d'aujourd'hui.
        let vivant = s
            .creer_client(Client::prospect("Piste récente", TypeClient::Professionnel))
            .unwrap();
        s.creer_document(
            NouveauDocument::nouveau(TypeDocument::Devis, vivant.id, "2026-11-02")
                .avec_activite("services")
                .avec_lignes(vec![Ligne::new("Étude", 1.0, 10_000, 2_000)]),
        )
        .unwrap();
        // Prospect dormant : aucune pièce, fiche créée il y a plus de 3 ans.
        let dormant = s
            .creer_client(Client::prospect("Piste ancienne", TypeClient::Professionnel))
            .unwrap();
        let mut ancien = s.client_par_id(dormant.id).unwrap();
        ancien.cree_le = "2022-01-15".to_string();
        ancien.notes = "piste froide".to_string();
        s.modifier_client(ancien).unwrap();
        let idx = s.clients.iter().position(|c| c.id == dormant.id).unwrap();
        s.clients[idx].cree_le = "2022-01-15".to_string();
        s.clients[idx].modifie_le = "2022-01-15".to_string();

        // La fiche « vivante » est créée aujourd'hui : on fige aussi ses dates
        // de création/modification, sinon le test dépendrait du jour où il
        // tourne (le contact le plus récent serait « aujourd'hui »).
        let idx = s.clients.iter().position(|c| c.id == vivant.id).unwrap();
        s.clients[idx].cree_le = "2026-11-02".to_string();
        s.clients[idx].modifie_le = "2026-11-02".to_string();

        assert_eq!(s.dernier_contact(vivant.id), "2026-11-02");
        assert_eq!(s.dernier_contact(dormant.id), "2022-01-15");
        let perimes = s.prospects_perimes("2026-11-02");
        assert_eq!(perimes.len(), 1, "{perimes:?}");
        assert_eq!(perimes[0].0.nom, "Piste ancienne");
        assert_eq!(perimes[0].1, "2022-01-15");
        // Un client (non prospect) n'est jamais « périmé » de la même façon :
        // sa conservation est comptable (10 ans).
        s.convertir_client(dormant.id).unwrap();
        assert!(s.prospects_perimes("2026-11-02").is_empty());
    }

    #[test]
    fn notes_datees_sur_une_fiche() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let prospect = s
            .creer_client(Client::prospect("Piste Nice", TypeClient::Professionnel))
            .unwrap();
        let client = s
            .creer_client(Client::nouveau("Atelier Dupont", TypeClient::Professionnel))
            .unwrap();
        // Notes sur un prospect ET sur un client, signées par la session.
        let note = s.noter_client(prospect.id, "Appelé, rappelle en janvier").unwrap();
        assert_eq!(note.client_id, prospect.id);
        assert_eq!(note.auteur, "local");
        assert!(!note.date_note.is_empty());
        assert_eq!(note.societe_id, s.societe().unwrap().id);
        s.noter_client(prospect.id, "Devis envoyé").unwrap();
        s.noter_client(client.id, "Paye toujours en retard").unwrap();
        assert!(s.noter_client(client.id, "   ").is_err(), "note vide refusée");
        assert!(s.noter_client(9999, "inconnu").is_err());

        // Chaque fiche ne voit que ses notes, dans l'ordre de saisie.
        let notes = s.notes_client(prospect.id);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].texte, "Appelé, rappelle en janvier");
        assert_eq!(notes[1].texte, "Devis envoyé");
        assert_eq!(s.notes_client(client.id).len(), 1);
        assert_eq!(s.notes().len(), 3);

        // Suppression d'une note (erreur de saisie).
        let supprimee = s.supprimer_note(note.id).unwrap();
        assert_eq!(supprimee.id, note.id);
        assert_eq!(s.notes_client(prospect.id).len(), 1);
        assert!(s.supprimer_note(note.id).is_err(), "déjà supprimée");

        // Une note datée compte comme contact (RGPD-02 : la purge des prospects
        // se calcule sur le dernier contact, notes comprises).
        assert_eq!(
            s.dernier_contact(prospect.id),
            crate::domain::aujourdhui(),
            "la note datée d'aujourd'hui est un contact"
        );
        // L'anonymisation RGPD efface aussi les notes (texte libre).
        s.effacer_client_rgpd(client.id).unwrap();
        assert!(s.notes_client(client.id).is_empty());
        assert!(!s.notes().is_empty(), "les notes des autres fiches restent");
    }

    #[test]
    fn les_notes_survivent_a_la_reouverture() {
        let dossier = std::env::temp_dir().join(format!("erp-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        let id = {
            let mut s = Store::ouvrir(&dossier).unwrap();
            societe_complete(&mut s);
            let client = s
                .creer_client(Client::nouveau("Dupont", TypeClient::Professionnel))
                .unwrap();
            s.noter_client(client.id, "Première visite").unwrap();
            s.noter_client(client.id, "Deuxième visite").unwrap();
            client.id
        };
        let s = Store::ouvrir(&dossier).unwrap();
        let notes = s.notes_client(id);
        assert_eq!(notes.len(), 2, "les notes sont persistées comme les autres tables");
        assert_eq!(notes[1].texte, "Deuxième visite");
        assert_eq!(notes[1].auteur, "local");
        // Le fichier attendu par le modèle existe bien.
        assert!(dossier.join("tables").join("notes.tsv").exists());
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn code_de_connexion_survit_a_la_reouverture() {
        // Le code doit fonctionner d'un processus à l'autre : c'est tout l'intérêt
        // d'un code envoyé par e-mail (SEC-03). Seule son empreinte est stockée.
        let dossier = std::env::temp_dir().join(format!("erp-code-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        let code = {
            let mut s = Store::ouvrir(&dossier).unwrap();
            let mut admin = Utilisateur::nouveau("jojo", Role::Admin);
            admin.email = "jojo@exemple.fr".into();
            s.creer_utilisateur(admin).unwrap();
            s.ouvrir_code_connexion("jojo").unwrap()
        };
        assert_eq!(code.len(), 6);
        let mut s = Store::ouvrir(&dossier).unwrap();
        let utilisateur = s.utilisateur("jojo").unwrap();
        assert_ne!(utilisateur.code_connexion, code, "le code n'est pas stocké en clair");
        assert!(utilisateur.code_connexion_en_attente(crate::securite::maintenant_secondes() as i64));
        s.verifier_code_connexion("jojo", &code).unwrap();
        // Usage unique : le même code ne repasse pas.
        let erreur = s.verifier_code_connexion("jojo", &code).unwrap_err();
        assert!(erreur.contains("aucun code"), "{erreur}");
        assert!(s.utilisateur("jojo").unwrap().code_connexion.is_empty());
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn code_de_connexion_invalide_puis_invalide_apres_trop_d_essais() {
        let mut s = Store::memoire();
        let mut admin = Utilisateur::nouveau("jojo", Role::Admin);
        admin.email = "jojo@exemple.fr".into();
        s.creer_utilisateur(admin).unwrap();
        let code = s.ouvrir_code_connexion("jojo").unwrap();
        let mauvais = if code == "000000" { "000001" } else { "000000" };
        for essai in 1..=Utilisateur::CODE_CONNEXION_ESSAIS_MAX {
            let erreur = s.verifier_code_connexion("jojo", mauvais).unwrap_err();
            let restants = Utilisateur::CODE_CONNEXION_ESSAIS_MAX - essai;
            assert!(erreur.contains(&format!("{restants} tentative")), "{erreur}");
        }
        // Après le plafond, même le bon code est refusé (SEC-03).
        let erreur = s.verifier_code_connexion("jojo", &code).unwrap_err();
        assert!(erreur.contains("trop de tentatives"), "{erreur}");
        // Un nouvel envoi remet le compteur à zéro et fournit un nouveau code.
        let nouveau = s.ouvrir_code_connexion("jojo").unwrap();
        s.verifier_code_connexion("jojo", &nouveau).unwrap();
    }

    #[test]
    fn code_de_connexion_expire_et_exige_un_email() {
        let mut s = Store::memoire();
        // `creer_utilisateur` attribue l'identifiant : on garde l'objet renvoyé.
        let mut admin = s
            .creer_utilisateur(Utilisateur::nouveau("jojo", Role::Admin))
            .unwrap();
        // Sans adresse e-mail, aucun envoi possible : le message dit quoi faire.
        let erreur = s.ouvrir_code_connexion("jojo").unwrap_err();
        assert!(erreur.contains("adresse e-mail"), "{erreur}");
        admin.email = "jojo@exemple.fr".into();
        s.maj_utilisateur(admin).unwrap();
        let code = s.ouvrir_code_connexion("jojo").unwrap();
        // On force l'échéance dans le passé : le code est refusé et purgé.
        let mut utilisateur = s.utilisateur("jojo").unwrap();
        utilisateur.code_connexion_expire = 1;
        s.maj_utilisateur(utilisateur).unwrap();
        let erreur = s.verifier_code_connexion("jojo", &code).unwrap_err();
        assert!(erreur.contains("expiré"), "{erreur}");
        let utilisateur = s.utilisateur("jojo").unwrap();
        assert!(utilisateur.code_connexion.is_empty());
        assert_eq!(utilisateur.code_connexion_expire, 0);
    }

    #[test]
    fn email_modifie_avec_validation() {
        let mut s = Store::memoire();
        s.creer_utilisateur(Utilisateur::nouveau("jojo", Role::Admin))
            .unwrap();
        assert!(s.maj_email_utilisateur("jojo", "adresse-invalide").is_err());
        let utilisateur = s.maj_email_utilisateur("jojo", "jojo@exemple.fr").unwrap();
        assert_eq!(utilisateur.email, "jojo@exemple.fr");
        assert!(s.maj_email_utilisateur("inconnu", "a@b.fr").is_err());
    }

    #[test]
    fn persistance_disque_aller_retour() {
        let dossier = std::env::temp_dir().join(format!("erp-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        let (client_id, numero) = {
            let mut s = Store::ouvrir(&dossier).unwrap();
            societe_complete(&mut s);
            let c = client_test(&mut s);
            let doc = facture_test(&mut s, c.id);
            s.valider_document(&doc.numero).unwrap();
            s.ajouter_paiement(&doc.numero, 40_000, "2026-11-02", "virement", "")
                .unwrap();
            (c.id, doc.numero)
        };
        let mut s = Store::ouvrir(&dossier).unwrap();
        assert_eq!(s.clients().len(), 1);
        let doc = s.document_par_numero(&numero).unwrap();
        assert_eq!(doc.lignes.len(), 1);
        assert_eq!(doc.paye, 40_000);
        assert_eq!(doc.statut, StatutDocument::PartiellementPaye);
        assert_eq!(s.client_par_id(client_id).unwrap().nom, "Atelier Dupont");
        assert!(s.verifier_chaine_audit().is_ok());
        // Sauvegarde puis restauration.
        let sauvegarde = s.creer_sauvegarde().unwrap();
        let c = s.client_par_id(client_id).unwrap();
        s.modifier_client(Client {
            nom: "Modifié".into(),
            ..c
        })
        .unwrap();
        assert_eq!(s.client_par_id(client_id).unwrap().nom, "Modifié");
        s.restaurer_sauvegarde(&sauvegarde.date_sauvegarde).unwrap();
        assert_eq!(s.client_par_id(client_id).unwrap().nom, "Atelier Dupont");
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn recherche_globale() {
        let mut s = Store::memoire();
        societe_complete(&mut s);
        let c = client_test(&mut s);
        facture_test(&mut s, c.id);
        let mut article = Article::nouveau("Zara", "Veste en jean");
        article.composition = "100 % coton".into();
        s.creer_article(article).unwrap();
        let resultats = s.recherche_globale("dupont");
        assert_eq!(resultats.clients.len(), 1);
        let resultats = s.recherche_globale("maintenance");
        assert_eq!(resultats.documents.len(), 1);
        let resultats = s.recherche_globale("zara");
        assert_eq!(resultats.articles.len(), 1);
        assert_eq!(s.recherche_globale("").total(), 0);
    }

    #[test]
    fn dates_des_recurrences() {
        assert_eq!(ajouter_mois("2026-01-31", 1), "2026-02-28");
        assert_eq!(ajouter_mois("2026-01-15", 3), "2026-04-15");
        assert_eq!(ajouter_mois("2026-11-30", 3), "2027-02-28");
        assert_eq!(avancer_periode("2026-05-10", "trimestrielle"), "2026-08-10");
        assert_eq!(avancer_periode("2026-05-10", "annuelle"), "2027-05-10");
    }

    #[test]
    fn import_etat_v1() {
        let dossier = std::env::temp_dir().join(format!("erp-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        let etat = "SOC\tMa micro\tfranchise\tEUR\t\nMETA\t42\nCLI\t1\tAtelier Dupont\tprofessionnel\ta@b.fr\t\t1 rue X\t\t\t\nDOC\t1\tFA-2026-0001\tfacture\tvalidée\t1\t2026-10-30\t2026-11-29\tservices\t10000\nLIG\t1\tMaintenance\t2000\t50000\t2000\nPAI\t1\t1\t2026-11-02\t10000\tvirement\n";
        std::fs::write(dossier.join("etat.tsv"), etat).unwrap();
        let mut s = Store::ouvrir(&dossier).unwrap();
        assert_eq!(s.clients().len(), 1);
        let doc = s.document_par_numero("FA-2026-0001").unwrap();
        assert_eq!(doc.lignes.len(), 1);
        assert_eq!(doc.paye, 10_000);
        assert_eq!(s.tous_paiements().len(), 1);
        assert_eq!(s.societe().unwrap().nom, "Ma micro");
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
