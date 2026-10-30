//! Implémentation PostgreSQL du trait `Store` (production, CDC §5.1).
//! Dates stockées en texte AAAA-MM-JJ, argent en centimes (bigint) :
//! aucun type exotique, migrations SQL embarquées et idempotentes.

use crate::domain::*;
use crate::store::*;
use postgres::{Client, NoTls};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS societe (
  id SERIAL PRIMARY KEY, nom TEXT NOT NULL, regime_tva TEXT NOT NULL DEFAULT 'franchise',
  devise TEXT NOT NULL DEFAULT 'EUR', siren TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS client (
  id SERIAL PRIMARY KEY, societe_id INT NOT NULL DEFAULT 1, nom TEXT NOT NULL,
  type_client TEXT NOT NULL DEFAULT 'professionnel', email TEXT NOT NULL DEFAULT '',
  telephone TEXT NOT NULL DEFAULT '', adresse TEXT NOT NULL DEFAULT '',
  siren TEXT NOT NULL DEFAULT '', tva_intracom TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS document (
  id SERIAL PRIMARY KEY, societe_id INT NOT NULL DEFAULT 1, client_id INT NOT NULL REFERENCES client(id),
  type_doc TEXT NOT NULL, numero TEXT NOT NULL UNIQUE, statut TEXT NOT NULL DEFAULT 'brouillon',
  date_doc TEXT NOT NULL, echeance TEXT NOT NULL DEFAULT '', activite TEXT NOT NULL DEFAULT 'services',
  paye_centimes BIGINT NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS ligne (
  id SERIAL PRIMARY KEY, document_id INT NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  designation TEXT NOT NULL, quantite_milli BIGINT NOT NULL, prix_centimes BIGINT NOT NULL, tva_bp INT NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS paiement (
  id SERIAL PRIMARY KEY, document_id INT NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  date_paiement TEXT NOT NULL, montant_centimes BIGINT NOT NULL, moyen TEXT NOT NULL DEFAULT 'virement');
CREATE TABLE IF NOT EXISTS article (
  id SERIAL PRIMARY KEY, marque TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
  taille TEXT NOT NULL DEFAULT '', composition TEXT NOT NULL, etat TEXT NOT NULL DEFAULT '',
  prix_achat_centimes BIGINT NOT NULL DEFAULT 0, date_achat TEXT NOT NULL DEFAULT '', statut TEXT NOT NULL DEFAULT 'en_stock');
CREATE TABLE IF NOT EXISTS vente (
  id SERIAL PRIMARY KEY, article_id INT NOT NULL REFERENCES article(id),
  date_vente TEXT NOT NULL, prix_vente_centimes BIGINT NOT NULL, frais_centimes BIGINT NOT NULL DEFAULT 0,
  plateforme TEXT NOT NULL DEFAULT 'Vinted');
CREATE TABLE IF NOT EXISTS audit (
  id SERIAL PRIMARY KEY, horodatage TEXT NOT NULL, action TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '');
";

pub struct PgStore {
    client: Client,
}

impl PgStore {
    pub fn connect(url: &str) -> Resultat<Self> {
        let mut client = Client::connect(url, NoTls).map_err(|e| format!("connexion PostgreSQL : {e}"))?;
        client
            .batch_execute(SCHEMA)
            .map_err(|e| format!("création du schéma : {e}"))?;
        Ok(Self { client })
    }

    fn ligne_from_row(&mut self, document_id: i64) -> Resultat<Vec<Ligne>> {
        let rows = self
            .client
            .query(
                "SELECT designation, quantite_milli, prix_centimes, tva_bp FROM ligne WHERE document_id = $1 ORDER BY id",
                &[&document_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| Ligne {
                designation: r.get("designation"),
                quantite_milli: r.get("quantite_milli"),
                prix_unitaire_ht: r.get("prix_centimes"),
                tva_bp: r.get("tva_bp"),
            })
            .collect())
    }

    fn doc_from(&mut self, id: i64, numero: &str, type_doc: &str, statut: &str, client_id: i64, date_doc: &str, echeance: &str, activite: &str, paye: i64) -> Resultat<Document> {
        Ok(Document {
            id,
            numero: numero.to_string(),
            type_doc: TypeDocument::from_str(type_doc)?,
            statut: statut_from_str(statut),
            client_id,
            date_doc: date_doc.to_string(),
            echeance: echeance.to_string(),
            activite: activite.to_string(),
            lignes: self.ligne_from_row(id)?,
            paye,
        })
    }
}

pub fn statut_from_str(s: &str) -> StatutDocument {
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

impl Store for PgStore {
    fn societe(&mut self) -> Resultat<Societe> {
        let rows = self
            .client
            .query("SELECT id, nom, regime_tva, devise, siren FROM societe ORDER BY id LIMIT 1", &[])
            .map_err(|e| e.to_string())?;
        if let Some(r) = rows.first() {
            let id: i32 = r.get("id");
            return Ok(Societe {
                id: i64::from(id),
                nom: r.get("nom"),
                regime: RegimeTva::from_str(&r.get::<_, String>("regime_tva"))?,
                devise: r.get("devise"),
                siren: r.get("siren"),
            });
        }
        let def = Societe::default();
        self.client
            .execute(
                "INSERT INTO societe (nom, regime_tva, devise, siren) VALUES ($1,$2,$3,$4)",
                &[&def.nom, &def.regime.as_str(), &def.devise, &def.siren],
            )
            .map_err(|e| e.to_string())?;
        Ok(def)
    }

    fn set_regime_tva(&mut self, regime: RegimeTva) -> Resultat<()> {
        let s = self.societe()?;
        self.client
            .execute(
                "UPDATE societe SET regime_tva = $1 WHERE id = $2",
                &[&regime.as_str(), &(s.id as i32)],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn creer_client(&mut self, c: Client) -> Resultat<Client> {
        if c.nom.trim().is_empty() {
            return Err("le nom du client est obligatoire".into());
        }
        let row = self
            .client
            .query_one(
                "INSERT INTO client (nom, type_client, email, telephone, adresse, siren, tva_intracom, notes)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
                &[&c.nom, &c.type_client.as_str(), &c.email, &c.telephone, &c.adresse, &c.siren, &c.tva_intracom, &c.notes],
            )
            .map_err(|e| e.to_string())?;
        let id: i32 = row.get("id");
        let mut c = c;
        c.id = i64::from(id);
        Ok(c)
    }

    fn clients(&mut self) -> Resultat<Vec<Client>> {
        let rows = self
            .client
            .query("SELECT id, nom, type_client, email, telephone, adresse, siren, tva_intracom, notes FROM client ORDER BY nom", &[])
            .map_err(|e| e.to_string())?;
        rows.iter()
            .map(|r| {
                let id: i32 = r.get("id");
                Ok(Client {
                    id: i64::from(id),
                    nom: r.get("nom"),
                    type_client: TypeClient::from_str(&r.get::<_, String>("type_client"))?,
                    email: r.get("email"),
                    telephone: r.get("telephone"),
                    adresse: r.get("adresse"),
                    siren: r.get("siren"),
                    tva_intracom: r.get("tva_intracom"),
                    notes: r.get("notes"),
                })
            })
            .collect()
    }

    fn client_par_id(&mut self, id: i64) -> Resultat<Client> {
        self.clients()?
            .into_iter()
            .find(|c| c.id == id)
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
        let row = self
            .client
            .query_one(
                "SELECT COUNT(*) AS n FROM document WHERE type_doc = $1 AND date_doc LIKE $2",
                &[&type_doc.as_str(), &format!("{annee}-%")],
            )
            .map_err(|e| e.to_string())?;
        let n: i64 = row.get("n");
        let numero = numero_document(type_doc, annee, (n + 1) as u32);
        let row = self
            .client
            .query_one(
                "INSERT INTO document (client_id, type_doc, numero, statut, date_doc, echeance, activite)
                 VALUES ($1,$2,$3,'brouillon',$4,$5,$6) RETURNING id",
                &[&(client_id as i32), &type_doc.as_str(), &numero, &date_doc, &echeance, &activite],
            )
            .map_err(|e| e.to_string())?;
        let doc_id: i32 = row.get("id");
        for l in &lignes {
            self.client
                .execute(
                    "INSERT INTO ligne (document_id, designation, quantite_milli, prix_centimes, tva_bp) VALUES ($1,$2,$3,$4,$5)",
                    &[&doc_id, &l.designation, &l.quantite_milli, &l.prix_unitaire_ht, &l.tva_bp],
                )
                .map_err(|e| e.to_string())?;
        }
        self.document_par_numero(&numero)
    }

    fn documents(&mut self, type_doc: Option<TypeDocument>) -> Resultat<Vec<Document>> {
        let rows = self
            .client
            .query(
                "SELECT id, numero, type_doc, statut, client_id, date_doc, echeance, activite, paye_centimes FROM document ORDER BY date_doc, numero",
                &[],
            )
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in &rows {
            let id: i32 = r.get("id");
            let client_id: i32 = r.get("client_id");
            let doc = self.doc_from(
                i64::from(id),
                &r.get::<_, String>("numero"),
                &r.get::<_, String>("type_doc"),
                &r.get::<_, String>("statut"),
                i64::from(client_id),
                &r.get::<_, String>("date_doc"),
                &r.get::<_, String>("echeance"),
                &r.get::<_, String>("activite"),
                r.get("paye_centimes"),
            )?;
            if type_doc.map(|t| doc.type_doc == t).unwrap_or(true) {
                out.push(doc);
            }
        }
        Ok(out)
    }

    fn document_par_numero(&mut self, numero: &str) -> Resultat<Document> {
        let row = self
            .client
            .query_one(
                "SELECT id, numero, type_doc, statut, client_id, date_doc, echeance, activite, paye_centimes FROM document WHERE numero = $1",
                &[&numero],
            )
            .map_err(|_| format!("document « {numero} » introuvable"))?;
        let id: i32 = row.get("id");
        let client_id: i32 = row.get("client_id");
        self.doc_from(
            i64::from(id),
            &row.get::<_, String>("numero"),
            &row.get::<_, String>("type_doc"),
            &row.get::<_, String>("statut"),
            i64::from(client_id),
            &row.get::<_, String>("date_doc"),
            &row.get::<_, String>("echeance"),
            &row.get::<_, String>("activite"),
            row.get("paye_centimes"),
        )
    }

    fn changer_statut(&mut self, numero: &str, statut: StatutDocument) -> Resultat<Document> {
        self.client
            .execute(
                "UPDATE document SET statut = $1 WHERE numero = $2",
                &[&statut.as_str(), &numero],
            )
            .map_err(|e| e.to_string())?;
        self.document_par_numero(numero)
    }

    fn ajouter_paiement(&mut self, numero: &str, montant: Cents, date_paiement: &str, moyen: &str) -> Resultat<Document> {
        let regime = self.societe()?.regime;
        let doc = self.document_par_numero(numero)?;
        self.client
            .execute(
                "INSERT INTO paiement (document_id, date_paiement, montant_centimes, moyen) VALUES ($1,$2,$3,$4)",
                &[&(doc.id as i32), &date_paiement, &montant, &moyen],
            )
            .map_err(|e| e.to_string())?;
        let nouveau_paye = doc.paye + montant;
        let statut = statut_apres_paiement(doc.totaux(regime).ttc, nouveau_paye);
        self.client
            .execute(
                "UPDATE document SET paye_centimes = $1, statut = $2 WHERE id = $3",
                &[&nouveau_paye, &statut.as_str(), &(doc.id as i32)],
            )
            .map_err(|e| e.to_string())?;
        self.document_par_numero(numero)
    }

    fn paiements(&mut self, document_id: i64) -> Resultat<Vec<Paiement>> {
        let rows = self
            .client
            .query(
                "SELECT id, date_paiement, montant_centimes, moyen FROM paiement WHERE document_id = $1 ORDER BY date_paiement",
                &[&(document_id as i32)],
            )
            .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                let id: i32 = r.get("id");
                Paiement {
                    id: i64::from(id),
                    document_id,
                    date_paiement: r.get("date_paiement"),
                    montant: r.get("montant_centimes"),
                    moyen: r.get("moyen"),
                }
            })
            .collect())
    }

    fn creer_article(&mut self, a: Article) -> Resultat<Article> {
        if a.composition.trim().is_empty() {
            return Err("composition textile obligatoire (règlement UE 1007/2011)".into());
        }
        let row = self
            .client
            .query_one(
                "INSERT INTO article (marque, description, taille, composition, etat, prix_achat_centimes, date_achat, statut)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
                &[&a.marque, &a.description, &a.taille, &a.composition, &a.etat, &a.prix_achat, &a.date_achat, &a.statut],
            )
            .map_err(|e| e.to_string())?;
        let id: i32 = row.get("id");
        let mut a = a;
        a.id = i64::from(id);
        Ok(a)
    }

    fn articles(&mut self) -> Resultat<Vec<Article>> {
        let rows = self
            .client
            .query("SELECT id, marque, description, taille, composition, etat, prix_achat_centimes, date_achat, statut FROM article ORDER BY id", &[])
            .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                let id: i32 = r.get("id");
                Article {
                    id: i64::from(id),
                    marque: r.get("marque"),
                    description: r.get("description"),
                    taille: r.get("taille"),
                    composition: r.get("composition"),
                    etat: r.get("etat"),
                    prix_achat: r.get("prix_achat_centimes"),
                    date_achat: r.get("date_achat"),
                    statut: r.get("statut"),
                }
            })
            .collect())
    }

    fn article_par_id(&mut self, id: i64) -> Resultat<Article> {
        self.articles()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("article n°{id} introuvable"))
    }

    fn enregistrer_vente(&mut self, v: Vente) -> Resultat<Vente> {
        self.article_par_id(v.article_id)?;
        let row = self
            .client
            .query_one(
                "INSERT INTO vente (article_id, date_vente, prix_vente_centimes, frais_centimes, plateforme) VALUES ($1,$2,$3,$4,$5) RETURNING id",
                &[&(v.article_id as i32), &v.date_vente, &v.prix_vente, &v.frais, &v.plateforme],
            )
            .map_err(|e| e.to_string())?;
        self.client
            .execute("UPDATE article SET statut = 'vendu' WHERE id = $1", &[&(v.article_id as i32)])
            .map_err(|e| e.to_string())?;
        let id: i32 = row.get("id");
        let mut v = v;
        v.id = i64::from(id);
        Ok(v)
    }

    fn ventes(&mut self) -> Resultat<Vec<Vente>> {
        let rows = self
            .client
            .query("SELECT id, article_id, date_vente, prix_vente_centimes, frais_centimes, plateforme FROM vente ORDER BY date_vente", &[])
            .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                let id: i32 = r.get("id");
                let article_id: i32 = r.get("article_id");
                Vente {
                    id: i64::from(id),
                    article_id: i64::from(article_id),
                    date_vente: r.get("date_vente"),
                    prix_vente: r.get("prix_vente_centimes"),
                    frais: r.get("frais_centimes"),
                    plateforme: r.get("plateforme"),
                }
            })
            .collect())
    }

    fn journaliser(&mut self, action: &str, detail: &str) -> Resultat<()> {
        self.client
            .execute(
                "INSERT INTO audit (horodatage, action, detail) VALUES ($1,$2,$3)",
                &[&aujourdhui(), &action, &detail],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn audit(&mut self, limite: usize) -> Resultat<Vec<(String, String, String)>> {
        let rows = self
            .client
            .query(
                "SELECT horodatage, action, detail FROM audit ORDER BY id DESC LIMIT $1",
                &[&(limite as i64)],
            )
            .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| (r.get("horodatage"), r.get("action"), r.get("detail")))
            .collect())
    }
}
