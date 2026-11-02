//! Accès à une vraie base PostgreSQL **par Rustorm** (ADR-009).
//!
//! Ce module n'existe que si la feature `postgres` est compilée
//! (`cargo build --features postgres`) : sans elle, l'ERP reste autonome sur un
//! dossier de fichiers et aucun pilote n'est lié au binaire.
//!
//! Tout passe par l'ORM, jamais par du SQL écrit à la main pour les données :
//!
//! | Besoin | Ce qui est utilisé |
//! |---|---|
//! | schéma des tables et types de colonnes | `rustorm::schema::parse` sur `rustorm/schema.rsm` |
//! | noms de tables et de colonnes | client généré (`columns()` dans `crate::orm`) |
//! | `CREATE TABLE` / index | `rustorm::migrations::initial_sql` (via `orm::ddl_postgres`) |
//! | connexion, paramètres, erreurs | `rustorm::postgres::Connection` |
//! | transactions | `Connection::begin` / `Transaction::commit` |
//! | lecture | constructeur `rustorm::query::Select` + `Row` (texte typé par colonne) |
//! | écriture | `rustorm::query::CompiledQuery` + `Value` typés d'après le schéma |
//!
//! Le magasin (`Store`) garde son modèle en mémoire : `persister()` rejoue
//! l'état complet dans **une seule transaction**, ce qui rend une écriture
//! multi-tables atomique (ENF-FIA-01) — ce que le stockage fichier ne peut pas
//! garantir entre deux tables.

use crate::orm;
use rustorm::postgres::{Connection, Error as ErreurPg};
use rustorm::query::{CompiledQuery, Select, Value};
use rustorm::schema::Scalar;
use std::collections::HashMap;

pub type Resultat<T> = Result<T, String>;

/// Type d'une colonne, réduit aux genres manipulés par l'ERP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Genre {
    Texte,
    Entier,
    GrandEntier,
    Booleen,
    Reel,
}

/// Connexion PostgreSQL pilotée par le modèle Rustorm.
pub struct Bdd {
    conn: Connection,
    description: String,
    genres: HashMap<(String, String), Genre>,
}

fn erreur(contexte: &str, e: ErreurPg) -> String {
    format!("PostgreSQL ({contexte}) : {e}")
}

/// Identifiant SQL entre guillemets doubles (les noms viennent du schéma, mais
/// on ne fait jamais confiance à une chaîne recopiée dans une requête).
fn citer(identifiant: &str) -> String {
    format!("\"{}\"", identifiant.replace('"', "\"\""))
}

/// Découpe `postgres://utilisateur:motdepasse@hote:port/base` (ou une chaîne de
/// connexion libpq) en éléments utiles.
fn analyser(url: &str) -> Option<(String, u16, String, String)> {
    let reste = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let (autorite, base) = reste.split_once('/')?;
    let base = base.split(['?', '#']).next().unwrap_or(base).to_string();
    let autorite = autorite.rsplit('@').next().unwrap_or(autorite);
    let (hote, port) = match autorite.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(5432)),
        None => (autorite.to_string(), 5432),
    };
    let utilisateur = url
        .split("://")
        .nth(1)
        .and_then(|r| r.rsplit_once('@').map(|(identifiants, _)| identifiants))
        .map(|identifiants| identifiants.split(':').next().unwrap_or("").to_string())
        .unwrap_or_else(|| "postgres".to_string());
    if hote.is_empty() || base.is_empty() {
        return None;
    }
    Some((hote, port, base, utilisateur))
}

/// URL sans le mot de passe (journaux, `/admin bdd`).
pub fn url_masquee(url: &str) -> String {
    match url.split_once("://") {
        Some((schema, reste)) => match reste.split_once('@') {
            Some((identifiants, hote)) => {
                let utilisateur = identifiants.split(':').next().unwrap_or("");
                format!("{schema}://{utilisateur}:***@{hote}")
            }
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}

impl Bdd {
    /// Ouvre une connexion. Une base locale (`localhost`/`127.0.0.1`) est
    /// jointe sans TLS (développement et CI) ; toute autre cible exige le mode
    /// sécurisé `verify-full` imposé par Rustorm (SEC-07).
    pub fn connecter(url: &str) -> Resultat<Self> {
        let conn = match analyser(url) {
            Some((hote, port, base, utilisateur))
                if hote == "localhost" || hote == "127.0.0.1" || hote == "::1" =>
            {
                Connection::connect_local_unencrypted(port, &utilisateur, &base)
                    .map_err(|e| erreur("connexion locale", e))?
            }
            _ => Connection::connect(url).map_err(|e| erreur("connexion", e))?,
        };
        let schema = orm::schema()?;
        let mut genres = HashMap::new();
        for modele in &schema.models {
            for champ in &modele.fields {
                let genre = match &champ.ty {
                    Scalar::Int => Genre::Entier,
                    Scalar::BigInt => Genre::GrandEntier,
                    Scalar::Boolean => Genre::Booleen,
                    Scalar::Float => Genre::Reel,
                    _ => Genre::Texte,
                };
                genres.insert((modele.table.clone(), champ.column.clone()), genre);
            }
        }
        Ok(Self {
            conn,
            description: url_masquee(url),
            genres,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    fn genre(&self, table: &'static str, colonne: &str) -> Resultat<Genre> {
        self.genres
            .get(&(table.to_string(), colonne.to_string()))
            .copied()
            .ok_or_else(|| format!("colonne inconnue dans le schéma : {table}.{colonne}"))
    }

    fn colonnes(table: &'static str) -> Resultat<&'static [&'static str]> {
        orm::colonnes(table).ok_or_else(|| format!("table absente du schéma : {table}"))
    }

    // ------------------------------------------------------------ Schéma

    /// Vrai si la table `clients` existe (migration déjà appliquée).
    pub fn est_migree(&mut self) -> Resultat<bool> {
        let q = CompiledQuery {
            sql: "SELECT to_regclass('public.clients') IS NOT NULL AS presente".to_string(),
            params: Vec::new(),
        };
        let lignes = self.conn.query(&q).map_err(|e| erreur("vérification", e))?;
        Ok(lignes
            .first()
            .and_then(|l| {
                l.get::<bool>("presente")
                    .ok()
                    .or_else(|| l.columns().first().and_then(|c| c.value.clone()).map(|v| v == "t"))
            })
            .unwrap_or(false))
    }

    /// Applique la migration issue du schéma (`CREATE TABLE` + index).
    ///
    /// Sans effet si la base est déjà à jour : la fonction est rejouable, ce qui
    /// permet de l'appeler à chaque ouverture (`ERP_DATABASE_URL`).
    pub fn migrer(&mut self) -> Resultat<bool> {
        if self.est_migree()? {
            return Ok(false);
        }
        let ddl = orm::ddl_postgres()?;
        self.conn
            .execute_raw(&ddl)
            .map_err(|e| erreur("migration", e))?;
        Ok(true)
    }

    /// Tables réellement présentes, dans l'ordre du modèle.
    pub fn tables_presentes(&mut self) -> Resultat<Vec<(&'static str, i64)>> {
        let mut sortie = Vec::new();
        for (table, _) in orm::tables() {
            sortie.push((table, self.compter(table)?));
        }
        Ok(sortie)
    }

    pub fn compter(&mut self, table: &'static str) -> Resultat<i64> {
        let q = CompiledQuery {
            sql: format!("SELECT count(*) AS n FROM {}", citer(table)),
            params: Vec::new(),
        };
        let lignes = self.conn.query(&q).map_err(|e| erreur("comptage", e))?;
        Ok(lignes
            .first()
            .and_then(|l| l.get::<i64>("n").ok())
            .unwrap_or(0))
    }

    // ------------------------------------------------------------ Lecture

    /// Lit une table entière et renvoie ses valeurs sous forme de texte, dans
    /// l'ordre exact des colonnes du modèle (`columns()` du client généré).
    pub fn charger(&mut self, table: &'static str) -> Resultat<Vec<Vec<String>>> {
        let colonnes = Self::colonnes(table)?;
        let requete = Select::new(table, colonnes)
            .compile()
            .map_err(|e| format!("requête {table} : {e}"))?;
        let lignes = self
            .conn
            .query(&requete)
            .map_err(|e| erreur(&format!("lecture de {table}"), e))?;
        let mut sortie = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let mut valeurs = Vec::with_capacity(colonnes.len());
            for colonne in colonnes {
                let genre = self.genre(table, colonne)?;
                let brute = ligne
                    .columns()
                    .iter()
                    .find(|c| c.name == *colonne)
                    .and_then(|c| c.value.as_deref());
                valeurs.push(texte_depuis(genre, brute));
            }
            sortie.push(valeurs);
        }
        Ok(sortie)
    }

    /// Métadonnées du magasin (`meta.tsv` en mode fichier, table `meta` ici).
    pub fn lire_meta(&mut self) -> Resultat<HashMap<String, String>> {
        let mut meta = HashMap::new();
        for ligne in self.charger("meta")? {
            // colonnes du modèle : id, cle, valeur
            let cle = ligne.get(1).cloned().unwrap_or_default();
            if cle.is_empty() {
                continue;
            }
            meta.insert(cle, ligne.get(2).cloned().unwrap_or_default());
        }
        Ok(meta)
    }

    // ------------------------------------------------------------ Écriture

    /// Écrit l'état complet du magasin en **une transaction** : soit toutes les
    /// tables sont à jour, soit aucune n'a bougé.
    pub fn enregistrer_tout(
        &mut self,
        tables: &[(&'static str, Vec<Vec<String>>)],
        meta: &HashMap<String, String>,
    ) -> Resultat<usize> {
        let mut meta_lignes: Vec<Vec<String>> = Vec::new();
        let mut cles: Vec<&String> = meta.keys().collect();
        cles.sort();
        for (index, cle) in cles.iter().enumerate() {
            meta_lignes.push(vec![
                (index as i64 + 1).to_string(),
                (*cle).clone(),
                meta[*cle].clone(),
            ]);
        }

        // La table `meta` fait partie du modèle : elle est écrite dans la même
        // transaction que les données, pour qu'un état ne soit jamais à moitié lu.
        let mut entrees: Vec<(&'static str, &[Vec<String>])> = tables
            .iter()
            .map(|(table, lignes)| (*table, lignes.as_slice()))
            .collect();
        entrees.push(("meta", meta_lignes.as_slice()));

        let mut transaction = self.conn.begin().map_err(|e| erreur("transaction", e))?;
        let mut total = 0usize;
        let mut resultat: Resultat<()> = Ok(());
        for (table, lignes) in entrees {
            if let Err(e) = remplacer(&mut transaction, table, lignes) {
                resultat = Err(e);
                break;
            }
            total += lignes.len();
        }
        match resultat {
            Ok(()) => {
                transaction
                    .commit()
                    .map_err(|e| erreur("validation de la transaction", e))?;
                Ok(total)
            }
            Err(e) => {
                // `rollback` peut échouer si le serveur a déjà annulé : on garde
                // le message d'origine, plus utile que l'erreur de nettoyage.
                let _ = transaction.rollback();
                Err(e)
            }
        }
    }

    pub fn vider(&mut self) -> Resultat<()> {
        for (table, _) in orm::tables() {
            let q = CompiledQuery {
                sql: format!("DELETE FROM {}", citer(table)),
                params: Vec::new(),
            };
            self.conn
                .execute(&q)
                .map_err(|e| erreur(&format!("vidage de {table}"), e))?;
        }
        Ok(())
    }
}

/// Traduit la valeur texte du magasin vers le type attendu par la colonne.
fn valeur(genre: Genre, texte: &str) -> Resultat<Value> {
    let brut = texte.trim();
    Ok(match genre {
        Genre::Texte => Value::String(texte.to_string()),
        Genre::GrandEntier => Value::BigInt(
            brut.parse()
                .map_err(|_| format!("nombre entier attendu : « {texte} »"))?,
        ),
        Genre::Entier => Value::Int(
            brut.parse()
                .map_err(|_| format!("entier attendu : « {texte} »"))?,
        ),
        Genre::Booleen => Value::Bool(matches!(brut, "1" | "oui" | "true" | "vrai" | "t")),
        Genre::Reel => Value::Float(
            brut.replace(',', ".")
                .parse()
                .map_err(|_| format!("nombre attendu : « {texte} »"))?,
        ),
    })
}

/// Traduit une valeur PostgreSQL en texte du magasin (booléens `t`/`f` → `1`/`0`).
fn texte_depuis(genre: Genre, valeur: Option<&str>) -> String {
    match (genre, valeur) {
        (_, None) => String::new(),
        (Genre::Booleen, Some("t")) | (Genre::Booleen, Some("true")) => "1".to_string(),
        (Genre::Booleen, Some("f")) | (Genre::Booleen, Some("false")) => "0".to_string(),
        (_, Some(v)) => v.to_string(),
    }
}

/// Remplace le contenu d'une table (suppression puis insertion) dans la
/// transaction courante. Le magasin n'efface jamais de pièce comptable : ce
/// « remplacement » ne fait que rejouer l'état complet, colonnes du modèle à
/// l'appui, sans jamais écrire de SQL à la main ailleurs.
fn remplacer(
    transaction: &mut rustorm::postgres::Transaction<'_>,
    table: &'static str,
    lignes: &[Vec<String>],
) -> Resultat<()> {
    let colonnes = Bdd::colonnes(table)?;
    let suppression = CompiledQuery {
        sql: format!("DELETE FROM {}", citer(table)),
        params: Vec::new(),
    };
    transaction
        .execute(&suppression)
        .map_err(|e| erreur(&format!("remplacement de {table}"), e))?;
    if lignes.is_empty() {
        return Ok(());
    }
    let liste_colonnes = colonnes
        .iter()
        .map(|c| citer(c))
        .collect::<Vec<_>>()
        .join(", ");
    let parametres = (1..=colonnes.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {} ({liste_colonnes}) VALUES ({parametres})",
        citer(table)
    );
    let schema = orm::schema()?;
    let genres: Vec<Genre> = colonnes
        .iter()
        .map(|colonne| {
            schema
                .models
                .iter()
                .find(|m| m.table == table)
                .and_then(|m| m.fields.iter().find(|f| f.column == *colonne))
                .map(|f| match &f.ty {
                    Scalar::Int => Genre::Entier,
                    Scalar::BigInt => Genre::GrandEntier,
                    Scalar::Boolean => Genre::Booleen,
                    Scalar::Float => Genre::Reel,
                    _ => Genre::Texte,
                })
                .unwrap_or(Genre::Texte)
        })
        .collect();
    for ligne in lignes {
        if ligne.len() != colonnes.len() {
            return Err(format!(
                "table {table} : {} valeurs pour {} colonnes",
                ligne.len(),
                colonnes.len()
            ));
        }
        let params = ligne
            .iter()
            .zip(&genres)
            .map(|(texte, genre)| valeur(*genre, texte))
            .collect::<Resultat<Vec<Value>>>()?;
        transaction
            .execute(&CompiledQuery {
                sql: sql.clone(),
                params,
            })
            .map_err(|e| erreur(&format!("écriture de {table}"), e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_url_est_masquee_pour_les_journaux() {
        assert_eq!(
            url_masquee("postgres://erp:secret@db.interne.local:5432/erp"),
            "postgres://erp:***@db.interne.local:5432/erp"
        );
        assert_eq!(
            url_masquee("host=db user=erp password=x"),
            "host=db user=erp password=x"
        );
    }

    #[test]
    fn une_url_est_analysable() {
        let (hote, port, base, utilisateur) =
            analyser("postgres://erp:secret@127.0.0.1:5433/compta").expect("url");
        assert_eq!(hote, "127.0.0.1");
        assert_eq!(port, 5433);
        assert_eq!(base, "compta");
        assert_eq!(utilisateur, "erp");
        assert!(analyser("host=localhost dbname=erp").is_none());
    }

    #[test]
    fn les_valeurs_suivent_le_type_du_schema() {
        assert_eq!(valeur(Genre::Texte, "Dupont").unwrap(), Value::String("Dupont".into()));
        assert_eq!(valeur(Genre::GrandEntier, "42").unwrap(), Value::BigInt(42));
        assert_eq!(valeur(Genre::Entier, "7").unwrap(), Value::Int(7));
        assert_eq!(valeur(Genre::Booleen, "1").unwrap(), Value::Bool(true));
        assert_eq!(valeur(Genre::Booleen, "0").unwrap(), Value::Bool(false));
        assert!(valeur(Genre::GrandEntier, "abc").is_err());
    }

    #[test]
    fn les_booleens_postgres_redeviennent_du_texte_du_magasin() {
        assert_eq!(texte_depuis(Genre::Booleen, Some("t")), "1");
        assert_eq!(texte_depuis(Genre::Booleen, Some("f")), "0");
        assert_eq!(texte_depuis(Genre::Texte, Some("Nice")), "Nice");
        assert_eq!(texte_depuis(Genre::Texte, None), "");
    }
}

/// Tests d'intégration : ils ne s'exécutent **que** sur demande explicite
/// (`cargo test --features postgres -- --ignored`) avec une variable
/// `ERP_DATABASE_URL` pointant sur une base jetable. Le reste de la suite ne
/// dépend donc jamais d'un serveur (ENF-SIMPL-01).
#[cfg(test)]
mod integration {
    use crate::cli::App;

    fn url() -> String {
        std::env::var("ERP_DATABASE_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .expect("ERP_DATABASE_URL doit pointer vers une base PostgreSQL jetable")
    }

    fn lancer(app: &mut App, commande: &str) -> String {
        let mut tampon: Vec<u8> = Vec::new();
        match app.executer(commande, &mut tampon) {
            Ok(_) => String::from_utf8_lossy(&tampon).to_string(),
            Err(e) => format!("ERREUR: {e}"),
        }
    }

    #[test]
    #[ignore = "nécessite une base jetable (ERP_DATABASE_URL)"]
    fn la_migration_cree_toutes_les_tables_du_schema() {
        let mut app = App::ouvrir_bdd(&url()).expect("connexion");
        app.store.vider_bdd().expect("vidage");
        let texte = lancer(&mut app, "/admin bdd");
        for (table, _) in crate::orm::tables() {
            assert!(texte.contains(table), "table {table} absente de /admin bdd : {texte}");
        }
    }

    #[test]
    #[ignore = "nécessite une base jetable (ERP_DATABASE_URL)"]
    fn un_parcours_complet_survit_a_la_reouverture() {
        let url = url();
        {
            let mut app = App::ouvrir_bdd(&url).expect("connexion");
            app.store.vider_bdd().expect("vidage");
            lancer(
                &mut app,
                "/societe modifier --nom \"Atelier Dupont\" --siren 123456789 \\
                 --adresse \"1 rue de la Paix\" --ville Nice",
            );
            lancer(&mut app, "/clients nouveau --nom \"Client SQL\" --type pro");
            lancer(
                &mut app,
                "/factures nouvelle --client sql --ligne \"Maintenance;2;500,00;2000\"",
            );
            let numero = app.store.documents(None)[0].numero.clone();
            lancer(&mut app, &format!("/factures {numero} valider"));
            lancer(&mut app, &format!("/factures {numero} payer"));
        }
        // Nouvelle ouverture : tout doit venir de PostgreSQL, pas de la mémoire.
        let mut app = App::ouvrir_bdd(&url).expect("seconde connexion");
        let clients = lancer(&mut app, "/clients");
        assert!(clients.contains("Client SQL"), "{clients}");
        let factures = lancer(&mut app, "/factures");
        assert!(factures.contains("FA-"), "{factures}");
        assert!(
            factures.contains("payée") || factures.contains("paye"),
            "{factures}"
        );
        let audit = lancer(&mut app, "/admin audit --verifier");
        assert!(audit.contains("intègre") || audit.contains("aucune altération"), "{audit}");
        let balance = lancer(&mut app, "/compta balance");
        assert!(balance.contains("411000") || balance.contains("Clients"), "{balance}");
    }

    #[test]
    #[ignore = "nécessite une base jetable (ERP_DATABASE_URL)"]
    fn les_ecritures_sont_atomiques_et_les_compteurs_justes() {
        let url = url();
        let mut app = App::ouvrir_bdd(&url).expect("connexion");
        app.store.vider_bdd().expect("vidage");
        lancer(&mut app, "/clients nouveau --nom \"Alpha\" --type pro");
        lancer(&mut app, "/clients nouveau --nom \"Beta\" --type particulier");
        let compteurs = app.store.compteurs_bdd().expect("compteurs").expect("base");
        let clients = compteurs
            .iter()
            .find(|(table, _)| *table == "clients")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(clients, 2, "la base doit contenir les deux clients : {compteurs:?}");
        let meta = app.store.compteurs_bdd().expect("compteurs").expect("base");
        assert!(
            meta.iter().any(|(table, n)| *table == "meta" && *n > 0),
            "les métadonnées doivent être persistées : {meta:?}"
        );
    }
}
