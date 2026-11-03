//! Comptabilité dérivée automatiquement du métier (CDC §7, module C).
//!
//! Rien n'est saisi deux fois : l'utilisateur valide une facture, encaisse un
//! paiement, enregistre une vente Vinted — les écritures en découlent
//! (EF-CPT-01), avec les journaux VE (ventes), BQ (banque) et les comptes du
//! plan comptable français. `fec()` produit le format DGFiP (EF-CPT-03),
//! `balance()` et `recap_tva_*` alimentent la déclaration (EF-CPT-02).
//!
//! Toutes les fonctions sont **pures** : elles prennent l'état en entrée et
//! rendent des écritures. Aucune entrée/sortie, aucun accès au disque.

use crate::domain::*;
use crate::store::{Ecriture, Paiement};

// ------------------------------------------------------------ Plan comptable

pub const CLIENTS: &str = "411000";
pub const FOURNISSEURS: &str = "401000";
pub const BANQUE: &str = "512000";
pub const PRESTATIONS: &str = "706000";
pub const VENTES_MARCHANDISES: &str = "707000";
pub const VENTES_BIENS_PERSONNELS: &str = "708500";
pub const TVA_COLLECTEE: &str = "445710";
pub const TVA_DEDUCTIBLE: &str = "445660";
pub const ACHATS_MARCHANDISES: &str = "607000";
pub const STOCK_MARCHANDISES: &str = "370000";
pub const FRAIS_PLATEFORME: &str = "627000";

/// Libellé officiel d'un compte du plan comptable utilisé par l'ERP.
pub fn libelle_compte(compte: &str) -> &'static str {
    match compte {
        CLIENTS => "Clients",
        FOURNISSEURS => "Fournisseurs",
        BANQUE => "Banque",
        PRESTATIONS => "Prestations de services",
        VENTES_MARCHANDISES => "Ventes de marchandises",
        VENTES_BIENS_PERSONNELS => "Ventes de biens personnels",
        TVA_COLLECTEE => "TVA collectée",
        TVA_DEDUCTIBLE => "TVA déductible",
        ACHATS_MARCHANDISES => "Achats de marchandises",
        STOCK_MARCHANDISES => "Stock de marchandises",
        FRAIS_PLATEFORME => "Frais de plateforme et de port",
        _ => "Compte divers",
    }
}

pub fn libelle_journal(code: &str) -> &'static str {
    match code {
        "VE" => "Journal des ventes",
        "BQ" => "Journal de banque",
        "AC" => "Journal des achats",
        _ => "Opérations diverses",
    }
}

// ------------------------------------------------------------ Écritures

struct Compteur {
    suivant: std::collections::HashMap<String, i64>,
}

impl Compteur {
    fn nouveau() -> Self {
        Self {
            suivant: std::collections::HashMap::new(),
        }
    }
    fn num(&mut self, journal: &str) -> i64 {
        let entree = self.suivant.entry(journal.to_string()).or_insert(0);
        *entree += 1;
        *entree
    }
}

/// Génère toutes les écritures depuis les documents, paiements et ventes.
pub fn ecritures_depuis(
    societe: &Societe,
    clients: &[Client],
    documents: &[Document],
    paiements: &[Paiement],
    ventes: &[Vente],
    articles: &[Article],
) -> Vec<Ecriture> {
    let mut compteur = Compteur::nouveau();
    let mut ecritures: Vec<Ecriture> = Vec::new();
    let regime = societe.regime;

    let mut documents_tries: Vec<&Document> = documents
        .iter()
        .filter(|d| {
            matches!(d.type_doc, TypeDocument::Facture | TypeDocument::Avoir)
                && matches!(
                    d.statut,
                    StatutDocument::Valide
                        | StatutDocument::Paye
                        | StatutDocument::PartiellementPaye
                )
        })
        .collect();
    documents_tries.sort_by(|a, b| a.date_doc.cmp(&b.date_doc).then(a.numero.cmp(&b.numero)));

    for document in documents_tries {
        let client = clients.iter().find(|c| c.id == document.client_id);
        let nom_client = client.map(|c| c.nom.clone()).unwrap_or_default();
        let num_client = client
            .map(|c| c.id.to_string())
            .unwrap_or_else(|| document.client_id.to_string());
        let totaux = document.totaux(regime);
        let avoir = document.type_doc == TypeDocument::Avoir;
        let numero = compteur.num("VE");
        let libelle = format!(
            "{} {} — {}",
            if avoir { "Avoir" } else { "Facture" },
            document.numero,
            nom_client
        );
        let compte_produit = if document.activite == "ventes" {
            VENTES_MARCHANDISES
        } else {
            PRESTATIONS
        };

        let mut pousser = |compte: &str,
                           debit: Cents,
                           credit: Cents,
                           libelle: &str,
                           document_id: i64,
                           paiement_id: i64| {
            ecritures.push(Ecriture {
                id: 0,
                societe_id: societe.id,
                journal: "VE".to_string(),
                ecriture_num: numero,
                date_ecriture: document.date_doc.clone(),
                compte: compte.to_string(),
                libelle: libelle.to_string(),
                debit,
                credit,
                piece_ref: document.numero.clone(),
                piece_date: document.date_doc.clone(),
                comp_aux_num: num_client.clone(),
                comp_aux_lib: nom_client.clone(),
                devise: document.devise.clone(),
                lettrage: String::new(),
                document_id,
                paiement_id,
            });
        };

        if avoir {
            pousser(CLIENTS, 0, totaux.ttc.abs(), &libelle, document.id, 0);
            pousser(compte_produit, totaux.ht.abs(), 0, &libelle, document.id, 0);
            if totaux.tva != 0 {
                pousser(TVA_COLLECTEE, totaux.tva.abs(), 0, "TVA sur avoir", document.id, 0);
            }
        } else {
            pousser(CLIENTS, totaux.ttc, 0, &libelle, document.id, 0);
            pousser(compte_produit, 0, totaux.ht, &libelle, document.id, 0);
            if totaux.tva != 0 {
                pousser(TVA_COLLECTEE, 0, totaux.tva, "TVA collectée", document.id, 0);
            }
        }
    }

    let mut paiements_tries: Vec<&Paiement> = paiements.iter().collect();
    paiements_tries.sort_by(|a, b| a.date_paiement.cmp(&b.date_paiement));
    for paiement in paiements_tries {
        let document = match documents.iter().find(|d| d.id == paiement.document_id) {
            Some(d) => d,
            None => continue,
        };
        if document.societe_id != societe.id {
            continue;
        }
        let client = clients.iter().find(|c| c.id == document.client_id);
        let numero = compteur.num("BQ");
        let lettrage = if paiement.reference.is_empty() {
            format!("L{:06}", paiement.id % 1_000_000)
        } else {
            paiement.reference.clone()
        };
        let libelle = format!("Encaissement {}", document.numero);
        for (compte, debit, credit) in [
            (BANQUE, paiement.montant, 0),
            (CLIENTS, 0, paiement.montant),
        ] {
            ecritures.push(Ecriture {
                id: 0,
                societe_id: societe.id,
                journal: "BQ".to_string(),
                ecriture_num: numero,
                date_ecriture: paiement.date_paiement.clone(),
                compte: compte.to_string(),
                libelle: libelle.clone(),
                debit,
                credit,
                piece_ref: document.numero.clone(),
                piece_date: paiement.date_paiement.clone(),
                comp_aux_num: client.map(|c| c.id.to_string()).unwrap_or_default(),
                comp_aux_lib: client.map(|c| c.nom.clone()).unwrap_or_default(),
                devise: document.devise.clone(),
                lettrage: lettrage.clone(),
                document_id: document.id,
                paiement_id: paiement.id,
            });
        }
    }

    let mut ventes_triees: Vec<&Vente> = ventes
        .iter()
        .filter(|v| v.societe_id == societe.id && v.regime == RegimeVente::AchatRevente)
        .collect();
    ventes_triees.sort_by(|a, b| a.date_vente.cmp(&b.date_vente));
    for vente in ventes_triees {
        let article = articles.iter().find(|a| a.id == vente.article_id);
        let numero = compteur.num("VE");
        let libelle = format!(
            "Vente {} sur {}",
            article.map(|a| a.description.clone()).unwrap_or_default(),
            vente.plateforme
        );
        let net = (vente.prix_vente - vente.frais).max(0);
        let mut pousser = |compte: &str, debit: Cents, credit: Cents, detail: &str| {
            ecritures.push(Ecriture {
                id: 0,
                societe_id: societe.id,
                journal: "VE".to_string(),
                ecriture_num: numero,
                date_ecriture: vente.date_vente.clone(),
                compte: compte.to_string(),
                libelle: format!("{libelle} — {detail}"),
                debit,
                credit,
                piece_ref: vente.reference.clone(),
                piece_date: vente.date_vente.clone(),
                comp_aux_num: String::new(),
                comp_aux_lib: String::new(),
                devise: DEVISE_REFERENCE.to_string(),
                lettrage: String::new(),
                document_id: 0,
                paiement_id: 0,
            });
        };
        pousser(BANQUE, net, 0, "encaissement plateforme");
        if vente.frais > 0 {
            pousser(FRAIS_PLATEFORME, vente.frais, 0, "frais de plateforme");
        }
        pousser(VENTES_MARCHANDISES, 0, vente.prix_vente, "prix de vente");
        if let Some(article) = article {
            if article.prix_achat > 0 {
                pousser(ACHATS_MARCHANDISES, article.prix_achat, 0, "coût d'achat");
                pousser(STOCK_MARCHANDISES, 0, article.prix_achat, "sortie de stock");
            }
        }
    }

    ecritures
}

// ------------------------------------------------------------ FEC (EF-CPT-03)

/// En-tête normalisée du FEC (18 colonnes, art. A.47 AA-1 du LPF).
pub const ENTETE_FEC: &str = "JournalCode|JournalLib|EcritureNum|EcritureDate|CompteNum|CompteLib|CompAuxNum|CompAuxLib|PieceRef|PieceDate|EcritureLib|Debit|Credit|EcritureLet|DateLet|ValidDate|Montantdevise|Idevise";

fn montant_fec(cents: Cents) -> String {
    let signe = if cents < 0 { "-" } else { "" };
    let abs = cents.abs();
    format!("{signe}{},{:02}", abs / 100, abs % 100)
}

fn date_fec(date: &str) -> String {
    date.replace('-', "")
}

/// Lignes du FEC, triées par journal, date puis numéro d'écriture.
pub fn fec(ecritures: &[Ecriture]) -> Vec<String> {
    let mut triees: Vec<Ecriture> = ecritures.to_vec();
    triees.sort_by(|a, b| {
        a.journal
            .cmp(&b.journal)
            .then(a.date_ecriture.cmp(&b.date_ecriture))
            .then(a.ecriture_num.cmp(&b.ecriture_num))
            .then(a.compte.cmp(&b.compte))
    });
    let mut lignes = vec![ENTETE_FEC.to_string()];
    for e in &triees {
        lignes.push(format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|||{}|{}",
            e.journal,
            libelle_journal(&e.journal),
            e.ecriture_num,
            date_fec(&e.date_ecriture),
            e.compte,
            libelle_compte(&e.compte),
            e.comp_aux_num,
            e.comp_aux_lib,
            e.piece_ref,
            date_fec(&e.piece_date),
            e.libelle.replace('|', "/"),
            montant_fec(e.debit),
            montant_fec(e.credit),
            e.lettrage,
            montant_fec(e.debit + e.credit),
            e.devise
        ));
    }
    lignes
}

// ------------------------------------------------------------ TVA (EF-CPT-02)

/// TVA par taux, sur un ensemble de documents et une période (`AAAA-MM` ou `AAAA`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecapTva {
    pub periode: String,
    pub taux_bp: i32,
    pub base_ht: Cents,
    pub tva: Cents,
}

pub fn recap_tva_par_taux(
    documents: &[Document],
    regime: RegimeTva,
    periode: &str,
) -> Vec<RecapTva> {
    let mut recap: Vec<RecapTva> = Vec::new();
    for document in documents {
        if !matches!(document.type_doc, TypeDocument::Facture | TypeDocument::Avoir) {
            continue;
        }
        if !matches!(
            document.statut,
            StatutDocument::Valide
                | StatutDocument::Paye
                | StatutDocument::PartiellementPaye
        ) {
            continue;
        }
        if !periode.is_empty() && !document.date_doc.starts_with(periode) {
            continue;
        }
        let signe: Cents = if document.type_doc == TypeDocument::Avoir { -1 } else { 1 };
        for ventilation in tva_par_taux(&document.lignes, regime) {
            let entree = match recap.iter_mut().find(|r| r.taux_bp == ventilation.taux_bp) {
                Some(e) => e,
                None => {
                    recap.push(RecapTva {
                        periode: periode.to_string(),
                        taux_bp: ventilation.taux_bp,
                        base_ht: 0,
                        tva: 0,
                    });
                    recap.last_mut().unwrap()
                }
            };
            entree.base_ht += signe * ventilation.base_ht;
            entree.tva += signe * ventilation.tva;
        }
    }
    recap.sort_by_key(|r| r.taux_bp);
    recap
}

// ------------------------------------------------------------ Balance & journaux

/// Balance par compte : (compte, total débit, total crédit, solde).
pub fn balance(ecritures: &[Ecriture]) -> Vec<(String, Cents, Cents, Cents)> {
    let mut lignes: Vec<(String, Cents, Cents, Cents)> = Vec::new();
    for e in ecritures {
        let entree = match lignes.iter_mut().find(|l| l.0 == e.compte) {
            Some(l) => l,
            None => {
                lignes.push((e.compte.clone(), 0, 0, 0));
                lignes.last_mut().unwrap()
            }
        };
        entree.1 += e.debit;
        entree.2 += e.credit;
        entree.3 += e.debit - e.credit;
    }
    lignes.sort_by(|a, b| a.0.cmp(&b.0));
    lignes
}

/// Écritures d'un journal, triées chronologiquement.
pub fn journal(ecritures: &[Ecriture], code: &str) -> Vec<Ecriture> {
    let mut selection: Vec<Ecriture> = ecritures
        .iter()
        .filter(|e| e.journal.eq_ignore_ascii_case(code))
        .cloned()
        .collect();
    selection.sort_by(|a, b| {
        a.date_ecriture
            .cmp(&b.date_ecriture)
            .then(a.ecriture_num.cmp(&b.ecriture_num))
    });
    selection
}

/// Journaux présents, avec leur libellé et leur nombre d'écritures.
pub fn journaux(ecritures: &[Ecriture]) -> Vec<(String, &'static str, usize)> {
    let mut liste: Vec<(String, &'static str, usize)> = Vec::new();
    for e in ecritures {
        match liste.iter_mut().find(|l| l.0 == e.journal) {
            Some(l) => l.2 += 1,
            None => liste.push((e.journal.clone(), libelle_journal(&e.journal), 1)),
        }
    }
    liste.sort_by(|a, b| a.0.cmp(&b.0));
    liste
}

/// Vérifie l'équilibre débit/crédit d'un jeu d'écritures.
pub fn est_equilibre(ecritures: &[Ecriture]) -> bool {
    let debit: Cents = ecritures.iter().map(|e| e.debit).sum();
    let credit: Cents = ecritures.iter().map(|e| e.credit).sum();
    debit == credit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn societe(regime: RegimeTva) -> Societe {
        Societe {
            id: 1,
            nom: "Dupont Conseil".into(),
            siren: "123456789".into(),
            regime,
            ..Societe::default()
        }
    }

    fn client() -> Client {
        let mut c = Client::nouveau("Atelier Dupont", TypeClient::Professionnel);
        c.id = 1;
        c.societe_id = 1;
        c
    }

    fn facture(statut: StatutDocument) -> Document {
        Document {
            id: 1,
            societe_id: 1,
            numero: "FA-2026-0001".into(),
            type_doc: TypeDocument::Facture,
            statut,
            client_id: 1,
            date_doc: "2026-10-30".into(),
            date_prestation: "2026-10-30".into(),
            echeance: "2026-11-29".into(),
            lignes: vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)],
            paye: 0,
            ..Document::default()
        }
    }

    #[test]
    fn ecritures_de_vente_tva_reelle() {
        let mut doc = facture(StatutDocument::Valide);
        doc.paye = 0;
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Reel),
            &[client()],
            &[doc],
            &[],
            &[],
            &[],
        );
        assert_eq!(ecritures.len(), 3);
        assert!(est_equilibre(&ecritures));
        let clients: Cents = ecritures.iter().filter(|e| e.compte == CLIENTS).map(|e| e.debit).sum();
        assert_eq!(clients, 120_000);
        let tva: Cents = ecritures.iter().filter(|e| e.compte == TVA_COLLECTEE).map(|e| e.credit).sum();
        assert_eq!(tva, 20_000);
        assert_eq!(ecritures[0].journal, "VE");
    }

    #[test]
    fn ecritures_en_franchise_sans_tva() {
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Franchise),
            &[client()],
            &[facture(StatutDocument::Valide)],
            &[],
            &[],
            &[],
        );
        assert_eq!(ecritures.len(), 2);
        assert!(ecritures.iter().all(|e| e.compte != TVA_COLLECTEE));
        assert!(est_equilibre(&ecritures));
    }

    #[test]
    fn ecritures_ignorent_les_brouillons() {
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Franchise),
            &[client()],
            &[facture(StatutDocument::Brouillon)],
            &[],
            &[],
            &[],
        );
        assert!(ecritures.is_empty());
    }

    #[test]
    fn ecritures_de_paiement_et_fec() {
        let mut doc = facture(StatutDocument::Paye);
        doc.paye = 100_000;
        let paiement = Paiement {
            id: 9,
            document_id: 1,
            date_paiement: "2026-11-02".into(),
            montant: 100_000,
            moyen: "virement".into(),
            reference: String::new(),
            rapproche: false,
        };
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Franchise),
            &[client()],
            &[doc],
            &[paiement],
            &[],
            &[],
        );
        assert!(est_equilibre(&ecritures));
        let lignes = fec(&ecritures);
        assert_eq!(lignes[0], ENTETE_FEC);
        assert_eq!(lignes[0].split('|').count(), 18);
        assert!(lignes.iter().any(|l| l.starts_with("BQ|Journal de banque")));
        assert!(lignes.iter().any(|l| l.contains("20261102")));
        assert!(lignes.iter().any(|l| l.contains("L000009")));
    }

    #[test]
    fn avoir_inverse_les_ecritures() {
        let mut avoir = facture(StatutDocument::Valide);
        avoir.numero = "AV-2026-0001".into();
        avoir.type_doc = TypeDocument::Avoir;
        avoir.lignes[0].prix_unitaire_ht = -50_000;
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Reel),
            &[client()],
            &[avoir],
            &[],
            &[],
            &[],
        );
        assert!(est_equilibre(&ecritures));
        let clients_credit: Cents = ecritures
            .iter()
            .filter(|e| e.compte == CLIENTS)
            .map(|e| e.credit)
            .sum();
        assert_eq!(clients_credit, 120_000);
    }

    #[test]
    fn recap_tva_par_taux_et_periode() {
        let doc = facture(StatutDocument::Valide);
        let recap = recap_tva_par_taux(&[doc.clone()], RegimeTva::Reel, "2026-10");
        assert_eq!(recap.len(), 1);
        assert_eq!(recap[0].taux_bp, 2000);
        assert_eq!(recap[0].base_ht, 100_000);
        assert_eq!(recap[0].tva, 20_000);
        assert!(recap_tva_par_taux(&[doc], RegimeTva::Reel, "2026-09").is_empty());
    }

    #[test]
    fn ventes_vinted_en_comptabilite() {
        let mut article = Article::nouveau("Zara", "Veste");
        article.id = 3;
        article.societe_id = 1;
        article.prix_achat = 800;
        article.composition = "100 % coton".into();
        let vente = Vente {
            id: 1,
            societe_id: 1,
            article_id: 3,
            date_vente: "2026-10-20".into(),
            prix_vente: 2_500,
            frais: 200,
            reference: "REF-1".into(),
            ..Vente::default()
        };
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Franchise),
            &[],
            &[],
            &[],
            &[vente],
            &[article],
        );
        assert!(est_equilibre(&ecritures));
        assert!(ecritures.iter().any(|e| e.compte == FRAIS_PLATEFORME));
        assert!(ecritures.iter().any(|e| e.compte == ACHATS_MARCHANDISES));
    }

    #[test]
    fn balance_et_journaux() {
        let ecritures = ecritures_depuis(
            &societe(RegimeTva::Franchise),
            &[client()],
            &[facture(StatutDocument::Valide)],
            &[],
            &[],
            &[],
        );
        let balance = balance(&ecritures);
        assert_eq!(balance.len(), 2);
        let clients = balance.iter().find(|l| l.0 == CLIENTS).unwrap();
        assert_eq!(clients.3, 100_000); // solde débiteur
        let liste = journaux(&ecritures);
        assert_eq!(liste[0].0, "VE");
        assert_eq!(journal(&ecritures, "VE").len(), 2);
        assert!(journal(&ecritures, "BQ").is_empty());
    }
}
