//! Persistance : un trait `Store`, deux implémentations.
//! - `Memoire` : en mémoire (tests, démonstration `--memoire`)
//! - `PgStore` (module `pg`) : PostgreSQL, la cible de production du CDC.

use crate::domain::*;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Societe {
    pub id: i64,
    pub nom: String,
    pub regime: RegimeTva,
    pub devise: String,
    pub siren: String,
}

impl Default for Societe {
    fn default() -> Self {
        Self {
            id: 1,
            nom: "Ma micro-entreprise".to_string(),
            regime: RegimeTva::Franchise,
            devise: "EUR".to_string(),
            siren: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: i64,
    pub numero: String,
    pub type_doc: TypeDocument,
    pub statut: StatutDocument,
    pub client_id: i64,
    pub date_doc: String,
    pub echeance: String,
    pub activite: String, // "services" | "ventes"
    pub lignes: Vec<Ligne>,
    pub paye: Cents,
}

impl Document {
    pub fn totaux(&self, regime: RegimeTva) -> Totaux {
        totaux(&self.lignes, regime)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paiement {
    pub id: i64,
    pub document_id: i64,
    pub date_paiement: String,
    pub montant: Cents,
    pub moyen: String,
}

pub type Resultat<T> = Result<T, String>;

pub trait Store {
    fn societe(&mut self) -> Resultat<Societe>;
    fn set_regime_tva(&mut self, regime: RegimeTva) -> Resultat<()>;

    fn creer_client(&mut self, c: Client) -> Resultat<Client>;
    fn clients(&mut self) -> Resultat<Vec<Client>>;
    fn chercher_clients(&mut self, recherche: &str) -> Resultat<Vec<Client>> {
        Ok(self
            .clients()?
            .into_iter()
            .filter(|c| correspond(&c.nom, recherche) || correspond(&c.email, recherche))
            .collect())
    }
    fn client_par_id(&mut self, id: i64) -> Resultat<Client>;

    fn creer_document(
        &mut self,
        type_doc: TypeDocument,
        client_id: i64,
        date_doc: &str,
        echeance: &str,
        activite: &str,
        lignes: Vec<Ligne>,
    ) -> Resultat<Document>;
    fn documents(&mut self, type_doc: Option<TypeDocument>) -> Resultat<Vec<Document>>;
    fn document_par_numero(&mut self, numero: &str) -> Resultat<Document>;
    fn changer_statut(&mut self, numero: &str, statut: StatutDocument) -> Resultat<Document>;
    fn ajouter_paiement(
        &mut self,
        numero: &str,
        montant: Cents,
        date_paiement: &str,
        moyen: &str,
    ) -> Resultat<Document>;
    fn paiements(&mut self, document_id: i64) -> Resultat<Vec<Paiement>>;

    fn creer_article(&mut self, a: Article) -> Resultat<Article>;
    fn articles(&mut self) -> Resultat<Vec<Article>>;
    fn article_par_id(&mut self, id: i64) -> Resultat<Article>;
    fn enregistrer_vente(&mut self, v: Vente) -> Resultat<Vente>;
    fn ventes(&mut self) -> Resultat<Vec<Vente>>;

    /// CA de l'année, séparé ventes / services (micro mixte — CDC §21.1).
    fn ca_annuel(&mut self, annee: u32) -> Resultat<(Cents, Cents)> {
        let mut ventes_ca: Cents = 0;
        let mut services_ca: Cents = 0;
        let regime = self.societe()?.regime;
        for d in self.documents(None)? {
            if annee_de(&d.date_doc) != annee || d.statut == StatutDocument::Annule {
                continue;
            }
            let t = d.totaux(regime).ttc;
            if d.activite == "ventes" {
                ventes_ca += t;
            } else {
                services_ca += t;
            }
        }
        for v in self.ventes()? {
            if annee_de(&v.date_vente) == annee {
                ventes_ca += v.prix_vente;
            }
        }
        Ok((ventes_ca, services_ca))
    }

    fn journaliser(&mut self, action: &str, detail: &str) -> Resultat<()>;
    fn audit(&mut self, limite: usize) -> Resultat<Vec<(String, String, String)>>;
}

// ------------------------------------------------------------ Implémentation mémoire

#[derive(Default)]
pub struct Memoire {
    societe: Option<Societe>,
    clients: Vec<Client>,
    documents: Vec<Document>,
    paiements: Vec<Paiement>,
    articles: Vec<Article>,
    ventes: Vec<Vente>,
    audit_log: Vec<(String, String, String)>,
    sequences: HashMap<String, u32>,
    prochain_id: i64,
}

impl Memoire {
    pub fn new() -> Self {
        Self {
            prochain_id: 1,
            ..Self::default()
        }
    }
    fn id(&mut self) -> i64 {
        let i = self.prochain_id;
        self.prochain_id += 1;
        i
    }
}

impl Store for Memoire {
    fn societe(&mut self) -> Resultat<Societe> {
        if self.societe.is_none() {
            self.societe = Some(Societe::default());
        }
        Ok(self.societe.clone().unwrap())
    }

    fn set_regime_tva(&mut self, regime: RegimeTva) -> Resultat<()> {
        let mut s = self.societe()?;
        s.regime = regime;
        self.societe = Some(s);
        Ok(())
    }

    fn creer_client(&mut self, mut c: Client) -> Resultat<Client> {
        if c.nom.trim().is_empty() {
            return Err("le nom du client est obligatoire".into());
        }
        c.id = self.id();
        self.clients.push(c.clone());
        Ok(c)
    }

    fn clients(&mut self) -> Resultat<Vec<Client>> {
        Ok(self.clients.clone())
    }

    fn client_par_id(&mut self, id: i64) -> Resultat<Client> {
        self.clients
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| format!("client n°{id} introuvable"))
    }

    fn creer_document(
        &mut self,
        type_doc: TypeDocument,
        client_id: i64,
        date_doc: &str,
        echeance: &str,
        activite: &str,
        lignes: Vec<Ligne>,
    ) -> Resultat<Document> {
        self.client_par_id(client_id)?;
        if lignes.is_empty() {
            return Err("un document sans ligne n'a pas de sens".into());
        }
        let annee = annee_de(date_doc);
        let cle = format!("{}-{}", type_doc.as_str(), annee);
        let seq = {
            let e = self.sequences.entry(cle).or_insert(0);
            *e += 1;
            *e
        };
        let doc = Document {
            id: self.id(),
            numero: numero_document(type_doc, annee, seq),
            type_doc,
            statut: StatutDocument::Brouillon,
            client_id,
            date_doc: date_doc.to_string(),
            echeance: echeance.to_string(),
            activite: activite.to_string(),
            lignes,
            paye: 0,
        };
        self.documents.push(doc.clone());
        Ok(doc)
    }

    fn documents(&mut self, type_doc: Option<TypeDocument>) -> Resultat<Vec<Document>> {
        Ok(self
            .documents
            .iter()
            .filter(|d| type_doc.map(|t| d.type_doc == t).unwrap_or(true))
            .cloned()
            .collect())
    }

    fn document_par_numero(&mut self, numero: &str) -> Resultat<Document> {
        self.documents
            .iter()
            .find(|d| d.numero.eq_ignore_ascii_case(numero))
            .cloned()
            .ok_or_else(|| format!("document « {numero} » introuvable"))
    }

    fn changer_statut(&mut self, numero: &str, statut: StatutDocument) -> Resultat<Document> {
        let idx = self
            .documents
            .iter()
            .position(|d| d.numero.eq_ignore_ascii_case(numero))
            .ok_or_else(|| format!("document « {numero} » introuvable"))?;
        self.documents[idx].statut = statut;
        Ok(self.documents[idx].clone())
    }

    fn ajouter_paiement(
        &mut self,
        numero: &str,
        montant: Cents,
        date_paiement: &str,
        moyen: &str,
    ) -> Resultat<Document> {
        let regime = self.societe()?.regime;
        let idx = self
            .documents
            .iter()
            .position(|d| d.numero.eq_ignore_ascii_case(numero))
            .ok_or_else(|| format!("document « {numero} » introuvable"))?;
        let doc_id = self.documents[idx].id;
        let p = Paiement {
            id: self.id(),
            document_id: doc_id,
            date_paiement: date_paiement.to_string(),
            montant,
            moyen: moyen.to_string(),
        };
        self.paiements.push(p);
        self.documents[idx].paye += montant;
        let total = self.documents[idx].totaux(regime).ttc;
        let paye = self.documents[idx].paye;
        self.documents[idx].statut = statut_apres_paiement(total, paye);
        Ok(self.documents[idx].clone())
    }

    fn paiements(&mut self, document_id: i64) -> Resultat<Vec<Paiement>> {
        Ok(self
            .paiements
            .iter()
            .filter(|p| p.document_id == document_id)
            .cloned()
            .collect())
    }

    fn creer_article(&mut self, mut a: Article) -> Resultat<Article> {
        if a.composition.trim().is_empty() {
            return Err("composition textile obligatoire (règlement UE 1007/2011)".into());
        }
        a.id = self.id();
        if a.statut.is_empty() {
            a.statut = "en_stock".to_string();
        }
        self.articles.push(a.clone());
        Ok(a)
    }

    fn articles(&mut self) -> Resultat<Vec<Article>> {
        Ok(self.articles.clone())
    }

    fn article_par_id(&mut self, id: i64) -> Resultat<Article> {
        self.articles
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| format!("article n°{id} introuvable"))
    }

    fn enregistrer_vente(&mut self, mut v: Vente) -> Resultat<Vente> {
        let idx = self
            .articles
            .iter()
            .position(|a| a.id == v.article_id)
            .ok_or_else(|| format!("article n°{} introuvable", v.article_id))?;
        v.id = self.id();
        self.articles[idx].statut = "vendu".to_string();
        self.ventes.push(v.clone());
        Ok(v)
    }

    fn ventes(&mut self) -> Resultat<Vec<Vente>> {
        Ok(self.ventes.clone())
    }

    fn journaliser(&mut self, action: &str, detail: &str) -> Resultat<()> {
        self.audit_log
            .push((aujourdhui(), action.to_string(), detail.to_string()));
        Ok(())
    }

    fn audit(&mut self, limite: usize) -> Resultat<Vec<(String, String, String)>> {
        let mut v = self.audit_log.clone();
        v.reverse();
        v.truncate(limite);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_test(s: &mut Memoire) -> Client {
        s.creer_client(Client {
            id: 0,
            nom: "Atelier Dupont".into(),
            type_client: TypeClient::Professionnel,
            email: "contact@example.fr".into(),
            telephone: String::new(),
            adresse: String::new(),
            siren: String::new(),
            tva_intracom: String::new(),
            notes: String::new(),
        })
        .unwrap()
    }

    #[test]
    fn parcours_facture_complet() {
        let mut s = Memoire::new();
        let c = client_test(&mut s);
        let doc = s
            .creer_document(
                TypeDocument::Facture,
                c.id,
                "2026-10-30",
                "2026-11-29",
                "services",
                vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)],
            )
            .unwrap();
        assert_eq!(doc.numero, "FA-2026-0001");
        s.changer_statut(&doc.numero, StatutDocument::Valide).unwrap();
        let doc = s
            .ajouter_paiement(&doc.numero, 50_000, "2026-11-02", "virement")
            .unwrap();
        assert_eq!(doc.statut, StatutDocument::PartiellementPaye);
        let doc = s
            .ajouter_paiement(&doc.numero, 50_000, "2026-11-05", "virement")
            .unwrap();
        assert_eq!(doc.statut, StatutDocument::Paye);
        let (ventes, services) = s.ca_annuel(2026).unwrap();
        assert_eq!(services, 100_000);
        assert_eq!(ventes, 0);
    }

    #[test]
    fn recherche_client_floue() {
        let mut s = Memoire::new();
        client_test(&mut s);
        assert_eq!(s.chercher_clients("dupont").unwrap().len(), 1);
        assert_eq!(s.chercher_clients("DUPONT").unwrap().len(), 1);
        assert_eq!(s.chercher_clients("xyz").unwrap().len(), 0);
    }

    #[test]
    fn article_sans_composition_refuse() {
        let mut s = Memoire::new();
        let r = s.creer_article(Article {
            id: 0,
            marque: "X".into(),
            description: "T-shirt".into(),
            taille: "L".into(),
            composition: String::new(),
            etat: "bon".into(),
            prix_achat: 500,
            date_achat: "2026-10-01".into(),
            statut: String::new(),
        });
        assert!(r.is_err());
    }
}
