//! Persistance fichier locale (ADR-002) : un seul fichier tabulé `etat.tsv`,
//! écriture atomique (fichier temporaire + renommage). Zéro dépendance.
//! Le passage à PostgreSQL se fera par une autre implémentation de `Store`.

use crate::domain::*;
use crate::store::*;
use std::path::{Path, PathBuf};

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

fn statut_depuis(s: &str) -> StatutDocument {
    match s {
        "envoyé" => StatutDocument::Envoye,
        "accepté" => StatutDocument::Accepte,
        "refusé" => StatutDocument::Refuse,
        "validée" => StatutDocument::Valide,
        "payée" => StatutDocument::Paye,
        "partiellement payée" => StatutDocument::PartiellementPaye,
        "annulée" => StatutDocument::Annule,
        _ => StatutDocument::Brouillon,
    }
}

pub struct Fichier {
    inner: Memoire,
    dossier: PathBuf,
}

impl Fichier {
    pub fn ouvrir(dossier: &Path) -> Resultat<Self> {
        std::fs::create_dir_all(dossier)
            .map_err(|e| format!("création du dossier de données {} : {e}", dossier.display()))?;
        let mut f = Self {
            inner: Memoire::new(),
            dossier: dossier.to_path_buf(),
        };
        let fichier_etat = dossier.join("etat.tsv");
        if fichier_etat.exists() {
            let contenu = std::fs::read_to_string(&fichier_etat)
                .map_err(|e| format!("lecture de {} : {e}", fichier_etat.display()))?;
            f.charger(&contenu)?;
        }
        Ok(f)
    }

    fn charger(&mut self, contenu: &str) -> Resultat<()> {
        let mut etat = Etat {
            societe: None,
            clients: Vec::new(),
            documents: Vec::new(),
            paiements: Vec::new(),
            articles: Vec::new(),
            ventes: Vec::new(),
            audit_log: Vec::new(),
            sequences: std::collections::HashMap::new(),
            prochain_id: 1,
        };
        for ligne in contenu.lines() {
            if ligne.is_empty() {
                continue;
            }
            let champs: Vec<String> = ligne.split('\t').map(desechapper).collect();
            let g = |i: usize| champs.get(i).cloned().unwrap_or_default();
            let gi = |i: usize| g(i).parse::<i64>().unwrap_or(0);
            match champs.first().map(|s| s.as_str()) {
                Some("SOC") => {
                    etat.societe = Some(Societe {
                        id: 1,
                        nom: g(1),
                        regime: RegimeTva::from_str(&g(2))?,
                        devise: g(3),
                        siren: g(4),
                    })
                }
                Some("SEQ") => {
                    etat.sequences.insert(g(1), g(2).parse().unwrap_or(0));
                }
                Some("META") => etat.prochain_id = gi(1).max(1),
                Some("CLI") => etat.clients.push(Client {
                    id: gi(1),
                    nom: g(2),
                    type_client: TypeClient::from_str(&g(3))?,
                    email: g(4),
                    telephone: g(5),
                    adresse: g(6),
                    siren: g(7),
                    tva_intracom: g(8),
                    notes: g(9),
                }),
                Some("DOC") => etat.documents.push(Document {
                    id: gi(1),
                    numero: g(2),
                    type_doc: TypeDocument::from_str(&g(3))?,
                    statut: statut_depuis(&g(4)),
                    client_id: gi(5),
                    date_doc: g(6),
                    echeance: g(7),
                    activite: g(8),
                    lignes: Vec::new(),
                    paye: gi(9),
                }),
                Some("LIG") => {
                    let doc_id = gi(1);
                    if let Some(d) = etat.documents.iter_mut().find(|d| d.id == doc_id) {
                        d.lignes.push(Ligne {
                            designation: g(2),
                            quantite_milli: gi(3),
                            prix_unitaire_ht: gi(4),
                            tva_bp: gi(5) as i32,
                        });
                    }
                }
                Some("PAI") => etat.paiements.push(Paiement {
                    id: gi(1),
                    document_id: gi(2),
                    date_paiement: g(3),
                    montant: gi(4),
                    moyen: g(5),
                }),
                Some("ART") => etat.articles.push(Article {
                    id: gi(1),
                    marque: g(2),
                    description: g(3),
                    taille: g(4),
                    composition: g(5),
                    etat: g(6),
                    prix_achat: gi(7),
                    date_achat: g(8),
                    statut: g(9),
                }),
                Some("VEN") => etat.ventes.push(Vente {
                    id: gi(1),
                    article_id: gi(2),
                    date_vente: g(3),
                    prix_vente: gi(4),
                    frais: gi(5),
                    plateforme: g(6),
                }),
                Some("AUD") => etat.audit_log.push((g(1), g(2), g(3))),
                _ => {}
            }
        }
        self.inner.restaurer(etat);
        Ok(())
    }

    fn sauvegarder(&self) -> Resultat<()> {
        let e = self.inner.snapshot();
        let mut out = String::new();
        if let Some(s) = &e.societe {
            out += &format!(
                "SOC\t{}\t{}\t{}\t{}\n",
                echapper(&s.nom),
                s.regime.as_str(),
                echapper(&s.devise),
                echapper(&s.siren)
            );
        }
        for (k, v) in &e.sequences {
            out += &format!("SEQ\t{}\t{}\n", echapper(k), v);
        }
        out += &format!("META\t{}\n", e.prochain_id);
        for c in &e.clients {
            out += &format!(
                "CLI\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                c.id,
                echapper(&c.nom),
                c.type_client.as_str(),
                echapper(&c.email),
                echapper(&c.telephone),
                echapper(&c.adresse),
                echapper(&c.siren),
                echapper(&c.tva_intracom),
                echapper(&c.notes)
            );
        }
        for d in &e.documents {
            out += &format!(
                "DOC\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                d.id,
                echapper(&d.numero),
                d.type_doc.as_str(),
                d.statut.as_str(),
                d.client_id,
                echapper(&d.date_doc),
                echapper(&d.echeance),
                echapper(&d.activite),
                d.paye
            );
            for l in &d.lignes {
                out += &format!(
                    "LIG\t{}\t{}\t{}\t{}\t{}\n",
                    d.id,
                    echapper(&l.designation),
                    l.quantite_milli,
                    l.prix_unitaire_ht,
                    l.tva_bp
                );
            }
        }
        for pa in &e.paiements {
            out += &format!(
                "PAI\t{}\t{}\t{}\t{}\t{}\n",
                pa.id,
                pa.document_id,
                echapper(&pa.date_paiement),
                pa.montant,
                echapper(&pa.moyen)
            );
        }
        for a in &e.articles {
            out += &format!(
                "ART\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                a.id,
                echapper(&a.marque),
                echapper(&a.description),
                echapper(&a.taille),
                echapper(&a.composition),
                echapper(&a.etat),
                a.prix_achat,
                echapper(&a.date_achat),
                echapper(&a.statut)
            );
        }
        for v in &e.ventes {
            out += &format!(
                "VEN\t{}\t{}\t{}\t{}\t{}\t{}\n",
                v.id,
                v.article_id,
                echapper(&v.date_vente),
                v.prix_vente,
                v.frais,
                echapper(&v.plateforme)
            );
        }
        for (d, a, detail) in &e.audit_log {
            out += &format!("AUD\t{}\t{}\t{}\n", echapper(d), echapper(a), echapper(detail));
        }
        let cible = self.dossier.join("etat.tsv");
        let tmp = self.dossier.join("etat.tsv.tmp");
        std::fs::write(&tmp, out).map_err(|e| format!("écriture des données : {e}"))?;
        std::fs::rename(&tmp, &cible).map_err(|e| format!("écriture des données : {e}"))?;
        Ok(())
    }

    fn apres_mutation<T>(&self, r: Resultat<T>) -> Resultat<T> {
        if r.is_ok() {
            self.sauvegarder()?;
        }
        r
    }
}

impl Store for Fichier {
    fn societe(&mut self) -> Resultat<Societe> {
        let creee = self.inner.societe.is_none();
        let s = self.inner.societe()?;
        if creee {
            self.sauvegarder()?;
        }
        Ok(s)
    }
    fn set_regime_tva(&mut self, regime: RegimeTva) -> Resultat<()> {
        let r = self.inner.set_regime_tva(regime);
        self.apres_mutation(r)
    }
    fn creer_client(&mut self, c: Client) -> Resultat<Client> {
        let r = self.inner.creer_client(c);
        self.apres_mutation(r)
    }
    fn clients(&mut self) -> Resultat<Vec<Client>> {
        self.inner.clients()
    }
    fn client_par_id(&mut self, id: i64) -> Resultat<Client> {
        self.inner.client_par_id(id)
    }
    fn creer_document(&mut self, t: TypeDocument, client_id: i64, date_doc: &str, echeance: &str, activite: &str, lignes: Vec<Ligne>) -> Resultat<Document> {
        let r = self.inner.creer_document(t, client_id, date_doc, echeance, activite, lignes);
        self.apres_mutation(r)
    }
    fn documents(&mut self, t: Option<TypeDocument>) -> Resultat<Vec<Document>> {
        self.inner.documents(t)
    }
    fn document_par_numero(&mut self, numero: &str) -> Resultat<Document> {
        self.inner.document_par_numero(numero)
    }
    fn changer_statut(&mut self, numero: &str, statut: StatutDocument) -> Resultat<Document> {
        let r = self.inner.changer_statut(numero, statut);
        self.apres_mutation(r)
    }
    fn ajouter_paiement(&mut self, numero: &str, montant: Cents, date: &str, moyen: &str) -> Resultat<Document> {
        let r = self.inner.ajouter_paiement(numero, montant, date, moyen);
        self.apres_mutation(r)
    }
    fn paiements(&mut self, document_id: i64) -> Resultat<Vec<Paiement>> {
        self.inner.paiements(document_id)
    }
    fn creer_article(&mut self, a: Article) -> Resultat<Article> {
        let r = self.inner.creer_article(a);
        self.apres_mutation(r)
    }
    fn articles(&mut self) -> Resultat<Vec<Article>> {
        self.inner.articles()
    }
    fn article_par_id(&mut self, id: i64) -> Resultat<Article> {
        self.inner.article_par_id(id)
    }
    fn enregistrer_vente(&mut self, v: Vente) -> Resultat<Vente> {
        let r = self.inner.enregistrer_vente(v);
        self.apres_mutation(r)
    }
    fn ventes(&mut self) -> Resultat<Vec<Vente>> {
        self.inner.ventes()
    }
    fn journaliser(&mut self, action: &str, detail: &str) -> Resultat<()> {
        let r = self.inner.journaliser(action, detail);
        self.apres_mutation(r)
    }
    fn audit(&mut self, limite: usize) -> Resultat<Vec<(String, String, String)>> {
        self.inner.audit(limite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aller_retour_fichier() {
        let dossier = std::env::temp_dir().join(format!("erp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        {
            let mut f = Fichier::ouvrir(&dossier).unwrap();
            let c = f
                .creer_client(Client {
                    id: 0,
                    nom: "Atelier Dupont".into(),
                    type_client: TypeClient::Professionnel,
                    email: "a@b.fr".into(),
                    telephone: String::new(),
                    adresse: "1 rue des Tests".into(),
                    siren: String::new(),
                    tva_intracom: String::new(),
                    notes: String::new(),
                })
                .unwrap();
            f.creer_document(
                TypeDocument::Facture,
                c.id,
                "2026-10-30",
                "2026-11-29",
                "services",
                vec![Ligne::new("Maintenance", 1.0, 10_000, 2000)],
            )
            .unwrap();
        }
        let mut f2 = Fichier::ouvrir(&dossier).unwrap();
        assert_eq!(f2.clients().unwrap().len(), 1);
        let doc = f2.document_par_numero("FA-2026-0001").unwrap();
        assert_eq!(doc.lignes.len(), 1);
        assert_eq!(doc.lignes[0].total_ht(), 10_000);
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn echappement() {
        assert_eq!(desechapper(&echapper("a\tb\n100 %")), "a\tb\n100 %");
    }
}
